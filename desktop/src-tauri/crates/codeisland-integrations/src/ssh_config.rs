use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{RemoteHost, SshError};

pub(super) enum IdentityAgent {
    Default,
    Disabled,
    Socket(PathBuf),
}

pub(super) struct ResolvedHost {
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub host_key_alias: Option<String>,
    pub proxy_jump: Option<String>,
    pub identity_files: Vec<PathBuf>,
    pub explicit_key: bool,
    pub identities_only: bool,
    pub agent: IdentityAgent,
}

pub(super) async fn resolve(host: &RemoteHost) -> Result<ResolvedHost, SshError> {
    validate(host)?;
    let mut command = ssh_command();
    command.kill_on_drop(true).args(["-G", "-T", "-o", "BatchMode=yes"]);
    if let Some(port) = host.port {
        command.arg("-p").arg(port.to_string());
    }
    if !host.user.trim().is_empty() {
        command.arg("-l").arg(host.user.trim());
    }
    if let Some(path) = host.key_path.as_deref().filter(|value| !value.trim().is_empty()) {
        command.arg("-i").arg(path);
    }
    command.arg("--").arg(host.host.trim());
    let output = tokio::time::timeout(Duration::from_secs(4), command.output()).await
        .map_err(|_| SshError::Ssh("reading OpenSSH configuration timed out".to_owned()))?;
    match output {
        Ok(output) if output.status.success() => {
            from_config(host, &String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => Err(SshError::Ssh(format!(
            "OpenSSH configuration: {}", String::from_utf8_lossy(&output.stderr).trim(),
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => from_config(host, ""),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn ssh_command() -> tokio::process::Command {
    #[cfg(windows)]
    let executable = {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join(r"System32\OpenSSH\ssh.exe");
        if system.is_file() {
            system
        } else {
            std::env::var_os("PATH").as_ref().and_then(|path| {
                std::env::split_paths(path).filter(|path| path.is_absolute())
                    .map(|path| path.join("ssh.exe")).find(|path| path.is_file())
            }).unwrap_or(system)
        }
    };
    #[cfg(not(windows))]
    let executable = "ssh";
    let mut command = tokio::process::Command::new(executable);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command.kill_on_drop(true);
    command
}

fn validate(host: &RemoteHost) -> Result<(), SshError> {
    let name = host.host.trim();
    let unsafe_field = |value: &str| value.starts_with('-') || value.chars().any(char::is_control);
    if name.is_empty() || unsafe_field(name) || name.chars().any(char::is_whitespace)
        || unsafe_field(host.user.trim())
        || host.key_path.as_deref().is_some_and(|key| unsafe_field(key.trim()))
        || host.port == Some(0)
    {
        return Err(SshError::Ssh("invalid SSH host, user, port, or key path".to_owned()));
    }
    Ok(())
}

fn from_config(host: &RemoteHost, text: &str) -> Result<ResolvedHost, SshError> {
    let fields: Vec<_> = text.lines().filter_map(|line| {
        let (key, value) = line.split_once(char::is_whitespace)?;
        Some((key, value.trim()))
    }).collect();
    let value = |field| fields.iter()
        .find_map(|(key, value)| (*key == field && !value.is_empty() && *value != "none").then_some(*value));
    if value("proxycommand").is_some() && value("proxyjump").is_none() {
        return Err(SshError::Ssh(
            "this SSH alias requires a custom ProxyCommand, which the embedded transport cannot run".to_owned(),
        ));
    }
    let local_user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let home = local_home();
    let mut config = ResolvedHost {
        hostname: value("hostname").unwrap_or(host.host.trim()).trim_matches(['[', ']']).to_owned(),
        user: if host.user.trim().is_empty() { value("user").unwrap_or(&local_user).to_owned() } else { host.user.trim().to_owned() },
        port: host.port.or_else(|| value("port").and_then(|port| port.parse().ok())).unwrap_or(22),
        host_key_alias: value("hostkeyalias").map(str::to_owned),
        proxy_jump: value("proxyjump").map(str::to_owned),
        identity_files: Vec::new(),
        explicit_key: host.key_path.as_deref().is_some_and(|path| !path.trim().is_empty()),
        identities_only: value("identitiesonly") == Some("yes"),
        agent: IdentityAgent::Default,
    };
    let mut identities = host.key_path.iter().map(String::as_str)
        .chain(fields.iter().filter_map(|(key, value)| (*key == "identityfile").then_some(*value)))
        .peekable();
    if identities.peek().is_none() && text.is_empty() {
        config.identity_files.extend(["id_ed25519", "id_ecdsa", "id_rsa"].map(|key| home.join(".ssh").join(key)));
    }
    for identity in identities.filter(|value| !value.is_empty() && *value != "none") {
        let path = expand_path(identity, host, &config, &home, &local_user);
        if !config.identity_files.contains(&path) {
            config.identity_files.push(path);
        }
    }
    let agent = host.auth_socket.as_deref().filter(|value| !value.trim().is_empty())
        .or_else(|| fields.iter().find_map(|(key, value)| (*key == "identityagent").then_some(*value)));
    config.agent = match agent.map(str::trim) {
        Some("none") => IdentityAgent::Disabled,
        Some("SSH_AUTH_SOCK" | "$SSH_AUTH_SOCK" | "${SSH_AUTH_SOCK}") | None => IdentityAgent::Default,
        Some(path) => IdentityAgent::Socket(expand_path(path, host, &config, &home, &local_user)),
    };
    Ok(config)
}

fn local_home() -> PathBuf {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn expand_path(path: &str, host: &RemoteHost, config: &ResolvedHost, home: &Path, local_user: &str) -> PathBuf {
    let port = config.port.to_string();
    let home_text = home.to_string_lossy();
    let mut chars = path.trim().chars();
    let mut expanded = String::new();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            expanded.push(ch);
            continue;
        }
        let replacement = match chars.next() {
            Some('%') => "%",
            Some('d') => &home_text,
            Some('h') => &config.hostname,
            Some('n') => host.host.trim(),
            Some('p') => &port,
            Some('r') => &config.user,
            Some('u') => local_user,
            Some(other) => {
                expanded.push('%');
                expanded.push(other);
                continue;
            }
            None => "%",
        };
        expanded.push_str(replacement);
    }
    for prefix in ["~/", "~\\", "%USERPROFILE%/", "%USERPROFILE%\\", "$HOME/"] {
        if let Some(relative) = expanded.strip_prefix(prefix) {
            return home.join(relative);
        }
    }
    if expanded == "~" {
        return home.to_path_buf();
    }
    PathBuf::from(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_alias_options_and_expands_identity_file_tokens() {
        let host = RemoteHost::new("build", "Build", "my-build");
        let text = "hostname dev.example\nport 2200\nuser runner\nidentityfile ~/.ssh/%n-%r\nidentityagent none\nidentitiesonly yes\nhostkeyalias ci-key\n";
        let config = from_config(&host, text).unwrap();
        assert_eq!(config.hostname, "dev.example");
        assert_eq!(config.port, 2200);
        assert_eq!(config.user, "runner");
        assert!(config.identity_files[0].ends_with(".ssh/my-build-runner"));
        assert!(config.identities_only);
        assert!(matches!(config.agent, IdentityAgent::Disabled));
        assert_eq!(config.host_key_alias.as_deref(), Some("ci-key"));
    }

    #[test]
    fn explicit_options_and_socket_override_alias() {
        let mut host = RemoteHost::new("build", "Build", "my-build");
        host.port = Some(22);
        host.user = "explicit".into();
        host.auth_socket = Some(r"\\.\pipe\password-manager-agent".into());
        let config = from_config(&host, "hostname dev.example\nport 2200\nuser runner\nidentityagent none\n").unwrap();
        assert_eq!((config.user.as_str(), config.port), ("explicit", 22));
        assert!(matches!(config.agent, IdentityAgent::Socket(path) if path == PathBuf::from(r"\\.\pipe\password-manager-agent")));
    }
}
