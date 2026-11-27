use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

use codeisland_domain::RemoteHostSetting;
use codeisland_integrations::ssh::{ReconnectPolicy, ReconnectStep, RemoteHost, SshManager};
use codeisland_runtime::{Runtime, remote_hook_handler};
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Default)]
struct RecoveryState {
    network: Option<bool>,
    generation: u64,
    settle: Duration,
}

#[derive(Clone)]
pub(crate) struct RecoverySignals {
    events: watch::Sender<RecoveryState>,
    policy: Arc<ReconnectPolicy>,
}

impl Default for RecoverySignals {
    fn default() -> Self {
        Self { events: watch::channel(RecoveryState::default()).0, policy: Arc::default() }
    }
}

impl RecoverySignals {
    pub(crate) fn wake(&self) {
        self.events.send_modify(|state| {
            state.generation = state.generation.wrapping_add(1);
            state.settle = self.policy.wake();
        });
    }

    pub(crate) fn network(&self, online: Option<bool>) {
        self.events.send_if_modified(|state| {
            if state.network == online {
                return false;
            }
            if let Some(settle) = self.policy.network(online) {
                state.generation = state.generation.wrapping_add(1);
                state.settle = settle;
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
            connection, runtime, host_id, send, self.recovery.events.subscribe(),
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
    let policy = ReconnectPolicy::default();
    policy.start(&host_id);
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
                policy.connected(&host_id);
                tokio::select! {
                    _ = cancelled.wait_for(|cancelled| *cancelled) => break,
                    _ = wait_for_drop(&connection, &runtime, &host_id) => {}
                }
                let current = *recovery.borrow_and_update();
                policy.network(current.network);
                if current.generation != generation {
                    generation = current.generation;
                    // Recovery while the tunnel was up resets its backoff.
                    policy.recover(&host_id);
                }
            }
            Err(error) => tracing::debug!(%host_id, %error, "SSH connection failed"),
        }
        let Some(auto_retry) = runtime.settings().remote_hosts.iter()
            .find(|host| host.id == host_id).map(|host| host.auto_connect)
        else {
            break;
        };
        policy.network(recovery.borrow().network);
        let step = policy.failed(&host_id, auto_retry);
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => break,
            reconnect = wait_for_retry(&connection, &policy, &host_id, step, &mut recovery, &mut generation) => {
                if !reconnect { break; }
            }
        }
    }
    policy.stop(&host_id);
    connection.manager.lock().await.disconnect().await;
    connection.set_phase(Phase::Disconnected);
    connection.stopped.send_replace(true);
}

async fn wait_for_retry(
    connection: &Connection,
    policy: &ReconnectPolicy,
    host: &str,
    mut step: ReconnectStep,
    recovery: &mut watch::Receiver<RecoveryState>,
    generation: &mut u64,
) -> bool {
    loop {
        let (phase, delay) = match step {
            ReconnectStep::Connect => return true,
            ReconnectStep::None => (Phase::Disconnected, None),
            ReconnectStep::Retry { seconds } => (Phase::Retrying, Some(Duration::from_secs_f64(seconds))),
            ReconnectStep::Offline { seconds } => (Phase::Offline, Some(Duration::from_secs_f64(seconds))),
        };
        connection.set_phase(phase);
        let mut deadline = delay.map(|delay| Instant::now() + delay);
        let mut recovering = false;
        loop {
            let state = *recovery.borrow_and_update();
            policy.network(state.network);
            if state.generation != *generation {
                *generation = state.generation;
                policy.wake();
                recovering = true;
                deadline = Some(Instant::now() + state.settle);
                connection.set_phase(if state.network == Some(false) { Phase::Offline } else { Phase::Retrying });
            }
            tokio::select! {
                change = recovery.changed() => if change.is_err() { return false },
                _ = async {
                    match deadline {
                        Some(time) => tokio::time::sleep_until(time).await,
                        None => std::future::pending::<()>().await,
                    }
                } => break,
            }
        }
        step = if recovering { policy.recover(host) } else { policy.due(host) };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_uses_original_fast_and_slow_schedule() {
        let policy = ReconnectPolicy::default();
        assert_eq!(policy.start("test"), ReconnectStep::Connect);
        let actual: Vec<_> = (0..12).map(|_| {
            let ReconnectStep::Retry { seconds } = policy.failed("test", true) else { panic!("expected retry") };
            assert_eq!(policy.due("test"), ReconnectStep::Connect);
            Duration::from_secs_f64(seconds).as_secs()
        }).collect();
        assert_eq!(actual, [5, 15, 45, 120, 300, 300, 300, 300, 300, 300, 900, 900]);
    }

    #[test]
    fn only_recovery_and_wake_advance_the_reconnect_generation() {
        let signals = RecoverySignals::default();
        let status = signals.events.subscribe();
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
