//! Remote hosts over SSH.
//!
//! Only key and agent endpoint paths are stored. The
//! `ssh` feature enables [`SshManager`]; otherwise it is an `Unsupported`
//! stub so the rest of the app compiles without the dependency tree.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use codeisland_domain::AgentEvent;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssh")]
#[path = "ssh_config.rs"]
mod config;

/// The original Swift policy owns the host's reconnect state. The Windows
/// worker enacts each step with its SSH transport and Tokio timers.
#[derive(Default)]
pub struct ReconnectPolicy(codeisland_swift::Context);

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "step", rename_all = "lowercase")]
pub enum ReconnectStep {
    Connect,
    Retry { seconds: f64 },
    Offline { seconds: f64 },
    None,
}

impl ReconnectPolicy {
    pub fn start(&self, id: &str) -> ReconnectStep { self.0.call(c"ssh.start", id) }
    pub fn connected(&self, id: &str) { self.0.call(c"ssh.connected", id) }
    pub fn stop(&self, id: &str) { self.0.call(c"ssh.stop", id) }
    pub fn failed(&self, id: &str, auto: bool) -> ReconnectStep {
        self.0.call(c"ssh.failed", serde_json::json!({"id":id, "auto":auto}))
    }
    pub fn due(&self, id: &str) -> ReconnectStep { self.0.call(c"ssh.due", id) }
    pub fn recover(&self, id: &str) -> ReconnectStep { self.0.call(c"ssh.recover", id) }
    pub fn wake(&self) -> Duration {
        Duration::from_secs_f64(self.0.call(c"ssh.wake", ()))
    }
    pub fn network(&self, online: Option<bool>) -> Option<Duration> {
        self.0.call::<Option<f64>>(c"ssh.network", online).map(Duration::from_secs_f64)
    }
}

/// A user-configured remote host. Unset SSH options come from OpenSSH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteHost {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    /// Path to an SSH private key. `None` means "let the SSH agent decide".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_socket: Option<String>,
}

impl RemoteHost {
    pub fn new(id: impl Into<String>, name: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port: None,
            user: String::new(),
            key_path: None,
            auth_socket: None,
        }
    }

    pub fn target(&self) -> String {
        if self.user.trim().is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.user.trim(), self.host)
        }
    }
}

/// Result of uploading the hook binary and its wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployResult {
    pub remote_binary: String,
    pub remote_wrapper: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RemoteCustomCli {
    pub name: String,
    pub source: String,
    pub config_path: String,
    pub config_key: String,
    pub format: &'static str,
    pub events: Vec<(String, u64)>,
}

#[derive(Debug, Default, Serialize)]
pub struct RemoteHookConfig {
    pub custom_clis: Vec<RemoteCustomCli>,
    pub unsupported_custom_clis: Vec<String>,
}

/// One raw remote hook request, with its response in the agent's native format.
pub type RemoteHookHandler =
    Arc<dyn Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("SSH support is not compiled into this build")]
    Unsupported,
    #[error("not connected")]
    NotConnected,
    #[error("SSH error: {0}")]
    Ssh(String),
    #[error("SFTP error: {0}")]
    Sftp(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("remote command exited with status {status}: {stderr}")]
    Command { status: i32, stderr: String },
}

#[cfg(feature = "ssh")]
mod manager {
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::process::Stdio;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::task::{Context, Poll};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use russh::ChannelMsg;
    use russh::client::{AuthResult, Handle};
    use russh::keys::agent::{AgentIdentity, client::{AgentClient, AgentStream}};
    use russh_sftp::client::SftpSession;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Join, ReadBuf};
    use tokio::process::{Child, ChildStdin, ChildStdout};
    use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};

    use super::*;
    use super::config::{IdentityAgent, ResolvedHost};

    const MAX_REQUEST_BYTES: usize = 1024 * 1024;
    const MAX_BUFFERED_BYTES: usize = 8 * 1024 * 1024;
    const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
    const READ_DEADLINE: Duration = Duration::from_secs(300);
    const INSTALLER: &str = include_str!("../resources/codeisland-remote-install.py");
    const PREPARE_SOCKET: &str = r#"
import errno, os, socket, stat, sys
uid = os.getuid()
folder = "/tmp/codeisland-%d-%s" % (uid, sys.argv[1])
try:
    os.mkdir(folder, 0o700)
except FileExistsError:
    pass
info = os.lstat(folder)
if not stat.S_ISDIR(info.st_mode) or info.st_uid != uid or stat.S_IMODE(info.st_mode) != 0o700:
    sys.exit("CodeIsland socket directory must be owned by the SSH user with mode 0700")
