use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codeisland_domain::RemoteHostSetting;
use codeisland_integrations::ssh::{RemoteHost, SshManager};
use codeisland_runtime::{Runtime, remote_hook_handler};
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const STABLE_AFTER: Duration = Duration::from_secs(120);
const OFFLINE_RECHECK: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Default)]
struct RecoveryState {
    network: Option<bool>,
    generation: u64,
    settle: Duration,
}

#[derive(Clone)]
pub(crate) struct RecoverySignals(watch::Sender<RecoveryState>);

impl Default for RecoverySignals {
    fn default() -> Self {
        Self(watch::channel(RecoveryState::default()).0)
    }
}

impl RecoverySignals {
    pub(crate) fn wake(&self) {
        self.0.send_modify(|state| {
            state.generation = state.generation.wrapping_add(1);
            state.settle = Duration::from_secs(3);
        });
    }

    pub(crate) fn network(&self, online: Option<bool>) {
        self.0.send_if_modified(|state| {
            if state.network == online {
                return false;
            }
            if state.network == Some(false) && online == Some(true) {
                state.generation = state.generation.wrapping_add(1);
                state.settle = Duration::from_secs(2);
            }
            state.network = online;
            true
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Connecting,
    Connected,
    Retrying,
    Offline,
    Disconnected,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Retrying => "reconnecting",
            Self::Offline => "waiting_network",
            Self::Disconnected => "disconnected",
        }
    }
}

struct Connection {
    manager: Arc<AsyncMutex<SshManager>>,
    phase: Mutex<Phase>,
    cancel: watch::Sender<bool>,
    stopped: watch::Sender<bool>,
}

impl Connection {
    fn new(host: RemoteHost) -> Self {
        Self {
            manager: Arc::new(AsyncMutex::new(SshManager::new(host))),
            phase: Mutex::new(Phase::Connecting),
            cancel: watch::channel(false).0,
            stopped: watch::channel(false).0,
        }
    }

    fn phase(&self) -> Phase {
        *self.phase.lock().expect("SSH status lock")
    }

    fn set_phase(&self, phase: Phase) {
        *self.phase.lock().expect("SSH status lock") = phase;
    }
}

#[derive(Default)]
pub(crate) struct Connections {
    hosts: Mutex<HashMap<String, Arc<Connection>>>,
    recovery: RecoverySignals,
}

impl Connections {
    pub(crate) fn recovery_signals(&self) -> RecoverySignals {
        self.recovery.clone()
    }

    pub(crate) fn start_auto(&self, runtime: Arc<Runtime>) {
        for host in runtime.settings().remote_hosts {
            if host.auto_connect {
                self.start(host, runtime.clone());
            }
        }
    }

    pub(crate) async fn connect(
        &self,
        host: RemoteHostSetting,
        runtime: Arc<Runtime>,
    ) -> Result<String, String> {
        let host_id = host.id.clone();
        match self.start(host, runtime) {
            Some(result) => result.await
                .map_err(|_| "SSH connection stopped".to_owned())?
                .map(|()| "connected".to_owned()),
            None => Ok(self.status(&host_id).to_owned()),
        }
    }

    fn start(
        &self,
        host: RemoteHostSetting,
        runtime: Arc<Runtime>,
    ) -> Option<oneshot::Receiver<Result<(), String>>> {
        let mut hosts = self.hosts.lock().expect("SSH hosts lock");
        if hosts.get(&host.id).is_some_and(|connection| connection.phase() != Phase::Disconnected) {
            return None;
        }
        if let Some(old) = hosts.remove(&host.id) {
            old.cancel.send_replace(true);
        }
        let key_path = (!host.key_path.trim().is_empty()).then(|| host.key_path.trim().to_owned());
        let auth_socket = (!host.auth_socket.trim().is_empty()).then(|| host.auth_socket.trim().to_owned());
        let host_id = host.id.clone();
        let connection = Arc::new(Connection::new(RemoteHost {
            id: host.id,
            name: host.name,
            host: host.host.trim().to_owned(),
            port: host.port,
            user: host.user.trim().to_owned(),
            key_path,
            auth_socket,
        }));
        hosts.insert(host_id.clone(), connection.clone());
        let (send, receive) = oneshot::channel();
        tauri::async_runtime::spawn(maintain(
            connection, runtime, host_id, send, self.recovery.0.subscribe(),
        ));
        Some(receive)
    }

    pub(crate) async fn disconnect(&self, host_id: &str) {
        let connection = self.hosts.lock().expect("SSH hosts lock").get(host_id).cloned();
        let Some(connection) = connection else {
            return;
        };
        let mut stopped = connection.stopped.subscribe();
        connection.cancel.send_replace(true);
        let _ = stopped.wait_for(|stopped| *stopped).await;
        let mut hosts = self.hosts.lock().expect("SSH hosts lock");
        if hosts.get(host_id).is_some_and(|current| Arc::ptr_eq(current, &connection)) {
            hosts.remove(host_id);
        }
    }

    pub(crate) fn status(&self, host_id: &str) -> &'static str {
        self.hosts.lock().expect("SSH hosts lock").get(host_id)
            .map_or("not_connected", |connection| connection.phase().name())
    }

    pub(crate) fn manager(&self, host_id: &str) -> Result<Arc<AsyncMutex<SshManager>>, String> {
        self.hosts.lock().expect("SSH hosts lock").get(host_id)
            .filter(|connection| connection.phase() == Phase::Connected)
            .map(|connection| connection.manager.clone())
            .ok_or_else(|| "host is not connected".to_owned())
    }
}

