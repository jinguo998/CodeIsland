//! Live process identity for transcripts stored in a WSL distribution.
//!
//! The WSL share exposes regular procfs files. It does not expose the
//! targets of /proc/PID/fd symlinks, so Codex rollouts are matched by the
//! Linux inode that the share exposes as a Windows file index, and mount ID.

use codeisland_domain::{SessionStatus, WslProcess};
use chrono::{DateTime, Utc};

#[derive(Debug, PartialEq, Eq)]
struct ProcStat {
    pid: u32,
    start_ticks: u64,
    tty: Option<String>,
}

fn proc_stat(value: &str) -> Option<ProcStat> {
    let (head, tail) = value.rsplit_once(')')?;
    let (pid, _) = head.split_once(" (")?;
    // Field 3 starts here. The executable in parentheses can contain spaces
    // and parentheses, so splitting the whole line would shift the offsets.
    let fields = tail.split_ascii_whitespace().collect::<Vec<_>>();
    if matches!(fields.first().copied()?, "Z" | "X" | "x") {
        return None;
    }
    let device = fields.get(4)?.parse::<i32>().ok()? as u32;
    let major = (device >> 8) & 0xfff;
    let minor = (device & 0xff) | ((device >> 12) & 0xfff00);
    Some(ProcStat {
        pid: pid.parse().ok()?,
        start_ticks: fields.get(19)?.parse().ok()?,
        tty: (major == 136).then(|| format!("/dev/pts/{minor}")),
    })
}

fn proc_uid(value: &str) -> Option<u32> {
    value.lines().find_map(|line| {
        line.strip_prefix("Uid:")?.split_ascii_whitespace().next()?.parse().ok()
    })
}

#[derive(Default)]
struct TerminalEnv {
    wt_session: Option<String>,
    wezterm_pane: Option<u32>,
    codex_home: Option<String>,
    claude_home: Option<String>,
    xdg_home: Option<String>,
    multiplexed: bool,
}

fn terminal_env(bytes: &[u8]) -> TerminalEnv {
    let mut env = TerminalEnv::default();
    for field in bytes.split(|byte| *byte == 0) {
        for (key, value) in [
            ("WT_SESSION=", &mut env.wt_session),
            ("CODEX_HOME=", &mut env.codex_home),
            ("CLAUDE_CONFIG_DIR=", &mut env.claude_home),
            ("XDG_CONFIG_HOME=", &mut env.xdg_home),
        ] {
            if let Some(field) = field.strip_prefix(key.as_bytes()) {
                *value = std::str::from_utf8(field).ok().map(ToOwned::to_owned);
            }
        }
        if let Some(value) = field.strip_prefix(b"WEZTERM_PANE=") {
            env.wezterm_pane = std::str::from_utf8(value).ok().and_then(|value| value.parse().ok());
        }
        env.multiplexed |= [b"TMUX=".as_slice(), b"STY="].into_iter().any(|prefix| {
            field.strip_prefix(prefix).is_some_and(|value| !value.is_empty())
        });
    }
    env.wt_session = env.wt_session.filter(|id| {
        id.len() == 36 && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
        })
    });
    if env.multiplexed {
        env.wt_session = None;
        env.wezterm_pane = None;
    }
    env
}

fn fd_identity(value: &str) -> Option<(u64, u64)> {
    let mut mount = None;
    let mut inode = None;
    for line in value.lines() {
        if let Some(value) = line.strip_prefix("mnt_id:") {
            mount = value.trim().parse().ok();
        } else if let Some(value) = line.strip_prefix("ino:") {
            inode = value.trim().parse().ok();
        }
    }
    Some((mount?, inode?))
}

fn mount_point_path(path: &str) -> String {
    // /proc/*/mountinfo escapes space, tab, newline, and backslash.
    path.replace(r"\040", " ").replace(r"\011", "\t")
        .replace(r"\012", "\n").replace(r"\134", "\\")
}

fn mount_id(mounts: &str, path: &str) -> Option<u64> {
    mounts.lines().filter_map(|line| {
        let mut fields = line.split_ascii_whitespace();
        let id = fields.next()?.parse().ok()?;
        let mount = mount_point_path(fields.nth(3)?);
        let covers = mount == "/" || path == mount || path.strip_prefix(&mount)
            .is_some_and(|rest| rest.starts_with('/'));
        covers.then_some((mount.len(), id))
    }).max().map(|(_, id)| id)
}

