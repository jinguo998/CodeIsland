//! Glue between the `codeisland-ipc` server and the [`Runtime`].

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use codeisland_domain::{AgentEvent, Decision, UserAction};
use codeisland_integrations::ssh::RemoteHookHandler;
use codeisland_ipc::{ClientMessage, ServerMessage, PROTOCOL_VERSION};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::{Runtime, hook_policy};

/// Used when a blocking event does not carry an explicit `timeout_ms`.
const DEFAULT_BLOCKING_TIMEOUT: Duration = Duration::from_secs(86_400);

static RESPONSE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn response_id() -> String {
    format!(
        "codeisland-{}-{}",
        std::process::id(),
        RESPONSE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn acknowledge(id: String) -> ServerMessage {
    ServerMessage::Ack {
        v: PROTOCOL_VERSION,
        id: response_id(),
        ack_for: id,
    }
}

fn decision_message(id: String, event: &AgentEvent, decision: Decision) -> ServerMessage {
    let (stdout, exit_code) =
        codeisland_agents::render_response(event.agent, event, Some(&decision), "");
    ServerMessage::Decision {
        v: PROTOCOL_VERSION,
        id: response_id(),
        ack_for: id,
        action: decision.action,
        text: decision.text,
        stdout,
        exit_code,
    }
}

/// Spawns the IPC accept loop on `handle`.
pub(crate) fn spawn(runtime: Arc<Runtime>, handle: tokio::runtime::Handle) -> JoinHandle<()> {
    handle.spawn(async move {
        let handler_runtime = Arc::clone(&runtime);
        let handler_fn = move |message: ClientMessage| -> Pin<
            Box<dyn std::future::Future<Output = Option<ServerMessage>> + Send>,
        > {
            let runtime = Arc::clone(&handler_runtime);
            Box::pin(async move { handle_message(runtime, message).await })
        };
        let handler: codeisland_ipc::Handler = Arc::new(handler_fn);
        if let Err(error) = codeisland_ipc::serve(handler).await {
            tracing::error!(%error, "codeisland ipc server stopped");
        }
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoutedHook {
    source: String,
    event: String,
    payload: Value,
    waits: bool,
    defer_to_provider: bool,
    fields: codeisland_agents::normalize::HookFields,
}

impl RoutedHook {
    fn receive(payload: Value, host: Option<(&str, &str)>) -> Option<Self> {
        let host = host.map(|(id, name)| json!({"id": id, "name": name}));
        codeisland_swift::call(c"hook.receive", json!({"payload": payload, "host": host}))
    }

    fn into_event(self) -> (String, AgentEvent) {
        let mut event = codeisland_agents::normalize::project_received(&self.source, &self.event, self.payload, self.fields);
        if !self.waits && event.event_type.is_blocking() {
            // A provider can report a tool/question as activity without waiting for UI.
            event.event_type = codeisland_domain::AgentEventType::Message;
        }
        event.waits = self.waits;
        (self.source, event)
    }
}

/// The tunnel selects the host before Swift examines any provider-supplied fields.
pub fn remote_hook_handler(runtime: Arc<Runtime>, host_id: String) -> RemoteHookHandler {
    let held_requests = Arc::new(Semaphore::new(32));
    Arc::new(move |bytes| {
        let runtime = runtime.clone();
        let host_id = host_id.clone();
        let held_requests = held_requests.clone();
        Box::pin(async move {
            let settings = runtime.settings();
            let host = settings.remote_hosts.iter().find(|host| host.id == host_id)?;
            let hook = RoutedHook::receive(serde_json::from_slice(&bytes).ok()?, Some((&host.id, &host.name)))?;
            let defer = hook.defer_to_provider;
            let (source, mut event) = hook.into_event();
            // Keep persisted Windows card identifiers and local OS handles distinct.
            event.pid = None;
            event.wezterm_pane = None;
            event.transcript_path = None;
            let namespace = |id: &str| format!("ssh:{}:{}:{id}", host.id.len(), host.id);
            event.id = namespace(&event.id);
            event.session_id = namespace(&event.session_id);
            event.parent_session_id = event.parent_session_id.as_deref().map(namespace);
            let allowed: bool = codeisland_swift::call(c"hook.remoteFilter", json!({
                "filter": host.cwd_filter, "cwd": event.cwd,
                "roots": event.payload.as_ref().and_then(|row| row.get("workspace_roots")),
                "event": event.raw_event_name, "session": event.session_id,
                "tracked": runtime.state().store.sessions.iter().any(|session| session.session_id == event.session_id),
            }));
            if !allowed { return None; }
            let _permit = if event.waits { Some(held_requests.try_acquire_owned().ok()?) } else { None };
            match dispatch_event(runtime, event.id.clone(), source, event, None, defer).await {
                Some(ServerMessage::Decision { stdout, .. }) => Some(stdout),
                _ => None,
            }
        })
    })
}

/// Handles one parsed client message.
pub(crate) async fn handle_message(
    runtime: Arc<Runtime>,
    message: ClientMessage,
) -> Option<ServerMessage> {
    match message {
        ClientMessage::Hello { id, .. } | ClientMessage::Heartbeat { id, .. } => {
            Some(acknowledge(id))
        }
        ClientMessage::Hook { id, payload, timeout_ms, .. } => {
            let Some(hook) = RoutedHook::receive(payload, None) else { return Some(acknowledge(id)) };
            let defer = hook.defer_to_provider;
            let (source, event) = hook.into_event();
            dispatch_event(runtime, id, source, event, timeout_ms, defer).await
        }
        ClientMessage::Event { id, source, mut event, waits, timeout_ms, .. } => {
            // The compatibility message is accepted on the local pipe only.
            let local: bool = codeisland_swift::call(c"hook.localOrigin", json!({"payload": event.payload}));
            if !local { return Some(acknowledge(id)); }
            event.waits |= waits || event.event_type.is_blocking();
            dispatch_event(runtime, id, source, event, timeout_ms, false).await
        }
    }
}

async fn dispatch_event(
    runtime: Arc<Runtime>, id: String, source: String, event: AgentEvent,
    timeout_ms: Option<u64>, defer_to_provider: bool,
) -> Option<ServerMessage> {
    let settings = runtime.settings();
    let policy = hook_policy::evaluate(&settings, &source, &event);
    if policy == hook_policy::Policy::Exclude { return Some(acknowledge(id)); }
    codeisland_integrations::hook_webhook::forward(&settings, &source, &event);
    if policy == hook_policy::Policy::Approve {
        let decision = Decision { event_id: event.id.clone(), session_id: event.session_id.clone(),
            agent: event.agent, action: UserAction::Approve, text: None };
        return Some(decision_message(id, &event, decision));
    }
    if defer_to_provider { return Some(acknowledge(id)); }
    if !event.waits {
        runtime.handle_event(event);
        return Some(acknowledge(id));
    }
    let timeout = timeout_ms.map(Duration::from_millis).unwrap_or(DEFAULT_BLOCKING_TIMEOUT);
    match runtime.handle_blocking(event.clone(), timeout).await {
        Some(decision) => Some(decision_message(id, &event, decision)),
        None => Some(ServerMessage::Error { v: PROTOCOL_VERSION, id: response_id(), error: "decision timeout".to_owned() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEventType, AgentKind, AppSettings};

    fn runtime() -> Arc<Runtime> {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let dir = std::env::temp_dir()
                .join(format!("codeisland-ipc-tests-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp state dir");
            // SAFETY: set once, before any runtime is created in this test binary.
            unsafe {
                std::env::set_var("CODEISLAND_STATE_DIR", &dir);
            }
        });
        Runtime::new_for_test(AppSettings::default())
    }

    #[tokio::test]
    async fn remote_hook_uses_bound_host_and_excludes_unrelated_directories() {
        let runtime = Runtime::new_for_test(AppSettings {
            remote_hosts: vec![codeisland_domain::RemoteHostSetting {
                id: "dev".into(),
                name: "Build host".into(),
                host: "localhost".into(),
                port: None,
                user: "ci".into(),
                key_path: String::new(),
                auth_socket: String::new(),
                cwd_filter: "/work/mine".into(),
                auto_connect: false,
            }],
            ..AppSettings::default()
        });
        let handle = remote_hook_handler(runtime.clone(), "dev".into());
        let mut payload = json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "same-session",
            "_remote_host_id": "spoofed-host",
            "cwd": "/work/other/project",
            "pid": 321,
            "transcript_path": "/host-only/log.jsonl",
            "_edge": {"source": "claude", "waits": false, "host": {"id": "spoofed-host"}}
        });
        handle(serde_json::to_vec(&payload).unwrap()).await;
        assert!(runtime.state().store.sessions.is_empty());
        payload["workspace_roots"] = json!(["/work/mine/project"]);
        handle(serde_json::to_vec(&payload).unwrap()).await;
        let state = runtime.state();
        let session = &state.store.sessions[0];
        assert_eq!(session.session_id, "ssh:3:dev:same-session");
        assert!(session.pid.is_none());
        assert!(session.transcript_path.is_none());
    }

    #[tokio::test]
    #[ignore = "requires the temporary OpenSSH fixture and original Python hook"]
    async fn openssh_remote_python_hook_delivers_event_and_receives_decision() {
        let (mut manager, runtime) = openssh_fixture();
        let host_id = manager.host().id.clone();
        manager.connect().await.expect("public-key SSH authentication");
        let source = std::fs::read(std::env::var("CODEISLAND_SSH_TEST_SCRIPT").unwrap())
            .expect("original remote hook");
        let result = manager.deploy_remote_hook(
            &source,
            &std::env::var("CODEISLAND_SSH_TEST_DEPLOY_DIR").unwrap(),
            remote_hook_handler(runtime.clone(), host_id),
        ).await.expect("reverse UDS and SFTP");
        let call = format!(
            "printf '%s' '{{\"hook_event_name\":\"PermissionRequest\",\"session_id\":\"fixture\",\"cwd\":\"/remote/work/project\",\"tool_name\":\"Bash\",\"tool_input\":{{\"command\":\"pwd\"}}}}' | CODEISLAND_SOURCE=claude '{}'",
            result.remote_wrapper,
        );
        verify_remote_approval(&manager, &runtime, &call).await;
        manager.disconnect().await;
    }

    fn openssh_fixture() -> (codeisland_integrations::ssh::SshManager, Arc<Runtime>) {
        let host = codeisland_domain::RemoteHostSetting {
            id: std::env::var("CODEISLAND_SSH_TEST_ID").unwrap(),
            name: "OpenSSH fixture".into(),
            host: "127.0.0.1".into(),
            port: Some(std::env::var("CODEISLAND_SSH_TEST_PORT").unwrap().parse().unwrap()),
            user: std::env::var("CODEISLAND_SSH_TEST_USER").unwrap(),
            key_path: std::env::var("CODEISLAND_SSH_TEST_KEY").unwrap(),
            auth_socket: String::new(),
            cwd_filter: "/remote/work".into(),
            auto_connect: false,
        };
        let runtime = Runtime::new_for_test(AppSettings {
            remote_hosts: vec![host.clone()],
            ..AppSettings::default()
        });
        let manager = codeisland_integrations::ssh::SshManager::new(
            codeisland_integrations::ssh::RemoteHost {
                id: host.id.clone(),
                name: host.name.clone(),
                host: host.host.clone(),
                port: host.port,
                user: host.user.clone(),
                key_path: Some(host.key_path.clone()),
                auth_socket: None,
            },
        );
        (manager, runtime)
    }

    async fn verify_remote_approval(
        manager: &codeisland_integrations::ssh::SshManager,
        runtime: &Arc<Runtime>,
        command: &str,
    ) {
        let receive = async {
            let pending = loop {
                if let Some(request) = runtime.state().store.pending.first() {
                    break request.event_id.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            };
            assert!(runtime.resolve(&pending, UserAction::Approve, None).is_some());
        };
        let (response, ()) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(manager.exec(command), receive)
        }).await.expect("remote request and decision round trip");
        let reply: Value = serde_json::from_str(&response.expect("remote hook reply")).unwrap();
        assert_eq!(reply["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }

    #[tokio::test]
    #[ignore = "requires a disposable OpenSSH container home and the two macOS remote resources"]
    async fn openssh_remote_installer_configures_executes_and_removes_its_hooks() {
        use codeisland_integrations::ssh::{RemoteCustomCli, RemoteHookConfig};

        let (mut manager, runtime) = openssh_fixture();
        manager.connect().await.expect("public-key SSH authentication");
        manager.exec(r#"python3 - <<'PY'
from pathlib import Path
import json
assert Path('/.dockerenv').is_file(), 'requires disposable container home'
assert Path('/tmp/codeisland-installer-fixture').is_file(), 'missing installer fixture marker'
root = Path.home()
user_hook = {'matcher': 'user', 'hooks': [{'type': 'command', 'command': '/usr/local/bin/codeisland-audit'}]}
for directory in ('.claude', '.qoder', '.codebuddy'):
    path = root / directory / 'settings.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({'theme': 'user', 'hooks': {'SessionStart': [user_hook]}}))
for directory in ('.codex', '.hermes', '.trae', '.config/opencode', '.fixture-cli'):
    (root / directory).mkdir(parents=True, exist_ok=True)
(root / '.codex/config.toml').write_text('# user config\nmodel = "custom"\n')
(root / '.codex/hooks.json').write_text(json.dumps({'hooks': {'SessionStart': [user_hook]}}))
(root / '.hermes/config.yaml').write_text('theme: user\nhooks:\n  pre_tool_call:\n    - command: /usr/local/bin/user-hermes\n      timeout: 7\n')
(root / '.trae/traecli.yaml').write_text('theme: user\nhooks:\n  - type: command\n    command: /usr/local/bin/user-trae\n')
(root / '.config/opencode/opencode.jsonc').write_text('// user config\n{"plugin":["@user/codeisland-export"]}\n')
(root / '.fixture-cli/bad.json').write_text('malformed content')
PY"#).await.expect("seed all remote CLI configurations");
        let script = std::fs::read(std::env::var("CODEISLAND_SSH_TEST_SCRIPT").unwrap()).unwrap();
        let plugin = std::fs::read_to_string(std::env::var("CODEISLAND_SSH_TEST_OPENCODE_PLUGIN").unwrap()).unwrap();
        let dir = std::env::var("CODEISLAND_SSH_TEST_DEPLOY_DIR").unwrap();
        let deployed = manager.deploy_remote_hook(
            &script, &dir, remote_hook_handler(runtime.clone(), manager.host().id.clone()),
        ).await.expect("deploy remote bridge");
        let custom = || {
            let cli = |name: &str, source: &str, path: &str, format| RemoteCustomCli {
                name: name.into(),
                source: source.into(),
                config_path: path.into(),
                config_key: "events".into(),
                format,
                events: vec![("SessionStart".into(), 5)],
            };
            RemoteHookConfig {
                custom_clis: vec![
                    cli("Custom Claude", "fixture-claude", ".fixture-cli/claude.json", "claude"),
                    cli("Custom nested", "fixture-nested", ".fixture-cli/nested.json", "nested"),
                    cli("Bad config", "fixture-bad", ".fixture-cli/bad.json", "claude"),
                ],
                unsupported_custom_clis: vec!["Cursor-style fixture".into()],
            }
        };
        let first = manager.configure_remote_hooks(&deployed, &plugin, custom()).await.expect("configure hooks");
        for status in ["Claude ok", "Qoder ok", "Hermes ok", "Codex ok", "CodeBuddy ok",
            "Traecli ok", "OpenCode ok", "Custom Claude ok", "Custom nested ok",
            "Bad config config unreadable", "Cursor-style fixture skipped"] {
            assert!(first.contains(status), "missing {status:?} in {first}");
        }
        manager.exec(r#"python3 - <<'PY'
from pathlib import Path
import json
root = Path.home()
path = root / '.claude/settings.json'
data = json.loads(path.read_text())
data['hooks']['SessionStart'][-1]['hooks'].append({'type':'command','command':'/usr/local/bin/mixed-user'})
path.write_text(json.dumps(data))
PY"#).await.expect("insert a user hook in a shared matcher");
        manager.configure_remote_hooks(&deployed, &plugin, custom()).await.expect("reconfigure hooks");
        manager.exec(r#"python3 - <<'PY'
from pathlib import Path
import json, tomllib
root = Path.home()
data = json.loads((root / '.claude/settings.json').read_text())
hooks = [item['command'] for group in data['hooks']['SessionStart'] for item in group['hooks']]
assert len(hooks) == 3 and '/usr/local/bin/codeisland-audit' in hooks and '/usr/local/bin/mixed-user' in hooks
codex = json.loads((root / '.codex/hooks.json').read_text())
assert codex['hooks']['SessionEnd'][-1]['hooks'][0]['timeout'] == 3
assert tomllib.loads((root / '.codex/config.toml').read_text())['features']['hooks']
assert 'CODEISLAND_SOURCE=hermes' in (root / '.hermes/config.yaml').read_text()
assert 'CODEISLAND_SOURCE=traecli' in (root / '.trae/traecli.yaml').read_text()
assert '@user/codeisland-export' in json.loads((root / '.config/opencode/opencode.jsonc').read_text())['plugin']
assert 'matcher' in json.loads((root / '.fixture-cli/claude.json').read_text())['events']['SessionStart'][0]
assert 'matcher' not in json.loads((root / '.fixture-cli/nested.json').read_text())['events']['SessionStart'][0]
assert (root / '.fixture-cli/bad.json').read_text() == 'malformed content'
PY"#).await.expect("validate remote hook schemas and idempotence");
        let installed_hook = r#"python3 - <<'PY'
from pathlib import Path
import json, subprocess
settings = json.loads((Path.home() / '.claude/settings.json').read_text())
hook = settings['hooks']['PermissionRequest'][0]['hooks'][0]['command']
subprocess.run(hook, input=json.dumps({
    'hook_event_name':'PermissionRequest', 'session_id':'installed-hook',
    'cwd':'/remote/work/project', 'tool_name':'Bash', 'tool_input':{'command':'pwd'}
}).encode(), shell=True, check=True)
PY"#;
        verify_remote_approval(&manager, &runtime, installed_hook).await;
        let tcp_port = std::env::var("CODEISLAND_SSH_TEST_HTTP_PORT").unwrap().parse().unwrap();
        let bound = manager.forward_local(0, "127.0.0.1", tcp_port).await.unwrap();
        verify_forwarded_http(bound).await;
        // Terminate only the authenticated session process in this fixture.
        let _ = manager.exec(r#"if [ -f /tmp/codeisland-installer-fixture ] && [ "$PPID" -gt 1 ]; then kill -TERM "$PPID"; fi"#).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.is_connected() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("fixture SSH connection ended");
        manager.connect().await.expect("reconnect after transport drop");
        manager.forward_remote_hook(remote_hook_handler(runtime.clone(), manager.host().id.clone()))
            .await.expect("restart reverse hook tunnel");
        verify_forwarded_http(bound).await;
        verify_remote_approval(&manager, &runtime, installed_hook).await;
        assert_eq!(manager.uninstall_remote_hooks(&dir).await.unwrap(), "Managed hooks removed");
        manager.exec(r#"python3 - <<'PY'
from pathlib import Path
import json
root = Path.home()
data = json.loads((root / '.claude/settings.json').read_text())
assert data['theme'] == 'user'
hooks = [item['command'] for group in data['hooks']['SessionStart'] for item in group['hooks']]
assert hooks == ['/usr/local/bin/codeisland-audit', '/usr/local/bin/mixed-user']
assert 'PermissionRequest' not in data['hooks']
for path in ('.qoder/settings.json', '.codebuddy/settings.json', '.codex/hooks.json'):
    assert 'CODEISLAND_SOURCE' not in (root / path).read_text()
for path, command in [('.hermes/config.yaml', 'user-hermes'), ('.trae/traecli.yaml', 'user-trae')]:
    content = (root / path).read_text()
    assert command in content and 'CODEISLAND_SOURCE' not in content
assert json.loads((root / '.config/opencode/opencode.jsonc').read_text())['plugin'] == ['@user/codeisland-export']
assert json.loads((root / '.fixture-cli/claude.json').read_text()) == {}
assert json.loads((root / '.fixture-cli/nested.json').read_text()) == {}
PY"#).await.expect("uninstall preserves the user's hooks");
        assert_eq!(manager.uninstall_remote_hooks(&dir).await.unwrap(), "No managed hooks installed");
        manager.disconnect().await;
    }

    async fn verify_forwarded_http(port: u16) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut connection = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        connection.write_all(b"HEAD / HTTP/1.0\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(5), connection.read_to_string(&mut response))
            .await.expect("forwarded HTTP response").unwrap();
        assert!(response.starts_with("HTTP/1.0 200"), "{response}");
    }

    #[tokio::test]
    async fn hook_exclusion_precedes_automatic_approval() {
        let runtime = Runtime::new_for_test(AppSettings {
            auto_approve_tools: "TaskCreate".to_owned(),
            excluded_hook_cwd_substrings: ", .cache/agents, ".to_owned(),
            ..AppSettings::default()
        });
        let mut event = AgentEvent::new("ignored", AgentKind::Claude, AgentEventType::PermissionRequest);
        event.cwd = Some("/home/example/.cache/agents/runner".to_owned());
        event.tool = Some("TaskCreate".to_owned());
        let response = handle_message(
            Arc::clone(&runtime),
            ClientMessage::Event {
                v: PROTOCOL_VERSION,
                id: "excluded".to_owned(),
                source: "codeisland-claude".to_owned(),
                event,
                waits: true,
                timeout_ms: None,
            },
        )
        .await;
        assert!(matches!(response, Some(ServerMessage::Ack { .. })));
        assert!(runtime.state().store.sessions.is_empty());
    }

    #[tokio::test]
    async fn automatic_approval_matches_source_aliases_without_opening_a_card() {
        let runtime = Runtime::new_for_test(AppSettings {
            auto_approve_sources: "AGY".to_owned(),
            ..AppSettings::default()
        });
        let mut event = AgentEvent::new(
            "approved", AgentKind::Antigravity, AgentEventType::PermissionRequest,
        );
        event.tool = Some("shell".to_owned());
        let response = handle_message(
            Arc::clone(&runtime),
            ClientMessage::Event {
                v: PROTOCOL_VERSION,
                id: "approved".to_owned(),
                source: "codeisland-google-antigravity".to_owned(),
                event: event.clone(),
                waits: true,
                timeout_ms: None,
            },
        )
        .await;
        assert!(matches!(
            response,
            Some(ServerMessage::Decision { action: UserAction::Approve, .. })
        ));
        assert!(runtime.state().store.pending.is_empty());

        assert!(hook_policy::evaluate(&runtime.settings(), "antigravity-cli", &event) == hook_policy::Policy::Approve);
        assert!(hook_policy::evaluate(&runtime.settings(), "antigravity", &event) != hook_policy::Policy::Approve);
        event.tool = Some("AskUserQuestion".to_owned());
        assert!(hook_policy::evaluate(&runtime.settings(), "agy", &event) != hook_policy::Policy::Approve);
    }

    #[tokio::test]
    async fn hello_and_heartbeat_are_acked() {
        let runtime = runtime();
        let ack = handle_message(
            Arc::clone(&runtime),
            ClientMessage::Hello {
                v: PROTOCOL_VERSION,
                id: "hello-1".to_owned(),
                source: Some("test".to_owned()),
            },
        )
        .await
        .expect("response");
        match ack {
            ServerMessage::Ack { ack_for, .. } => assert_eq!(ack_for, "hello-1"),
            _ => panic!("unexpected response"),
        }

        let ack = handle_message(
            Arc::clone(&runtime),
            ClientMessage::Heartbeat {
                v: PROTOCOL_VERSION,
                id: "beat-1".to_owned(),
            },
        )
        .await
        .expect("response");
        match ack {
            ServerMessage::Ack { ack_for, .. } => assert_eq!(ack_for, "beat-1"),
            _ => panic!("unexpected response"),
        }
    }

    #[tokio::test]
    async fn non_blocking_event_is_acked_and_applied() {
        let runtime = runtime();
        let event = AgentEvent::new("ipc-session", AgentKind::Claude, AgentEventType::UserPrompt);
        let response = handle_message(
            Arc::clone(&runtime),
            ClientMessage::Event {
                v: PROTOCOL_VERSION,
                id: "event-1".to_owned(),
                source: "claude".to_owned(),
                event,
                waits: false,
                timeout_ms: None,
            },
        )
        .await
        .expect("response");

        match response {
            ServerMessage::Ack { ack_for, .. } => assert_eq!(ack_for, "event-1"),
            _ => panic!("unexpected response"),
        }
        assert!(runtime
            .state()
            .store
            .sessions
            .iter()
            .any(|session| session.session_id == "ipc-session"));
    }
}