async fn maintain(
    connection: Arc<Connection>,
    runtime: Arc<Runtime>,
    host_id: String,
    first_result: oneshot::Sender<Result<(), String>>,
    mut recovery: watch::Receiver<RecoveryState>,
) {
    let mut cancelled = connection.cancel.subscribe();
    let mut first_result = Some(first_result);
    let mut failures = 0usize;
    let mut generation = recovery.borrow().generation;
    loop {
        connection.set_phase(Phase::Connecting);
        let outcome = tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => break,
            outcome = connect_transport(&connection, &runtime, &host_id) => outcome,
        };
        if outcome.is_ok() {
            connection.set_phase(Phase::Connected);
        }
        if let Some(sender) = first_result.take() {
            let _ = sender.send(outcome.clone());
        }
        match outcome {
            Ok(()) => {
                let connected_at = Instant::now();
                tokio::select! {
                    _ = cancelled.wait_for(|cancelled| *cancelled) => break,
                    _ = wait_for_drop(&connection, &runtime, &host_id) => {}
                }
                if connected_at.elapsed() >= STABLE_AFTER {
                    failures = 0;
                }
                // A live tunnel survives wake or network recovery. Reset
                // backoff for a later failure, leaving the transport alone.
                let current = recovery.borrow_and_update().generation;
                if current != generation {
                    generation = current;
                    failures = 0;
                }
            }
            Err(error) => tracing::debug!(%host_id, %error, "SSH connection failed"),
        }
        let Some(auto_retry) = runtime.settings().remote_hosts.iter()
            .find(|host| host.id == host_id).map(|host| host.auto_connect)
        else {
            break;
        };
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => break,
            _ = wait_for_retry(&connection, &mut recovery, auto_retry, &mut generation, &mut failures) => {}
        }
    }
    connection.manager.lock().await.disconnect().await;
    connection.set_phase(Phase::Disconnected);
    connection.stopped.send_replace(true);
}

async fn wait_for_retry(
    connection: &Connection,
    recovery: &mut watch::Receiver<RecoveryState>,
    auto_retry: bool,
    generation: &mut u64,
    failures: &mut usize,
) {
    let mut backoff = None;
    loop {
        let state = *recovery.borrow_and_update();
        let reset = state.generation != *generation;
        if reset {
            *generation = state.generation;
            *failures = 0;
            backoff = Some(state.settle);
        }
        let delay = if state.network == Some(false) && (auto_retry || backoff.is_some()) {
            connection.set_phase(Phase::Offline);
            OFFLINE_RECHECK
        } else if auto_retry || backoff.is_some() {
            connection.set_phase(Phase::Retrying);
            *backoff.get_or_insert_with(|| {
                *failures += 1;
                retry_delay(*failures)
            })
        } else {
            connection.set_phase(Phase::Disconnected);
            if recovery.changed().await.is_err() {
                return;
            }
            continue;
        };
        tokio::select! {
            _ = tokio::time::sleep(delay) => return,
            change = recovery.changed() => if change.is_err() { return },
        }
    }
}

async fn connect_transport(connection: &Connection, runtime: &Arc<Runtime>, host_id: &str) -> Result<(), String> {
    let mut manager = connection.manager.lock().await;
    tokio::time::timeout(CONNECT_TIMEOUT, manager.connect()).await
        .map_err(|_| "SSH connection timed out".to_owned())?
        .map_err(|error| error.to_string())?;
    let handler = remote_hook_handler(runtime.clone(), host_id.to_owned());
    if let Err(error) = manager.forward_remote_hook(handler).await {
        tracing::debug!(%error, %host_id, "SSH hook tunnel unavailable");
    }
    Ok(())
}

async fn wait_for_drop(connection: &Connection, runtime: &Runtime, host_id: &str) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if !connection.manager.lock().await.is_connected()
            || !runtime.settings().remote_hosts.iter().any(|host| host.id == host_id)
        {
            return;
        }
    }
}

fn retry_delay(failures: usize) -> Duration {
    let schedule = [5, 15, 45, 120, 300];
    Duration::from_secs(if failures <= 10 {
        schedule[failures.saturating_sub(1).min(schedule.len() - 1)]
    } else {
        900
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_uses_original_fast_and_slow_schedule() {
        let actual: Vec<_> = (1..=12).map(|attempt| retry_delay(attempt).as_secs()).collect();
        assert_eq!(actual, [5, 15, 45, 120, 300, 300, 300, 300, 300, 300, 900, 900]);
    }

    #[test]
    fn only_recovery_and_wake_advance_the_reconnect_generation() {
        let signals = RecoverySignals::default();
        let status = signals.0.subscribe();
        signals.network(Some(true));
        assert_eq!(status.borrow().generation, 0);
        signals.network(Some(false));
        assert_eq!(status.borrow().generation, 0);
        signals.network(Some(true));
        assert_eq!(status.borrow().generation, 1);
        assert_eq!(status.borrow().settle, Duration::from_secs(2));
        signals.network(Some(true));
        assert_eq!(status.borrow().generation, 1);
        signals.wake();
        assert_eq!(status.borrow().generation, 2);
        assert_eq!(status.borrow().settle, Duration::from_secs(3));
    }
}