#[cfg(windows)]
use std::{collections::HashSet, fs::{self, File}, io::{self, Read}, path::{Path, PathBuf}};
#[cfg(windows)]
use codeisland_domain::{AgentKind, SessionSnapshot};
#[cfg(windows)]
use crate::wsl::{WslDistribution, WslTranscriptSource, valid_component};

#[cfg(windows)]
pub(crate) struct Observation {
    pub process: Option<WslProcess>,
    pub status: Option<(SessionStatus, DateTime<Utc>)>,
    pub settle: bool,
}

#[cfg(windows)]
pub(crate) struct DistributionProcesses {
    distro: WslDistribution,
    processes: Vec<ProviderProcess>,
    machine_id: Option<String>,
}

#[cfg(windows)]
enum Provider {
    Codex { files: HashSet<(u64, u64)>, mounts: String },
    Claude,
}

#[cfg(windows)]
struct ProviderProcess {
    identity: WslProcess,
    config: Vec<PathBuf>,
    provider: Provider,
}

#[cfg(windows)]
fn read_limited(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "WSL metadata exceeds limit"));
    }
    Ok(bytes)
}

#[cfg(windows)]
fn read_text(path: &Path, limit: u64) -> io::Result<String> {
    String::from_utf8(read_limited(path, limit)?).map_err(io::Error::other)
}

#[cfg(windows)]
fn stat_for(proc_path: &Path) -> io::Result<Option<ProcStat>> {
    read_text(&proc_path.join("stat"), 8192).map(|text| proc_stat(&text))
}

#[cfg(windows)]
fn absolute_linux_path(root: &Path, value: &str) -> Option<PathBuf> {
    let value = value.strip_prefix('/')?.trim_end_matches('/');
    if value.is_empty() {
        return Some(root.to_path_buf());
    }
    let components = value.split('/');
    if !components.clone().all(valid_component) {
        return None;
    }
    Some(components.fold(root.to_path_buf(), |path, component| path.join(component)))
}

#[cfg(windows)]
fn transcript_location(path: &Path) -> Option<(&str, String)> {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Component::Prefix(prefix) = components.next()? else { return None };
    let (Prefix::UNC(server, distro) | Prefix::VerbatimUNC(server, distro)) = prefix.kind()
    else { return None };
    let server = server.to_str()?;
    if !server.eq_ignore_ascii_case("wsl.localhost") && !server.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    if components.next()? != Component::RootDir { return None; }
    let mut linux = String::new();
    for component in components {
        let Component::Normal(component) = component else { return None };
        linux.push('/');
        linux.push_str(component.to_str()?);
    }
    Some((distro.to_str()?, linux))
}

#[cfg(windows)]
pub(crate) fn is_wsl_transcript(path: &Path) -> bool {
    transcript_location(path).is_some()
}

#[cfg(windows)]
fn file_inode(path: &Path) -> Option<u64> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = File::open(path).ok()?;
    let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: the File owns a live Windows handle, and a successful query
    // initializes the whole writable BY_HANDLE_FILE_INFORMATION.
    let info = unsafe {
        if GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) == 0 {
            return None;
        }
        info.assume_init()
    };
    let inode = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    (inode != 0).then_some(inode)
}