path = os.path.join(folder, "island.sock")
if os.path.lexists(path):
    info = os.lstat(path)
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != uid:
        sys.exit("Existing CodeIsland socket is not owned by the SSH user")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
        probe.settimeout(2)
        result = probe.connect_ex(path)
    if result == errno.ECONNREFUSED:
        os.unlink(path)
    elif result != errno.ENOENT:
        sys.exit("Existing CodeIsland socket is already in use")
print(path)
"#;

    struct HookForward {
        path: String,
        handler: RemoteHookHandler,
        connections: Arc<Semaphore>,
        read_budget: Arc<Semaphore>,
        connected: watch::Sender<()>,
    }

    struct ClientHandler {
        host: String,
        port: u16,
        hook_forward: Arc<Mutex<Option<HookForward>>>,
    }

    impl Drop for ClientHandler {
        fn drop(&mut self) {
            if let Ok(mut forward) = self.hook_forward.lock() {
                forward.take();
            }
        }
    }

    impl russh::client::Handler for ClientHandler {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            server_public_key: &russh::keys::PublicKeyOrCertificate,
        ) -> Result<bool, Self::Error> {
            let key = server_public_key.public_key();
            match russh::keys::check_known_hosts(&self.host, self.port, &key) {
                // A key already on file must match.
                Ok(true) => Ok(true),
                // Trust on first use, like `ssh -o StrictHostKeyChecking=accept-new`:
                // record the key so a later change is rejected.
                Ok(false) => {
                    let _ = russh::keys::known_hosts::learn_known_hosts(
                        &self.host,
                        self.port,
                        &key,
                    );
                    Ok(true)
                }
                Err(_) => Ok(false),
            }
        }

        async fn server_channel_open_forwarded_streamlocal(
            &mut self,
            channel: russh::Channel<russh::client::Msg>,
            socket_path: &str,
            reply: russh::client::ChannelOpenHandle,
            _session: &mut russh::client::Session,
        ) -> Result<(), Self::Error> {
            let destination = self.hook_forward.lock().ok().and_then(|forward| {
                let forward = forward.as_ref().filter(|entry| entry.path == socket_path)?;
                let permit = forward.connections.clone().try_acquire_owned().ok()?;
                Some((
                    forward.handler.clone(),
                    permit,
                    forward.read_budget.clone(),
                    forward.connected.subscribe(),
                ))
            });
            let Some((handler, permit, read_budget, mut connected)) = destination else {
                return Ok(());
            };
            reply.accept().await;
            tokio::spawn(async move {
                let _permit = permit;
                let mut stream = channel.into_stream();
                tokio::select! {
                    _ = connected.changed() => {}
                    _ = async {
                        if let Ok(Some(bytes)) = tokio::time::timeout(
                            READ_DEADLINE, read_hook_request(&mut stream, read_budget),
                        ).await {
                            if let Some(response) = handler(bytes).await {
                                let _ = tokio::time::timeout(
                                    READ_IDLE_TIMEOUT, stream.write_all(response.as_bytes()),
                                ).await;
                            }
                        }
                        let _ = stream.shutdown().await;
                    } => {}
                }
            });
            Ok(())
        }
    }

    pub struct SshManager {
        host: RemoteHost,
        session: Option<Arc<russh::client::Handle<ClientHandler>>>,
        current_session: watch::Sender<Option<Arc<russh::client::Handle<ClientHandler>>>>,
        /// Tasks piping a local TCP listener through this session.
        forwarders: Vec<tokio::task::JoinHandle<()>>,
        hook_forward: Arc<Mutex<Option<HookForward>>>,
    }

    impl SshManager {
        pub fn new(host: RemoteHost) -> Self {
            Self {
                host,
                session: None,
                current_session: watch::channel(None).0,
                forwarders: Vec::new(),
                hook_forward: Arc::new(Mutex::new(None)),
            }
        }

        pub fn host(&self) -> &RemoteHost {
            &self.host
        }

        pub fn is_connected(&self) -> bool {
            self.session.as_ref().is_some_and(|session| !session.is_closed())
        }

        pub async fn connect(&mut self) -> Result<(), SshError> {
            if self.is_connected() {
                return Ok(());
            }
            self.current_session.send_replace(None);
            self.session.take();
            self.hook_forward = Arc::new(Mutex::new(None));
            let resolved = super::config::resolve(&self.host).await?;
            let config = Arc::new(russh::client::Config {
                keepalive_interval: Some(Duration::from_secs(15)),
                keepalive_max: 2,
                ..Default::default()
            });
            let address = (resolved.hostname.as_str(), resolved.port);
            let handler = ClientHandler {
                host: resolved.host_key_alias.clone().unwrap_or_else(|| resolved.hostname.clone()),
                port: if resolved.host_key_alias.is_some() { 22 } else { resolved.port },
                hook_forward: self.hook_forward.clone(),
            };
            let mut session = if let Some(jumps) = &resolved.proxy_jump {
                let proxy = ProxyStream::start(jumps, &resolved)?;
                russh::client::connect_stream(config, proxy, handler).await
            } else {
                russh::client::connect(config, address, handler).await
            }.map_err(|error| SshError::Ssh(error.to_string()))?;

            if !authenticate(&mut session, &resolved).await? {
                return Err(SshError::Ssh(format!(
                    "authentication failed for {}",
                    self.host.target()
                )));
            }
            let session = Arc::new(session);
            self.current_session.send_replace(Some(session.clone()));
            self.session = Some(session);
            Ok(())
        }

        /// Uploads the hook binary and a POSIX shell wrapper over SFTP, then
        /// makes both executable. The wrapper lets the agent run one stable
        /// path even when the binary is replaced by an update.
        pub async fn deploy_hook(
            &mut self,
            local_hook: &Path,
            remote_dir: &str,
        ) -> Result<DeployResult, SshError> {
            let bytes = std::fs::read(local_hook)?;
            self.upload_hook(&bytes, remote_dir, None).await
        }

        /// Open the host's private Unix socket and bind it to this SSH
        /// connection's hook handler.
        pub async fn forward_remote_hook(
            &mut self,
            handler: RemoteHookHandler,
        ) -> Result<String, SshError> {
            let session = self.session.as_ref().ok_or(SshError::NotConnected)?;
            if let Some(path) = self
                .hook_forward
                .lock()
                .ok()
                .and_then(|forward| forward.as_ref().map(|entry| entry.path.clone()))
            {
                return Ok(path);
            }
            if self.host.id.trim().is_empty() {
                return Err(SshError::Ssh("remote host id is empty".to_owned()));
            }
            let socket_key = self.host_key();
            let prepare = format!("python3 -c {} {}", shell_quote(PREPARE_SOCKET), socket_key);
            let path = tokio::time::timeout(Duration::from_secs(8), self.exec(&prepare))
                .await
                .map_err(|_| SshError::Ssh("preparing remote hook socket timed out".to_owned()))??
                .trim()
                .to_owned();
            let valid_uid = path
                .strip_prefix("/tmp/codeisland-")
                .and_then(|path| path.strip_suffix(&format!("-{socket_key}/island.sock")))
                .is_some_and(|uid| !uid.is_empty() && uid.bytes().all(|byte| byte.is_ascii_digit()));
            if !valid_uid {
                return Err(SshError::Ssh("invalid remote hook socket path".to_owned()));
            }
            *self.hook_forward.lock().expect("hook forward lock") = Some(HookForward {
                path: path.clone(),
                handler,
                connections: Arc::new(Semaphore::new(64)),
                read_budget: Arc::new(Semaphore::new(MAX_BUFFERED_BYTES)),
                connected: watch::channel(()).0,
            });
            match tokio::time::timeout(Duration::from_secs(8), session.streamlocal_forward(path.clone())).await {
                Ok(Ok(())) => return Ok(path),
                Ok(Err(error)) => {
                    self.hook_forward.lock().expect("hook forward lock").take();
                    Err(SshError::Ssh(error.to_string()))
                }
                Err(_) => {
                    self.hook_forward.lock().expect("hook forward lock").take();
                    let _ = tokio::time::timeout(Duration::from_secs(5), session.cancel_streamlocal_forward(path)).await;
                    Err(SshError::Ssh("binding remote hook socket timed out".to_owned()))
                }
            }
        }

        /// Deploy the original platform-independent hook and point its wrapper
        /// at this host's reverse tunnel.
        pub async fn deploy_remote_hook(
            &mut self,
            script: &[u8],
            remote_dir: &str,
            handler: RemoteHookHandler,
        ) -> Result<DeployResult, SshError> {
            let socket = self.forward_remote_hook(handler).await?;
            let result = self.upload_hook(script, remote_dir, Some(&socket)).await?;
            self.exec(&format!("{} --version", shell_quote(&result.remote_wrapper)))
                .await?;
            Ok(result)
        }

        pub async fn configure_remote_hooks(
            &self,
            deployed: &DeployResult,
            opencode_plugin: &str,
            custom: RemoteHookConfig,
        ) -> Result<String, SshError> {
            let socket_path = self.hook_forward.lock().ok()
                .and_then(|forward| forward.as_ref().map(|entry| entry.path.clone()))
                .ok_or(SshError::NotConnected)?;
            let (dir, _) = deployed.remote_binary.rsplit_once('/')
                .ok_or_else(|| SshError::Ssh("invalid remote hook path".to_owned()))?;
            let plugin = self.plugin_path(dir);
            let source = self.render_opencode_plugin(opencode_plugin, &deployed.remote_binary, &socket_path);
            self.stage_file(&plugin, source.as_bytes(), "644").await?;
            self.run_installer(dir, serde_json::json!({
                "operation": "install",
                "host_id": self.host.id,
                "host_name": self.host.name,
                "socket_path": socket_path,
                "hook_path": deployed.remote_binary,
                "plugin_path": plugin,
                "manifest_path": self.manifest_path(dir),
                "custom_clis": custom.custom_clis,
                "unsupported_custom_clis": custom.unsupported_custom_clis,
            })).await
        }

        pub async fn uninstall_remote_hooks(&self, remote_dir: &str) -> Result<String, SshError> {
            let dir = self.resolve_remote_dir(remote_dir).await?;
            let manifest = self.manifest_path(&dir);
            let exists = self.exec(&format!(
                "if test -f {}; then printf yes; fi",
                shell_quote(&manifest),
            )).await?;
            if exists.trim() != "yes" {
                return Ok("No managed hooks installed".to_owned());
            }
            self.run_installer(&dir, serde_json::json!({
                "operation": "uninstall",
                "host_id": self.host.id,
                "host_name": self.host.name,
                "hook_path": format!("{dir}/codeisland-hook"),
                "plugin_path": self.plugin_path(&dir),
                "manifest_path": manifest,
            })).await
        }

        fn host_key(&self) -> String {
            let mut hasher = DefaultHasher::new();
            self.host.id.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        }

        fn plugin_path(&self, dir: &str) -> String {
            format!("{dir}/codeisland-opencode-remote-{}.js", self.host_key())
        }

        fn manifest_path(&self, dir: &str) -> String {
            format!("{dir}/remote-hooks-{}.json", self.host_key())
        }

        fn render_opencode_plugin(&self, source: &str, hook_path: &str, socket_path: &str) -> String {
            codeisland_swift::call(c"install.remotePlugin", serde_json::json!({
                "source": source, "hook": hook_path, "socket": socket_path,
                "id": self.host.id, "name": self.host.name,
            }))
        }

        async fn run_installer(&self, dir: &str, request: serde_json::Value) -> Result<String, SshError> {
            let installer = format!("{dir}/.remote-installer-{}.py", self.host_key());
            let script: String = codeisland_swift::call(c"install.remoteScript", INSTALLER);
            self.stage_file(&installer, script.as_bytes(), "600").await?;
            let command = format!("python3 {} {}", shell_quote(&installer), shell_quote(&request.to_string()));
            let output = self.exec(&format!(r#""${{SHELL:-/bin/sh}}" -lc {}"#, shell_quote(&command))).await?;
            output.lines().rev().find_map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).ok()?;
                value.get("codeisland_summary")?.as_str().map(str::to_owned)
            }).ok_or_else(|| SshError::Ssh("remote hook installer did not report a result".to_owned()))
        }

        async fn stage_file(&self, path: &str, bytes: &[u8], mode: &str) -> Result<(), SshError> {
            static UPLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
            let number = UPLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temp = format!("{path}.{time:x}-{number:x}.upload");
            let sftp = self.open_sftp().await?;
            write_remote(&sftp, &temp, bytes).await?;
            let quoted = shell_quote(&temp);
            let result = self.exec(&format!("chmod {mode} {quoted} && mv -f {quoted} {}", shell_quote(path))).await;
            if result.is_err() {
                let _ = sftp.remove_file(&temp).await;
            }
            result.map(|_| ())
        }

        async fn resolve_remote_dir(&self, remote_dir: &str) -> Result<String, SshError> {
            let dir = remote_dir.trim();
            let resolved = if dir.starts_with('/') {
                dir.to_owned()
            } else {
                let home = self.exec("printf %s \"$HOME\"").await?;
                let home = home.lines().last().unwrap_or_default().trim();
                let rest = dir.strip_prefix("~/").or_else(|| (dir == "~").then_some("")).unwrap_or(dir);
                if rest.is_empty() { home.to_owned() } else { format!("{home}/{rest}") }
            };
            let resolved = resolved.trim_end_matches('/');
            if !resolved.starts_with('/') || resolved.contains(['\r', '\n', '\0']) {
                return Err(SshError::Ssh("invalid remote deployment directory".to_owned()));
            }
            Ok(resolved.to_owned())
        }

        async fn upload_hook(
            &self,
            bytes: &[u8],
            remote_dir: &str,
            socket: Option<&str>,
        ) -> Result<DeployResult, SshError> {
            let dir = self.resolve_remote_dir(remote_dir).await?;
            self.exec(&format!("mkdir -p {}", shell_quote(&dir))).await?;

            let remote_binary = format!("{dir}/codeisland-hook");
            self.stage_file(&remote_binary, bytes, "755").await?;

            let remote_wrapper = format!("{dir}/codeisland-hook.sh");
            let socket_export = socket.map_or_else(String::new, |path| {
                format!("export CODEISLAND_SOCKET_PATH={}\n", shell_quote(path))
            });
            let wrapper = format!(
                "#!/bin/sh\n{socket_export}exec {} \"$@\"\n",
                shell_quote(&remote_binary)
            );
            self.stage_file(&remote_wrapper, wrapper.as_bytes(), "755").await?;

            Ok(DeployResult {
                remote_binary,
                remote_wrapper,
                summary: None,
            })
        }

        async fn open_sftp(&self) -> Result<SftpSession, SshError> {
            let session = self.session.as_ref().ok_or(SshError::NotConnected)?;
            let channel = session.channel_open_session().await
                .map_err(|error| SshError::Ssh(error.to_string()))?;
            channel.request_subsystem(false, "sftp").await
                .map_err(|error| SshError::Sftp(error.to_string()))?;
            SftpSession::new(channel.into_stream()).await
                .map_err(|error| SshError::Sftp(error.to_string()))
        }

        /// Runs the remote hook and streams NDJSON `AgentEvent`s. The returned
        /// receiver closes when the remote process exits.
        pub async fn stream_events(
            &mut self,
            command: &str,
        ) -> Result<mpsc::Receiver<AgentEvent>, SshError> {
            let session = self.session.as_ref().ok_or(SshError::NotConnected)?;
            let mut channel = session
                .channel_open_session()
                .await
                .map_err(|error| SshError::Ssh(error.to_string()))?;
            channel
                .exec(true, command)
                .await
                .map_err(|error| SshError::Ssh(error.to_string()))?;

            let (sender, receiver) = mpsc::channel(64);
            tokio::spawn(async move {
                let mut buffer: Vec<u8> = Vec::new();
                while let Some(message) = channel.wait().await {
                    match message {
                        ChannelMsg::Data { data } => {
                            buffer.extend_from_slice(&data);
                            while let Some(position) =
                                buffer.iter().position(|byte| *byte == b'\n')
                            {
                                let line: Vec<u8> = buffer.drain(..=position).collect();
                                let text =
                                    String::from_utf8_lossy(&line[..line.len() - 1]);
                                let text = text.trim();
                                if text.is_empty() {
                                    continue;
                                }
                                if let Ok(event) = serde_json::from_str::<AgentEvent>(text) {
                                    if sender.send(event).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                        ChannelMsg::Eof | ChannelMsg::Close => break,
                        _ => {}
                    }
                }
            });
            Ok(receiver)
        }

        /// Runs a command and collects stdout/stderr/exit status.
        pub async fn exec(&self, command: &str) -> Result<String, SshError> {
            let session = self.session.as_ref().ok_or(SshError::NotConnected)?;
            let mut channel = session
                .channel_open_session()
                .await
                .map_err(|error| SshError::Ssh(error.to_string()))?;
            channel
                .exec(true, command)
                .await
                .map_err(|error| SshError::Ssh(error.to_string()))?;

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut status = -1;
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, .. } => {
                        stderr.extend_from_slice(&data)
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        status = exit_status as i32
                    }
                    _ => {}
                }
            }
            if status == 0 {
                Ok(String::from_utf8_lossy(&stdout).into_owned())
            } else {
                Err(SshError::Command {
                    status,
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                })
            }
        }

        /// Forwards a local TCP port to `remote_host:remote_port` through this
        /// session. Returns the bound local port (`0` picks an ephemeral one).
        pub async fn forward_local(
            &mut self,
            local_port: u16,
            remote_host: &str,
            remote_port: u16,
        ) -> Result<u16, SshError> {
            if !self.is_connected() {
                return Err(SshError::NotConnected);
            }
            let current_session = self.current_session.subscribe();
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", local_port)).await?;
            let bound = listener.local_addr()?.port();
            let remote_host = remote_host.to_owned();
            let task = tokio::spawn(async move {
                loop {
                    let Ok((stream, peer)) = listener.accept().await else {
                        break;
                    };
                    let session = current_session.borrow().clone();
                    let Some(session) = session.filter(|session| !session.is_closed()) else {
                        continue;
                    };
                    let remote_host = remote_host.clone();
                    tokio::spawn(async move {
                        let Ok(channel) = session
                            .channel_open_direct_tcpip(
                                remote_host,
                                u32::from(remote_port),
                                peer.ip().to_string(),
                                u32::from(peer.port()),
                            )
                            .await
                        else {
                            return;
                        };
                        let mut channel = channel.into_stream();
                        let mut stream = stream;
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut channel)
                            .await;
                    });
                }
            });
            self.forwarders.push(task);
            Ok(bound)
        }

        pub async fn disconnect(&mut self) {
            self.current_session.send_replace(None);
            for task in self.forwarders.drain(..) {
                task.abort();
            }
            let hook = self.hook_forward.lock().expect("hook forward lock").take();
            if let Some(session) = self.session.take() {
                if let Some(hook) = hook {
                    let path = hook.path.clone();
                    drop(hook);
                    let _ = tokio::time::timeout(
                        Duration::from_secs(5), session.cancel_streamlocal_forward(path),
                    ).await;
                }
                let _ = tokio::time::timeout(
                    Duration::from_secs(5), session.disconnect(russh::Disconnect::ByApplication, "", ""),
                ).await;
            }
        }
    }

    struct ProxyStream {
        io: Join<ChildStdout, ChildStdin>,
        // Dropping the SSH transport also terminates the proxy process.
        _process: Child,
    }

    impl ProxyStream {
        fn start(jumps: &str, target: &ResolvedHost) -> Result<Self, SshError> {
            let (intermediate, last) = jumps.rsplit_once(',').map_or((None, jumps), |(chain, last)| (Some(chain), last));
            if last.is_empty() || last.starts_with('-') || last.chars().any(char::is_whitespace) {
                return Err(SshError::Ssh("invalid ProxyJump host".to_owned()));
            }
            let mut command = super::config::ssh_command();
            command.args(["-T", "-o", "BatchMode=yes", "-o", "ClearAllForwardings=yes", "-W"])
                .arg(format!("[{}]:{}", target.hostname, target.port));
            if let Some(intermediate) = intermediate {
                command.arg("-J").arg(intermediate);
            }
            if let IdentityAgent::Socket(path) = &target.agent {
                command.env("SSH_AUTH_SOCK", path);
            }
            let target = if last.starts_with("ssh://") { last.to_owned() } else { format!("ssh://{last}") };
            let mut process = command.arg("--").arg(target)
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
            let stdin = process.stdin.take().ok_or_else(|| std::io::Error::other("SSH proxy stdin unavailable"))?;
            let stdout = process.stdout.take().ok_or_else(|| std::io::Error::other("SSH proxy stdout unavailable"))?;
            Ok(Self { io: tokio::io::join(stdout, stdin), _process: process })
        }
    }

    impl AsyncRead for ProxyStream {
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.io).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for ProxyStream {
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.io).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.io).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.io).poll_shutdown(cx)
        }
    }

    type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

    async fn connect_agent(agent: &IdentityAgent) -> Result<Option<Agent>, SshError> {
        let explicit = match agent {
            IdentityAgent::Disabled => return Ok(None),
            IdentityAgent::Socket(path) => Some(path.clone()),
            IdentityAgent::Default => None,
        };
        let environment = std::env::var_os("SSH_AUTH_SOCK")
            .filter(|value| !value.is_empty()).map(PathBuf::from);
        #[cfg(unix)]
        let result = match explicit.as_ref().or(environment.as_ref()) {
            Some(path) => AgentClient::connect_uds(path).await.map(|agent| agent.dynamic()),
            None => return Ok(None),
        };
        #[cfg(windows)]
        let result = {
            let path = explicit.as_ref().or(environment.as_ref())
                .map(PathBuf::as_path).unwrap_or_else(|| Path::new(r"\\.\pipe\openssh-ssh-agent"));
            match AgentClient::connect_named_pipe(path).await {
                Ok(agent) => Ok(agent.dynamic()),
                Err(_) if explicit.is_none() && environment.is_none() => {
                    AgentClient::connect_pageant().await.map(|agent| agent.dynamic())
                }
                Err(error) => Err(error),
            }
        };
        #[cfg(not(any(unix, windows)))]
        let result = return Ok(None);
        match result {
            Ok(agent) => Ok(Some(agent)),
            Err(error) if explicit.is_some() => Err(SshError::Ssh(format!("SSH agent: {error}"))),
            Err(_) => Ok(None),
        }
    }

    async fn authenticate(session: &mut Handle<ClientHandler>, host: &ResolvedHost) -> Result<bool, SshError> {
        let ssh_error = |error: russh::Error| SshError::Ssh(error.to_string());
        if matches!(session.authenticate_none(&host.user).await.map_err(ssh_error)?, AuthResult::Success) {
            return Ok(true);
        }
        let mut files = host.identity_files.iter();
        if host.explicit_key && let Some(path) = files.next()
            && authenticate_file(session, &host.user, path).await?
        {
            return Ok(true);
        }
        let mut agent_error = None;
        match connect_agent(&host.agent).await {
            Ok(Some(mut agent)) => {
                if authenticate_agent(session, host, &mut agent).await? {
                    return Ok(true);
                }
            }
            Ok(None) => {}
            Err(error) => agent_error = Some(error),
        }
        for path in files {
            if authenticate_file(session, &host.user, path).await? {
                return Ok(true);
            }
        }
        agent_error.map_or(Ok(false), Err)
    }

    async fn authenticate_file(session: &mut Handle<ClientHandler>, user: &str, path: &Path) -> Result<bool, SshError> {
        let Ok(key) = russh::keys::load_secret_key(path, None) else {
            return Ok(false);
        };
        let hash = rsa_hash(session, key.algorithm()).await;
        let key = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        let result = session.authenticate_publickey(user, key).await
            .map_err(|error| SshError::Ssh(error.to_string()))?;
        Ok(matches!(result, AuthResult::Success))
    }

    async fn authenticate_agent(
        session: &mut Handle<ClientHandler>,
        host: &ResolvedHost,
        agent: &mut Agent,
    ) -> Result<bool, SshError> {
        let Ok(mut identities) = agent.request_identities().await else {
            return Ok(false);
        };
        let configured: Vec<_> = host.identity_files.iter().filter_map(|path| {
            russh::keys::load_public_key(path)
                .or_else(|_| {
                    let mut public = path.as_os_str().to_os_string();
                    public.push(".pub");
                    russh::keys::load_public_key(public)
                }).ok()
        }).collect();
        let matches_file = |identity: &AgentIdentity| {
            configured.iter().any(|key| key.key_data() == identity.public_key().key_data())
        };
        if host.identities_only {
            identities.retain(matches_file);
        } else {
            identities.sort_by_key(|identity| !matches_file(identity));
        }
        for identity in identities {
            let hash = rsa_hash(session, identity.public_key().algorithm()).await;
            let result = match identity {
                AgentIdentity::PublicKey { key, .. } => session
                    .authenticate_publickey_with(&host.user, key, hash, agent).await,
                AgentIdentity::Certificate { certificate, .. } => session
                    .authenticate_certificate_with(&host.user, certificate, hash, agent).await,
            };
            if matches!(result, Ok(AuthResult::Success)) {
                return Ok(true);
            }
            if session.is_closed() {
                break;
            }
        }
        Ok(false)
    }

    async fn rsa_hash(session: &Handle<ClientHandler>, algorithm: russh::keys::Algorithm) -> Option<russh::keys::HashAlg> {
        if algorithm.is_rsa() {
            session.best_supported_rsa_hash().await.ok().flatten()
                .unwrap_or(Some(russh::keys::HashAlg::Sha512))
        } else {
            None
        }
    }

    async fn read_hook_request(
        stream: &mut (impl AsyncRead + Unpin),
        read_budget: Arc<Semaphore>,
    ) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut buffered: Option<OwnedSemaphorePermit> = None;
        let mut chunk = [0; 8192];
        loop {
            let size = tokio::time::timeout(READ_IDLE_TIMEOUT, stream.read(&mut chunk))
                .await.ok()?.ok()?;
            if size == 0 {
                return (!bytes.is_empty()).then_some(bytes);
            }
            if bytes.len() + size > MAX_REQUEST_BYTES {
                return None;
            }
            let permit = read_budget.clone().try_acquire_many_owned(size as u32).ok()?;
            match &mut buffered {
                Some(existing) => existing.merge(permit),
                None => buffered = Some(permit),
            }
            bytes.extend_from_slice(&chunk[..size]);
        }
    }

    /// Create a new temporary file without following existing paths.
    async fn write_remote(
        sftp: &SftpSession,
        path: &str,
        data: &[u8],
    ) -> Result<(), SshError> {
        use tokio::io::AsyncWriteExt;

        let flags = russh_sftp::protocol::OpenFlags::WRITE
            | russh_sftp::protocol::OpenFlags::CREATE
            | russh_sftp::protocol::OpenFlags::EXCLUDE;
        let mut file = sftp
            .open_with_flags(path, flags)
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        let result = async {
            file.write_all(data).await?;
            file.shutdown().await
        }.await;
        if result.is_err() {
            drop(file);
            let _ = sftp.remove_file(path).await;
        }
        result.map_err(|error| SshError::Sftp(error.to_string()))
    }

    fn shell_quote(value: &str) -> String {
        codeisland_swift::call(c"install.shellWord", value)
    }
}

