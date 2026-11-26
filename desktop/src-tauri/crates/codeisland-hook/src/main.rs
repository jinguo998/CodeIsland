//! Tiny one-shot hook client.
//!
//! Agent CLIs invoke this on hook events. It normalizes the raw payload, sends
//! one IPC request, and prints the app's response. It must never fail the
//! calling CLI: every runtime error falls back to a safe agent-specific reply
//! and exit code 0.

use std::io::{Read, Write};
use std::time::Duration;

use codeisland_agents::ResponseFormat;
use codeisland_domain::{AgentEventType, AgentKind};
use codeisland_ipc::{client, ClientMessage, ServerMessage, PROTOCOL_VERSION};

const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 1_000;
/// Matches the runtime's default blocking-decision budget, so the hook keeps
/// listening at least as long as the server keeps the request open.
const DEFAULT_DECISION_TIMEOUT_MS: u64 = 86_400_000;
const SHORT_DECISION_TIMEOUT_MS: u64 = 60_000;
const READ_TIMEOUT_GRACE_MS: u64 = 2_000;

const HELP: &str = "\
codeisland-hook - one-shot CodeIsland IPC client

Usage:
  codeisland-hook send --source <tag> --event <Name> [--stdin] [--file <path>]
                       [--timeout-ms <N>] [--connect-timeout-ms <N>]
  codeisland-hook --version
  codeisland-hook --help

Options:
  --source <tag>            Agent source tag (claude, codex, gemini, ...)
  --event <Name>            Raw hook event name
  --stdin                   Read the raw hook payload from stdin
  --file <path>             Read the raw hook payload from a file
  --timeout-ms <N>          Decision timeout forwarded to CodeIsland
  --connect-timeout-ms <N>  How long to wait for the app socket (default: 1000)
";

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("send") => run_send(&args[1..]),
        Some("--version" | "-V") => {
            println!("codeisland-hook {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--help" | "-h") | None => {
            print!("{HELP}");
            0
        }
        Some(other) => {
            eprintln!("codeisland-hook: unknown command {other:?}");
            eprint!("{HELP}");
            2
        }
    }
}

fn run_send(args: &[String]) -> i32 {
    let options = match SendOptions::parse(args) {
        Ok(options) => options,
        Err(error) => {
            debug(&error);
            return 0;
        }
    };

    let fallback = fallback_for(&options.source);

    let raw = match read_payload(&options) {
        Ok(raw) => raw,
        Err(error) => {
            debug(&format!("failed to read payload: {error}"));
            emit(fallback);
            return 0;
        }
    };
    // The agent spawns one hook process per event, so the parent pid is the
    // only identity stable across a session when stdin carries no session id.
    // Inject it into the payload (and pass it explicitly) so normalize can
    // build `<source>-ppid-<ppid>` instead of the shared "default".
    let ppid = parent_process_id();
    let raw = inject_ppid(&raw, ppid);

    // Antigravity fails closed on empty stdout: answer its tool gate before
    // anything else, exactly like the macOS bridge.
    let source_tag = options.source.trim_start_matches("codeisland-");
    if matches!(source_tag, "antigravity" | "google-antigravity")
        && options.event.eq_ignore_ascii_case("PreToolUse")
    {
        emit(r#"{"decision":"ask"}"#);
        return 0;
    }

    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    let mut event = match codeisland_agents::normalize_with_ppid(
        &options.source,
        &options.event,
        &raw,
        cwd.as_deref(),
        Some(ppid),
    ) {
        Ok(event) => event,
        Err(error) => {
            debug(&format!("failed to normalize payload: {error:?}"));
            emit(fallback);
            return 0;
        }
    };
    if event.agent == AgentKind::Claude
        && let Some(host_id) = env_value("CLAUDE_CODE_HOST_SESSION_ID")
            .filter(|id| codeisland_domain::claude_desktop_code::valid_id(id))
        && let Some(payload) = event.payload.as_mut().and_then(serde_json::Value::as_object_mut)
    {
        payload.insert("_claude_desktop_session".to_owned(), host_id.into());
    }
    event.wezterm_pane = event.wezterm_pane.or_else(|| env_value("WEZTERM_PANE")?.parse().ok());
    if event.wezterm_pane.is_some() {
        event.terminal_kind = Some("WezTerm".to_owned());
    } else if event.terminal_kind.is_none() {
        event.terminal_kind = if env_value("WT_SESSION").is_some() {
            Some("Windows Terminal".to_owned())
        } else {
            env_value("TERM_PROGRAM")
        };
    }

    let waits = matches!(
        event.event_type,
        AgentEventType::PermissionRequest | AgentEventType::Question
    );
    let message = ClientMessage::Event {
        v: PROTOCOL_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        source: options.source.clone(),
        event,
        waits,
        timeout_ms: options.timeout_ms,
    };

    let connect_timeout = Duration::from_millis(options.connect_timeout_ms);
    let default_timeout = if waits {
        DEFAULT_DECISION_TIMEOUT_MS
    } else {
        SHORT_DECISION_TIMEOUT_MS
    };
    let read_timeout = Duration::from_millis(
        options
            .timeout_ms
            .unwrap_or(default_timeout)
            .saturating_add(READ_TIMEOUT_GRACE_MS),
    );

    match client::send(&message, connect_timeout, read_timeout) {
        Ok(ServerMessage::Decision {
            stdout, exit_code, ..
        }) => {
            emit(&stdout);
            exit_code
        }
        Ok(ServerMessage::Ack { .. }) => {
            emit(fallback);
            0
        }
        Ok(ServerMessage::Error { error, .. }) => {
            debug(&format!("server error: {error}"));
            emit(fallback);
            0
        }
        Err(error) => {
            debug(&format!("ipc send failed: {error}"));
            emit(fallback);
            0
        }
    }
}

/// Reply printed when there is no decision. Claude, Codex, Gemini and Cursor
/// accept an empty JSON object; plain-text consumers get nothing.
fn fallback_for(source: &str) -> &'static str {
    if let Some(definition) = codeisland_agents::definition_for_source(source) {
        return match &definition.response {
            ResponseFormat::Plain => "",
            _ => "{}",
        };
    }
    match AgentKind::from_tag(source) {
        AgentKind::Claude | AgentKind::Codex | AgentKind::Gemini | AgentKind::Cursor => "{}",
        _ => "",
    }
}