#[cfg(windows)]
impl DistributionProcesses {
    pub(crate) fn scan(distro: &WslDistribution) -> io::Result<Self> {
        let process_dir = distro.root.join("proc");
        let mut processes = Vec::new();
        for entry in fs::read_dir(&process_dir)?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok())
            else { continue };
            let path = entry.path();
            let Ok(command) = read_text(&path.join("comm"), 256) else { continue };
            let agent = match command.trim() {
                "codex" => AgentKind::Codex,
                "claude" => AgentKind::Claude,
                _ => continue,
            };
            if let Some(process) = ProviderProcess::read(distro, agent, pid, &path) {
                processes.push(process);
            }
        }
        let machine_id = read_text(&distro.root.join(r"etc\machine-id"), 256).ok()
            .map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
        Ok(Self { distro: distro.clone(), processes, machine_id })
    }

    pub(crate) fn transcript_roots(&self) -> impl Iterator<Item = (WslTranscriptSource, PathBuf)> + '_ {
        self.processes.iter().filter(|process| process.identity.tty.is_some()).flat_map(|process| {
            let (source, folder) = match process.provider {
                Provider::Codex { .. } => (WslTranscriptSource::Codex, "sessions"),
                Provider::Claude => (WslTranscriptSource::Claude, "projects"),
            };
            process.config.iter().map(move |root| (source, root.join(folder)))
        })
    }

    pub(crate) fn observe(&self, session: &SessionSnapshot) -> Option<Observation> {
        let transcript = Path::new(session.transcript_path.as_deref()?);
        let (name, linux_path) = transcript_location(transcript)?;
        if !name.eq_ignore_ascii_case(&self.distro.name) {
            return None;
        }
        let mut matched: Option<(&WslProcess, Option<(SessionStatus, DateTime<Utc>)>)> = None;
        match session.agent {
            AgentKind::Codex if !session.session_id.starts_with(codeisland_domain::CODEX_DESKTOP_SESSION_PREFIX) => {
                if let Some(inode) = file_inode(transcript) {
                    for process in &self.processes {
                        let Provider::Codex { files, mounts } = &process.provider else { continue };
                        if mount_id(mounts, &linux_path).is_some_and(|mount| files.contains(&(mount, inode))) {
                            if matched.is_some() { return None; }
                            matched = Some((&process.identity, None));
                        }
                    }
                }
            }
            AgentKind::Claude if session.claude_desktop_session_id.is_none() => {
                let id = session.provider_session_id.as_deref().unwrap_or(&session.session_id);
                for process in &self.processes {
                    if !matches!(process.provider, Provider::Claude) { continue; }
                    if let Some(record) = process.claude_record(id, self.machine_id.as_deref()) {
                        if matched.is_some() { return None; }
                        matched = Some((&process.identity, record.status));
                    }
                }
            }
            _ => return None,
        }
        if let Some((process, status)) = matched {
            let status = status.or_else(|| {
                // The saved tail cursor may already be at EOF when Windows
                // restarts in the middle of an open Codex turn.
                if session.agent != AgentKind::Codex || session.status != SessionStatus::Idle
                    || session.wsl_process.as_ref() == Some(process)
                {
                    return None;
                }
                let id = session.provider_session_id.as_deref().unwrap_or(&session.session_id);
                let summary = crate::tailer::read_recovery_summary(transcript, session.agent, id)?;
                if summary.raw_id.as_deref() != Some(id)
                    || summary.turn != Some(crate::tailer::RecoveredTurn::Running)
                {
                    return None;
                }
                let modified = DateTime::from(fs::metadata(transcript).ok()?.modified().ok()?);
                Some((SessionStatus::Running, modified))
            });
            return Some(Observation { process: Some(process.clone()), status, settle: false });
        }
        let previous = session.wsl_process.as_ref()?;
        let path = self.distro.root.join("proc").join(previous.pid.to_string());
        match stat_for(&path) {
            Ok(Some(stat)) if stat.pid == previous.pid && stat.start_ticks == previous.start_ticks => {
                // A process may resume a different rollout in the same tab.
                // Keep a route only while the live association still matches.
                Some(Observation { process: None, status: None, settle: false })
            }
            Ok(_) => Some(Observation { process: None, status: None, settle: true }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Some(Observation { process: None, status: None, settle: true })
            }
            Err(_) => None,
        }
    }
}