#[cfg(not(feature = "ssh"))]
mod manager {
    use tokio::sync::mpsc;

    use super::*;

    /// Stub used when the `ssh` feature is off.
    pub struct SshManager {
        host: RemoteHost,
    }

    impl SshManager {
        pub fn new(host: RemoteHost) -> Self {
            Self { host }
        }

        pub fn host(&self) -> &RemoteHost {
            &self.host
        }

        pub fn is_connected(&self) -> bool {
            false
        }

        pub async fn connect(&mut self) -> Result<(), SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn deploy_hook(
            &mut self,
            _local_hook: &Path,
            _remote_dir: &str,
        ) -> Result<DeployResult, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn forward_remote_hook(
            &mut self,
            _handler: RemoteHookHandler,
        ) -> Result<String, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn deploy_remote_hook(
            &mut self,
            _script: &[u8],
            _remote_dir: &str,
            _handler: RemoteHookHandler,
        ) -> Result<DeployResult, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn configure_remote_hooks(
            &self,
            _deployed: &DeployResult,
            _opencode_plugin: &str,
            _custom: RemoteHookConfig,
        ) -> Result<String, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn uninstall_remote_hooks(&self, _remote_dir: &str) -> Result<String, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn stream_events(
            &mut self,
            _command: &str,
        ) -> Result<mpsc::Receiver<AgentEvent>, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn exec(&self, _command: &str) -> Result<String, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn forward_local(
            &mut self,
            _local_port: u16,
            _remote_host: &str,
            _remote_port: u16,
        ) -> Result<u16, SshError> {
            Err(SshError::Unsupported)
        }