fn read_payload(options: &SendOptions) -> std::io::Result<String> {
    if let Some(path) = &options.file {
        return std::fs::read_to_string(path);
    }
    if options.stdin {
        let mut raw = String::new();
        std::io::stdin().read_to_string(&mut raw)?;
        return Ok(raw);
    }
    Ok(String::new())
}

/// Add `_ppid` to an object payload. Empty stdin becomes `{"_ppid":N}`;
/// non-JSON input is left alone so normalize can fail open as before.
fn inject_ppid(raw: &str, ppid: u32) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return format!("{{\"_ppid\":{ppid}}}");
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return raw.to_owned();
    };
    if let Some(object) = value.as_object_mut() {
        object
            .entry("_ppid")
            .or_insert_with(|| serde_json::Value::from(ppid));
        return value.to_string();
    }
    raw.to_owned()
}

/// The pid of the process that spawned this hook (the agent CLI).
#[cfg(unix)]
fn parent_process_id() -> u32 {
    let pid = unsafe { libc::getppid() };
    if pid > 0 {
        pid as u32
    } else {
        std::process::id()
    }
}

/// Windows has no `getppid`; walk a Toolhelp process snapshot to our own
/// entry and read its parent. Falls back to our own pid on any failure.
#[cfg(windows)]
fn parent_process_id() -> u32 {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let own = std::process::id();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return own;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut parent = own;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == own {
                    parent = entry.th32ParentProcessID;
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        parent
    }
}

#[cfg(not(any(unix, windows)))]
fn parent_process_id() -> u32 {
    std::process::id()
}

/// Write hook output verbatim: no added newline, nothing at all when empty.
fn emit(text: &str) {
    if !text.is_empty() {
        print!("{text}");
        let _ = std::io::stdout().flush();
    }
}

