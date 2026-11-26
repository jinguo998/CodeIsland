use std::sync::{Arc, Mutex};

use codeisland_runtime::Runtime;
use serde::Serialize;
use tokio::sync::watch;

#[derive(Clone, Default, Serialize)]
pub(crate) struct Status {
    powered_on: bool,
    advertising: bool,
    subscribed_clients: u32,
    error: Option<String>,
}

pub(crate) struct Publisher {
    status: Arc<Mutex<Status>>,
    restart: watch::Sender<u64>,
}

impl Default for Publisher {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(Status::default())),
            restart: watch::channel(0).0,
        }
    }
}

impl Publisher {
    pub(crate) fn start(&self, runtime: Arc<Runtime>) {
        let status = self.status.clone();
        #[cfg(windows)]
        winrt::start(runtime, self.restart.subscribe(), status);
        #[cfg(not(windows))]
        {
            let _ = runtime;
            status.lock().expect("companion status").error =
                Some("Apple Companion BLE requires Windows".into());
        }
    }

    pub(crate) fn status(&self) -> Status {
        self.status.lock().expect("companion status").clone()
    }

    pub(crate) fn restart(&self) {
        self.restart.send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

#[cfg(windows)]
mod winrt {
    use std::time::Duration;

    use codeisland_integrations::apple_companion::{CompanionState, NOTIFY_UUID, SERVICE_UUID};
    use codeisland_runtime::RuntimeUpdate;
    use tokio::sync::{Notify, broadcast};
    use tokio::time::Instant;
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristicProperties, GattCommunicationStatus, GattLocalCharacteristic,
        GattLocalCharacteristicParameters, GattServiceProvider,
        GattServiceProviderAdvertisementStatus, GattServiceProviderAdvertisingParameters,
        GattSubscribedClient,
    };
    use windows::Devices::Bluetooth::{BluetoothAdapter, BluetoothError};
    use windows::Devices::Radios::{Radio, RadioState};
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    use windows::core::GUID;

    use super::*;

    const RETRY_DELAY: Duration = Duration::from_secs(3);
    const RADIO_POLL: Duration = Duration::from_secs(1);

    pub(super) fn start(runtime: Arc<Runtime>, restart: watch::Receiver<u64>, status: Arc<Mutex<Status>>) {
        let worker_status = status.clone();
        let spawned = std::thread::Builder::new().name("apple-companion-ble".into()).spawn(move || {
            // WinRT activation and asynchronous continuations use this dedicated MTA.
            if let Err(error) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                worker_status.lock().expect("companion status").error = Some(error.to_string());
                return;
            }
            match tokio::runtime::Builder::new_current_thread().enable_time().build() {
                Ok(executor) => executor.block_on(publish_loop(runtime, restart, worker_status)),
                Err(error) => {
                    worker_status.lock().expect("companion status").error = Some(error.to_string());
                }
            }
            // SAFETY: this is the thread that initialized the apartment above.
            unsafe { RoUninitialize() };
        });
        if let Err(error) = spawned {
            status.lock().expect("companion status").error = Some(error.to_string());
        }
    }

    async fn publish_loop(runtime: Arc<Runtime>, mut restart: watch::Receiver<u64>, status: Arc<Mutex<Status>>) {
        let mut updates = runtime.subscribe();
        let changed = Arc::new(Notify::new());
        let mut peripheral = None::<Peripheral>;
        let mut retry_at = Instant::now();
        let mut sent_at = None::<Instant>;
        let mut dirty = true;
        let mut sequence = 0u64;
        let mut was_enabled = false;

        loop {
            let settings = runtime.settings();
            let enabled = settings.apple_companion_enabled;
            if enabled != was_enabled {
                dirty = true;
                retry_at = Instant::now();
                was_enabled = enabled;
            }
            if !enabled {
                peripheral = None;
                *status.lock().expect("companion status") = Status::default();
            } else if peripheral.is_none() && Instant::now() >= retry_at {
                let start = tokio::time::timeout(Duration::from_secs(5), Peripheral::open(changed.clone()))
                    .await.unwrap_or_else(|_| Err("Timed out opening the Bluetooth adapter".to_owned()));
                match start {
                    Ok(opened) => {
                        peripheral = Some(opened);
                        dirty = true;
                        sent_at = None;
                    }
                    Err(error) => {
                        *status.lock().expect("companion status") = Status { error: Some(error), ..Status::default() };
                        retry_at = Instant::now() + RETRY_DELAY;
                    }
                }
            }

            if let Some(service) = peripheral.as_ref() {
                let next = service.status();
                let failed = next.error.is_some();
                let subscribed = next.subscribed_clients > 0;
                *status.lock().expect("companion status") = next;
                if failed {
                    peripheral = None;
                    retry_at = Instant::now() + RETRY_DELAY;
                } else {
                    let heartbeat = Duration::from_secs(u64::from(settings.apple_companion_heartbeat_seconds.clamp(1, 30)));
                    if subscribed && (dirty || sent_at.is_none_or(|sent| sent.elapsed() >= heartbeat))
                        && runtime.settings().apple_companion_enabled
                    {
                        sequence = sequence.wrapping_add(1);
                        let snapshot = runtime.state();
                        let payload = CompanionState::from_snapshot(
                            &snapshot.store, &settings.default_mascot, sequence, chrono::Utc::now(),
                        );
                        if let Ok(Err(error)) = tokio::time::timeout(Duration::from_secs(4), service.send(&payload, &runtime)).await {
                            tracing::debug!(%error, "Apple Companion notification failed");
                        }
                        sent_at = Some(Instant::now());
                        dirty = false;
                    }
                }
            }

            tokio::select! {
                command = restart.changed() => {
                    if command.is_err() { break; }
                    peripheral = None;
                    retry_at = Instant::now();
                    dirty = true;
                }
                update = updates.recv() => match update {
                    Ok(RuntimeUpdate::State(_) | RuntimeUpdate::Settings(_))
                    | Err(broadcast::error::RecvError::Lagged(_)) => dirty = true,
                    Err(broadcast::error::RecvError::Closed) => break,
                    _ => {}
                },
                _ = changed.notified(), if enabled => dirty = true,
                _ = tokio::time::sleep(RADIO_POLL), if enabled => {}
            }
        }
    }