#[cfg(windows)]
impl ProviderProcess {
    fn read(distro: &WslDistribution, agent: AgentKind, pid: u32, path: &Path) -> Option<Self> {
        let stat = stat_for(path).ok()??;
        if stat.pid != pid || proc_uid(&read_text(&path.join("status"), 65536).ok()?)? != distro.uid {
            return None;
        }
        let env = read_limited(&path.join("environ"), 256 * 1024)
            .ok().map(|bytes| terminal_env(&bytes)).unwrap_or_default();
        let ProviderProcessConfig { mut roots, provider } = match agent {
            AgentKind::Codex => {
                let files = fs::read_dir(path.join("fdinfo")).ok()?.flatten()
                    .filter_map(|entry| fd_identity(&read_text(&entry.path(), 65536).ok()?))
                    .collect();
                let mounts = read_text(&path.join("mountinfo"), 512 * 1024).ok()?;
                let roots = env.codex_home.as_deref().and_then(|value| absolute_linux_path(&distro.root, value))
                    .into_iter().collect();
                ProviderProcessConfig { roots, provider: Provider::Codex { files, mounts } }
            }
            AgentKind::Claude => {
                let mut roots = Vec::new();
                if let Some(path) = env.claude_home.as_deref().and_then(|value| absolute_linux_path(&distro.root, value)) {
                    roots.push(path);
                }
                if let Some(path) = env.xdg_home.as_deref().and_then(|value| absolute_linux_path(&distro.root, value)) {
                    roots.extend([path.join("claude"), path.join("claude-code")]);
                }
                roots.extend([
                    distro.home.join(".claude"), distro.home.join(r".config\claude"),
                    distro.home.join(r".config\claude-code"),
                ]);
                ProviderProcessConfig { roots, provider: Provider::Claude }
            }
            _ => return None,
        };
        if stat_for(path).ok()?? != stat {
            return None;
        }
        roots.sort();
        roots.dedup();
        Some(Self {
            identity: WslProcess {
                distro: distro.name.clone(), pid, start_ticks: stat.start_ticks, tty: stat.tty,
                windows_terminal_session: env.wt_session, wezterm_pane: env.wezterm_pane,
            },
            config: roots,
            provider,
        })
    }

    fn claude_record(&self, id: &str, machine: Option<&str>) -> Option<ClaudeRecord> {
        self.config.iter().find_map(|root| {
            let path = root.join("sessions").join(format!("{}.json", self.identity.pid));
            let record = read_limited(&path, 65536).ok()?;
            claude_record(&record, id, &self.identity, machine)
        })
    }
}

#[cfg(windows)]
struct ProviderProcessConfig {
    roots: Vec<PathBuf>,
    provider: Provider,
}

struct ClaudeRecord {
    status: Option<(SessionStatus, DateTime<Utc>)>,
}

fn claude_record(bytes: &[u8], id: &str, process: &WslProcess, machine: Option<&str>) -> Option<ClaudeRecord> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value["sessionId"].as_str()? != id
        || value["pid"].as_u64()? != u64::from(process.pid)
        || value["procStart"].as_str()?.parse::<u64>().ok()? != process.start_ticks
        || value["hostSessionId"].is_string()
    {
        return None;
    }
    let (record_machine, _) = value["pidDomain"].as_str()?.strip_prefix("linux:")?.split_once(":pid:")?;
    if machine.is_some_and(|machine| record_machine != machine) {
        return None;
    }
    let status = value["statusUpdatedAt"].as_i64()
        .and_then(DateTime::from_timestamp_millis)
        .filter(|timestamp| *timestamp <= Utc::now())
        .and_then(|timestamp| match value["status"].as_str()? {
            "busy" => Some((SessionStatus::Running, timestamp)),
            "idle" => Some((SessionStatus::Idle, timestamp)),
            _ => None,
        });
    Some(ClaudeRecord { status })
}

/// Revalidate a previously observed route immediately before using its tty.
/// Procfs reads can block during WSL shutdown, so call on a blocking worker.
#[cfg(windows)]
pub fn wsl_process_is_current(process: &WslProcess) -> bool {
    if !valid_component(&process.distro) || process.pid == 0 {
        return false;
    }
    ["wsl.localhost", "wsl$"].into_iter().any(|server| {
        let path = PathBuf::from(format!(r"\\{server}\{}\proc\{}", process.distro, process.pid));
        let matches = |stat: ProcStat| {
            stat.pid == process.pid && stat.start_ticks == process.start_ticks && stat.tty == process.tty
        };
        if !stat_for(&path).ok().flatten().is_some_and(matches) {
            return false;
        }
        let Ok(bytes) = read_limited(&path.join("environ"), 256 * 1024) else { return false };
        let env = terminal_env(&bytes);
        !env.multiplexed
            && env.wt_session == process.windows_terminal_session
            && env.wezterm_pane == process.wezterm_pane
            && stat_for(&path).ok().flatten().is_some_and(matches)
    })
}