fn debug(message: &str) {
    if std::env::var("CODEISLAND_DEBUG").ok().as_deref() == Some("1") {
        eprintln!("codeisland-hook: {message}");
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SendOptions {
    source: String,
    event: String,
    stdin: bool,
    file: Option<String>,
    timeout_ms: Option<u64>,
    connect_timeout_ms: u64,
}

impl SendOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut source: Option<String> = None;
        let mut event: Option<String> = None;
        let mut stdin = false;
        let mut file: Option<String> = None;
        let mut timeout_ms: Option<u64> = None;
        let mut connect_timeout_ms = DEFAULT_CONNECT_TIMEOUT_MS;

        let mut index = 0;
        while index < args.len() {
            let arg = args[index].clone();
            let (flag, inline) = match arg.split_once('=') {
                Some((flag, value)) => (flag, Some(value)),
                None => (arg.as_str(), None),
            };

            match flag {
                "--source" => source = Some(take_value(args, &mut index, inline, "--source")?),
                "--event" => event = Some(take_value(args, &mut index, inline, "--event")?),
                "--file" => file = Some(take_value(args, &mut index, inline, "--file")?),
                "--timeout-ms" => {
                    let value = take_value(args, &mut index, inline, "--timeout-ms")?;
                    timeout_ms = Some(parse_u64("--timeout-ms", &value)?);
                }
                "--connect-timeout-ms" => {
                    let value = take_value(args, &mut index, inline, "--connect-timeout-ms")?;
                    connect_timeout_ms = parse_u64("--connect-timeout-ms", &value)?;
                }
                "--stdin" => {
                    if inline.is_some() {
                        return Err("--stdin does not take a value".to_string());
                    }
                    stdin = true;
                }
                other => return Err(format!("unknown argument: {other}")),
            }

            index += 1;
        }

        if stdin && file.is_some() {
            return Err("--stdin and --file cannot be combined".to_string());
        }

        // The `codeisland send` CLI passes these through the environment
        // instead of repeating them on the command line.
        let source = source.or_else(|| env_value("CODEISLAND_SOURCE"));
        let event = event.or_else(|| env_value("CODEISLAND_EVENT"));

        Ok(Self {
            source: source.ok_or_else(|| "missing required --source".to_string())?,
            event: event.ok_or_else(|| "missing required --event".to_string())?,
            stdin,
            file,
            timeout_ms,
            connect_timeout_ms,
        })
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn take_value(
    args: &[String],
    index: &mut usize,
    inline: Option<&str>,
    name: &str,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value.to_string());
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{name} requires a value"))
}

fn parse_u64(name: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("{name} expects an integer, got {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_minimal_send() {
        let options = SendOptions::parse(&args(&[
            "--source",
            "claude",
            "--event",
            "PermissionRequest",
        ]))
        .expect("parse");
        assert_eq!(options.source, "claude");
        assert_eq!(options.event, "PermissionRequest");
        assert!(!options.stdin);
        assert_eq!(options.file, None);
        assert_eq!(options.timeout_ms, None);
        assert_eq!(options.connect_timeout_ms, DEFAULT_CONNECT_TIMEOUT_MS);
    }

    #[test]
    fn parses_flags_and_equals_form() {
        let options = SendOptions::parse(&args(&[
            "--source=codex",
            "--event=PreToolUse",
            "--stdin",
            "--timeout-ms=5000",
            "--connect-timeout-ms",
            "250",
        ]))
        .expect("parse");
        assert_eq!(options.source, "codex");
        assert_eq!(options.event, "PreToolUse");
        assert!(options.stdin);
        assert_eq!(options.timeout_ms, Some(5_000));
        assert_eq!(options.connect_timeout_ms, 250);
    }

    #[test]
    fn parses_file_payload() {
        let options = SendOptions::parse(&args(&[
            "--source",
            "cursor",
            "--event",
            "beforeShellExecution",
            "--file",
            "payload.json",
        ]))
        .expect("parse");
        assert_eq!(options.file.as_deref(), Some("payload.json"));
        assert!(!options.stdin);
    }

    #[test]
    fn rejects_missing_required_arguments() {
        assert!(SendOptions::parse(&args(&["--event", "X"])).is_err());
        assert!(SendOptions::parse(&args(&["--source", "claude"])).is_err());
    }

    #[test]
    fn rejects_unknown_or_invalid_arguments() {
        assert!(SendOptions::parse(&args(&["--source", "claude", "--event", "X", "--bogus"])).is_err());
        assert!(
            SendOptions::parse(&args(&["--source", "claude", "--event", "X", "--timeout-ms", "soon"]))
                .is_err()
        );
        assert!(
            SendOptions::parse(&args(&["--source", "claude", "--event", "X", "--stdin=yes"])).is_err()
        );
        assert!(SendOptions::parse(&args(&["--source", "claude", "--event"])).is_err());
    }

    #[test]
    fn rejects_conflicting_payload_sources() {
        assert!(
            SendOptions::parse(&args(&[
                "--source", "claude", "--event", "X", "--stdin", "--file", "p.json",
            ]))
            .is_err()
        );
    }

    #[test]
    fn unknown_source_has_empty_fallback() {
        assert_eq!(fallback_for("definitely-not-an-agent"), "");
    }

    #[test]
    fn ppid_is_injected_without_clobbering_the_payload() {
        let injected = inject_ppid(r#"{"session_id":"s"}"#, 99);
        let value: serde_json::Value = serde_json::from_str(&injected).expect("json");
        assert_eq!(value["session_id"], "s");
        assert_eq!(value["_ppid"], 99);

        // An agent that already reports a ppid keeps its own value.
        let injected = inject_ppid(r#"{"_ppid":7}"#, 99);
        let value: serde_json::Value = serde_json::from_str(&injected).expect("json");
        assert_eq!(value["_ppid"], 7);

        // Empty stdin still gives normalize an identity to fall back on.
        let value: serde_json::Value = serde_json::from_str(&inject_ppid("", 5)).expect("json");
        assert_eq!(value["_ppid"], 5);

        // Non-JSON stdin is left untouched (normalize fails open as before).
        assert_eq!(inject_ppid("not json", 5), "not json");
    }

    #[test]
    fn parent_process_id_is_nonzero() {
        assert_ne!(parent_process_id(), 0);
    }
}