    struct Peripheral {
        provider: GattServiceProvider,
        characteristic: GattLocalCharacteristic,
        radio: Radio,
        subscription_token: i64,
        advertising_token: i64,
    }

    impl Peripheral {
        async fn open(changed: Arc<Notify>) -> Result<Self, String> {
            let adapter = BluetoothAdapter::GetDefaultAsync().map_err(display_error)?
                .await.map_err(display_error)?;
            if !adapter.IsPeripheralRoleSupported().map_err(display_error)? {
                return Err("This Bluetooth adapter does not support BLE peripheral mode".into());
            }
            let radio = adapter.GetRadioAsync().map_err(display_error)?.await.map_err(display_error)?;
            if radio.State().map_err(display_error)? != RadioState::On {
                return Err("Bluetooth is turned off".into());
            }
            let result = GattServiceProvider::CreateAsync(GUID::from_u128(SERVICE_UUID))
                .map_err(display_error)?.await.map_err(display_error)?;
            let error = result.Error().map_err(display_error)?;
            if error != BluetoothError::Success {
                return Err(format!("Could not create the Bluetooth service ({})", error.0));
            }
            let provider = result.ServiceProvider().map_err(display_error)?;
            let parameters = GattLocalCharacteristicParameters::new().map_err(display_error)?;
            parameters.SetCharacteristicProperties(GattCharacteristicProperties::Notify).map_err(display_error)?;
            let characteristic = provider.Service().map_err(display_error)?
                .CreateCharacteristicAsync(GUID::from_u128(NOTIFY_UUID), &parameters)
                .map_err(display_error)?.await.map_err(display_error)?;
            let error = characteristic.Error().map_err(display_error)?;
            if error != BluetoothError::Success {
                return Err(format!("Could not create the Bluetooth notification characteristic ({})", error.0));
            }
            let characteristic = characteristic.Characteristic().map_err(display_error)?;
            let subscription_changed = changed.clone();
            let subscription_token = characteristic.SubscribedClientsChanged(&TypedEventHandler::new(move |_, _| {
                subscription_changed.notify_one();
                Ok(())
            })).map_err(display_error)?;
            let advertising_token = provider.AdvertisementStatusChanged(&TypedEventHandler::new(move |_, _| {
                changed.notify_one();
                Ok(())
            })).map_err(display_error)?;
            let service = Self { provider, characteristic, radio, subscription_token, advertising_token };
            let advertising = GattServiceProviderAdvertisingParameters::new().map_err(display_error)?;
            advertising.SetIsConnectable(true).map_err(display_error)?;
            advertising.SetIsDiscoverable(true).map_err(display_error)?;
            service.provider.StartAdvertisingWithParameters(&advertising).map_err(display_error)?;
            Ok(service)
        }

        fn status(&self) -> Status {
            let powered_on = self.radio.State().is_ok_and(|state| state == RadioState::On);
            let advertisement = self.provider.AdvertisementStatus();
            let advertising = advertisement.as_ref().is_ok_and(|status| matches!(
                *status, GattServiceProviderAdvertisementStatus::Started
                    | GattServiceProviderAdvertisementStatus::StartedWithoutAllAdvertisementData
            ));
            let error = if !powered_on {
                Some("Bluetooth is turned off".to_owned())
            } else {
                match advertisement {
                    Ok(GattServiceProviderAdvertisementStatus::Aborted | GattServiceProviderAdvertisementStatus::Stopped) => {
                        Some("Bluetooth advertising stopped".to_owned())
                    }
                    Err(error) => Some(error.to_string()),
                    _ => None,
                }
            };
            let subscribed_clients = self.characteristic.SubscribedClients()
                .and_then(|clients| clients.Size()).unwrap_or(0);
            Status { powered_on, advertising, subscribed_clients, error }
        }

        async fn send(&self, state: &CompanionState, runtime: &Runtime) -> windows::core::Result<()> {
            let clients = self.characteristic.SubscribedClients()?;
            for index in 0..clients.Size()? {
                if let Ok(client) = clients.GetAt(index)
                    && let Err(error) = self.send_client(state, runtime, &client).await
                {
                    tracing::debug!(%error, "Apple Companion client notification failed");
                }
            }
            Ok(())
        }

        async fn send_client(
            &self, state: &CompanionState, runtime: &Runtime, client: &GattSubscribedClient,
        ) -> windows::core::Result<()> {
            for frame in state.bluetooth_frames(usize::from(client.MaxNotificationSize()?)) {
                if !runtime.settings().apple_companion_enabled {
                    break;
                }
                let writer = DataWriter::new()?;
                writer.WriteBytes(&frame)?;
                let result = self.characteristic.NotifyValueForSubscribedClientAsync(&writer.DetachBuffer()?, client)?.await?;
                if result.Status()? != GattCommunicationStatus::Success {
                    break;
                }
            }
            Ok(())
        }
    }

    impl Drop for Peripheral {
        fn drop(&mut self) {
            let _ = self.provider.RemoveAdvertisementStatusChanged(self.advertising_token);
            let _ = self.characteristic.RemoveSubscribedClientsChanged(self.subscription_token);
            let _ = self.provider.StopAdvertising();
        }
    }

    fn display_error(error: windows::core::Error) -> String {
        error.to_string()
    }
}