/// A live process can switch to a different conversation without changing
/// its PID or terminal. Re-check the session identity before an exact jump.
#[cfg(windows)]
pub fn wsl_session_is_current(session: &SessionSnapshot) -> bool {
    fn verify(session: &SessionSnapshot) -> Option<bool> {
        let expected = session.wsl_process.as_ref()?;
        let transcript = Path::new(session.transcript_path.as_deref()?);
        let (name, linux_path) = transcript_location(transcript)?;
        if !name.eq_ignore_ascii_case(&expected.distro) {
            return None;
        }
        let distro = crate::wsl::distribution_by_name(&expected.distro)?;
        let path = distro.root.join("proc").join(expected.pid.to_string());
        let process = ProviderProcess::read(&distro, session.agent, expected.pid, &path)?;
        if &process.identity != expected {
            return None;
        }
        match process.provider {
            Provider::Codex { files, mounts } => {
                Some(files.contains(&(mount_id(&mounts, &linux_path)?, file_inode(transcript)?)))
            }
            Provider::Claude => {
                let machine = read_text(&distro.root.join(r"etc\machine-id"), 256).ok();
                let id = session.provider_session_id.as_deref().unwrap_or(&session.session_id);
                Some(process.claude_record(id, machine.as_deref().map(str::trim)).is_some())
            }
        }
    }
    verify(session).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process() -> WslProcess {
        WslProcess {
            distro: "Ubuntu".into(), pid: 123, start_ticks: 1000, tty: Some("/dev/pts/12".into()),
            windows_terminal_session: None, wezterm_pane: None,
        }
    }

    #[test]
    fn proc_stat_accepts_complex_executable_names_and_decodes_tty() {
        let stat = "123 (codex () shell) S 1 2 3 34828 5 6 7 8 9 10 11 12 13 14 15 16 17 18 1000 20";
        assert_eq!(proc_stat(stat), Some(ProcStat {
            pid: 123, start_ticks: 1000, tty: Some("/dev/pts/12".into()),
        }));
        assert!(proc_stat(&stat.replace(") S ", ") Z ")).is_none());
        assert_eq!(proc_uid("Name:\tcodex\nUid:\t1000\t1000\t1000\t1000\n"), Some(1000));
    }

    #[test]
    fn fd_inode_is_qualified_by_longest_mount_component() {
        let mounts = "30 21 8:1 / / rw - ext4 root rw\n31 30 8:2 / /home/alice/.codex/a\\040b rw - ext4 root rw";
        assert_eq!(mount_id(mounts, "/home/alice/.codex/a b/session.jsonl"), Some(31));
        assert_eq!(mount_id(mounts, "/home/alice/.codex/a bb/session.jsonl"), Some(30));
        assert_eq!(fd_identity("pos:\t5\nmnt_id:\t31\nino:\t54632\n"), Some((31, 54632)));
        assert_eq!(fd_identity("ino:\t54632\n"), None);
    }

    #[test]
    fn terminal_env_and_claude_record_are_scoped_to_expected_identity() {
        let env = terminal_env(b"UNRELATED_KEY=unused\0WT_SESSION=01234567-89ab-cdef-0123-456789abcdef\0WEZTERM_PANE=9\0CODEX_HOME=/tmp/codex\0");
        assert_eq!(env.wt_session.as_deref(), Some("01234567-89ab-cdef-0123-456789abcdef"));
        assert_eq!(env.wezterm_pane, Some(9));
        assert_eq!(env.codex_home.as_deref(), Some("/tmp/codex"));
        let record = br#"{"sessionId":"selected","pid":123,"pidDomain":"linux:host:pid:[1]","procStart":"1000","status":"busy","statusUpdatedAt":1700000000000}"#;
        assert_eq!(claude_record(record, "selected", &process(), Some("host")).unwrap().status.unwrap().0, SessionStatus::Running);
        assert!(claude_record(record, "other", &process(), Some("host")).is_none());
        assert!(claude_record(record, "selected", &process(), Some("other")).is_none());
        assert!(claude_record(record, "selected", &WslProcess { start_ticks: 1001, ..process() }, Some("host")).is_none());
    }
}