        pub async fn disconnect(&mut self) {}
    }
}

pub use manager::SshManager;

/// Convenience for callers that only have a string key path.
pub fn host_with_key(
    id: impl Into<String>,
    name: impl Into<String>,
    host: impl Into<String>,
    user: impl Into<String>,
    key_path: impl Into<PathBuf>,
) -> RemoteHost {
    let mut remote = RemoteHost::new(id, name, host);
    remote.user = user.into();
    remote.key_path = Some(key_path.into().to_string_lossy().into_owned());
    remote
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_target_uses_user_when_present() {
        let mut host = RemoteHost::new("1", "dev", "example.com");
        assert_eq!(host.target(), "example.com");
        host.user = "alice".into();
        assert_eq!(host.target(), "alice@example.com");
    }

    #[test]
    fn host_with_key_records_only_the_path() {
        let host = host_with_key("1", "dev", "example.com", "alice", "/home/alice/.ssh/id_ed25519");
        assert_eq!(host.key_path.as_deref(), Some("/home/alice/.ssh/id_ed25519"));
        assert_eq!(host.port, None);
    }

    #[test]
    fn remote_host_preserves_an_unset_port_for_alias_resolution() {
        let host: RemoteHost = serde_json::from_str(
            r#"{"id":"1","name":"dev","host":"example.com","user":"alice"}"#,
        )
        .unwrap();
        assert_eq!(host.port, None);
        assert!(host.key_path.is_none());
    }

    #[cfg(feature = "ssh")]
    #[tokio::test]
    #[ignore = "requires an isolated OpenSSH config, SSH agent, and temporary sshd"]
    async fn openssh_alias_uses_only_the_configured_agent_key() {
        let mut alias = SshManager::new(RemoteHost::new("fixture", "Build fixture", "ci-agent"));
        alias.connect().await.expect("config alias and IdentityAgent");
        assert_eq!(alias.exec("printf agent-authenticated").await.unwrap(), "agent-authenticated");
        alias.disconnect().await;

        let mut host = RemoteHost::new("fixture", "Build fixture", "ci-agent-override");
        host.auth_socket = Some(std::env::var("CODEISLAND_TEST_SSH_AGENT").unwrap());
        let mut explicit = SshManager::new(host);
        explicit.connect().await.expect("host socket overrides IdentityAgent none");
        assert_eq!(explicit.exec("printf explicit-agent").await.unwrap(), "explicit-agent");
        explicit.disconnect().await;

        let mut jump = SshManager::new(RemoteHost::new("fixture", "Build fixture", "ci-via-jump"));
        jump.connect().await.expect("ProxyJump and agent auth");
        assert_eq!(jump.exec("printf jumped").await.unwrap(), "jumped");
        jump.disconnect().await;
    }
}
