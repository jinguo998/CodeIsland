use std::process::Command;
use std::time::Duration;

use codeisland_domain::{AgentKind, CODEX_DESKTOP_SESSION_PREFIX, SessionSnapshot, claude_desktop_code, cowork};

struct Launch {
    program: &'static str,
    args: Vec<String>,
}

/// Focuses the session's existing host window, with a workspace launcher as a fallback.
pub async fn launch_for_session(session: &SessionSnapshot) -> bool {
    if let Some(activated) = focus_session_terminal(session).await {
        return activated;
    }

    let Some(cwd) = session
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
    else {
        return false;
    };

    for launch in launchers(session.agent, cwd) {
        if spawn(launch) {
            return true;
        }
    }
    false
}

pub(crate) async fn focus_session_terminal(session: &SessionSnapshot) -> Option<bool> {
    if session.session_id.starts_with("ssh:") {
        return Some(false);
    }
    if session.agent == AgentKind::Codex && session.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX) {
        return Some(focus_native_session(session, codex_desktop_thread_link(session)).await);
    }
    if crate::terminal::is_claude_desktop_session(session) {
        return Some(focus_native_session(session, claude_desktop_link(session)).await);
    }
    crate::terminal::activate(session).await
}

async fn focus_native_session(session: &SessionSnapshot, link: Option<String>) -> bool {
    let delivered = link.is_some_and(|link| tauri_plugin_opener::open_url(link, None::<&str>).is_ok());
    if !delivered {
        // Bring the app forward as a fallback. A failed URI delivery cannot
        // establish that the window opened the requested conversation.
        let _ = crate::terminal::activate(session).await;
        return false;
    }

    for millis in [120, 320, 640] {
        tokio::time::sleep(Duration::from_millis(millis)).await;
        if crate::terminal::is_frontmost(session) {
            return true;
        }
    }
    false
}

fn claude_desktop_link(session: &SessionSnapshot) -> Option<String> {
    if session.agent != AgentKind::Claude || session.session_id.starts_with("ssh:") {
        return None;
    }
    if session.session_id.starts_with(cowork::PREFIX) {
        return cowork::deep_link(&session.session_id);
    }
    let host_id = session.claude_desktop_session_id.as_deref().filter(|id| claude_desktop_code::valid_id(id))?;
    // Claude Desktop resolves Code /continue to /epitaxy/{host_id}. Its Windows
    // handler also accepts that route directly, including in the third-party shell.
    Some(format!("claude://claude.ai/epitaxy/{host_id}"))
}

fn codex_desktop_thread_link(session: &SessionSnapshot) -> Option<String> {
    if session.agent != AgentKind::Codex {
        return None;
    }
    let card_id = session.session_id.strip_prefix(CODEX_DESKTOP_SESSION_PREFIX)?;
    // Older snapshots recovered from a rollout only carried the namespaced
    // card ID. Its suffix is the provider thread ID for those snapshots.
    let thread_id = session.provider_session_id.as_deref().unwrap_or(card_id);
    let parsed = uuid::Uuid::parse_str(thread_id).ok()?;
    if !thread_id.eq_ignore_ascii_case(&parsed.hyphenated().to_string()) {
        return None;
    }
    Some(format!("codex://threads/{thread_id}"))
}

fn launchers(agent: AgentKind, cwd: &str) -> Vec<Launch> {
    match agent {
        // Cursor sessions open in Cursor, falling back to VS Code/Explorer.
        AgentKind::Cursor => vec![
            Launch {
                program: "cursor",
                args: vec![cwd.to_owned()],
            },
            Launch {
                program: "code",
                args: vec![cwd.to_owned()],
            },
            Launch {
                program: "explorer",
                args: vec![cwd.to_owned()],
            },
        ],
        // VS Code family agents.
        AgentKind::Copilot | AgentKind::Cline => vec![
            Launch {
                program: "code",
                args: vec![cwd.to_owned()],
            },
            Launch {
                program: "explorer",
                args: vec![cwd.to_owned()],
            },
        ],
        // Terminal agents open a Windows Terminal rooted at the session cwd.
        _ => vec![
            Launch {
                program: "wt.exe",
                args: vec!["-d".to_owned(), cwd.to_owned()],
            },
            Launch {
                program: "explorer",
                args: vec![cwd.to_owned()],
            },
        ],
    }
}

fn spawn(launch: Launch) -> bool {
    let mut command = Command::new(launch.program);
    command.args(&launch.args);

    // Avoid a console window flashing for console-subsystem launchers.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use codeisland_domain::{AgentEvent, AgentEventType};

    use super::*;

    fn session(agent: AgentKind, id: &str, provider_id: Option<&str>) -> SessionSnapshot {
        let mut snapshot = SessionSnapshot::new(&AgentEvent::new(id, agent, AgentEventType::SessionStart));
        snapshot.provider_session_id = provider_id.map(ToOwned::to_owned);
        snapshot
    }

    #[test]
    fn deep_link_is_only_for_a_well_formed_native_codex_thread() {
        let thread = "91919050-bca6-4123-9833-4396b13ed42b";
        let native = format!("codexapp:{thread}");
        assert_eq!(
            codex_desktop_thread_link(&session(AgentKind::Codex, &native, None)),
            Some(format!("codex://threads/{thread}")),
        );
        assert_eq!(codex_desktop_thread_link(&session(AgentKind::Codex, thread, Some(thread))), None);
        assert_eq!(codex_desktop_thread_link(&session(AgentKind::Claude, &native, None)), None);
        assert_eq!(codex_desktop_thread_link(&session(AgentKind::Codex, "codexapp:../other", None)), None);
    }

    #[test]
    fn deep_link_prefers_the_recorded_provider_thread() {
        let card = "codexapp:91919050-bca6-4123-9833-4396b13ed42b";
        let thread = "b71b633d-aeb2-4564-9004-852a363dfcac";
        assert_eq!(
            codex_desktop_thread_link(&session(AgentKind::Codex, card, Some(thread))),
            Some(format!("codex://threads/{thread}")),
        );
        assert_eq!(
            codex_desktop_thread_link(&session(AgentKind::Codex, card, Some("../other"))),
            None,
        );
    }

    #[test]
    fn claude_code_uses_the_host_id_without_redirecting_cli_or_remote_sessions() {
        let cli_id = "91919050-bca6-4123-9833-4396b13ed42b";
        let host_id = "local_b71b633d-aeb2-4564-9004-852a363dfcac";
        let mut code = session(AgentKind::Claude, cli_id, Some(cli_id));
        assert_eq!(claude_desktop_link(&code), None);
        code.claude_desktop_session_id = Some("local_x&session=last".to_owned());
        assert_eq!(claude_desktop_link(&code), None);
        code.claude_desktop_session_id = Some(host_id.to_owned());
        assert_eq!(
            claude_desktop_link(&code),
            Some("claude://claude.ai/epitaxy/local_b71b633d-aeb2-4564-9004-852a363dfcac".to_owned()),
        );
        code.session_id = format!("ssh:build:{cli_id}");
        assert_eq!(claude_desktop_link(&code), None);
    }

    #[test]
    fn cowork_keeps_its_own_route_even_when_code_metadata_is_present() {
        let id = "cowork:local_a1b2";
        let mut cowork = session(AgentKind::Claude, id, None);
        cowork.claude_desktop_session_id = Some("local_other".to_owned());
        assert_eq!(claude_desktop_link(&cowork), codeisland_domain::cowork::deep_link(id));
    }
}
