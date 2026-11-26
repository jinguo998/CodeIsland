//! Session metadata recovered from a provider's store, with no hook continuation.

use chrono::{DateTime, Utc};

use crate::{AgentKind, SessionStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredSession {
    pub session_id: String,
    pub provider_session_id: String,
    pub claude_desktop_session_id: Option<String>,
    pub agent: AgentKind,
    pub parent_session_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub last_prompt: Option<String>,
    pub last_message: Option<String>,
    pub transcript_path: String,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub modified_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub finished_subagent: bool,
}

pub fn same_transcript_path(first: &str, second: &str) -> bool {
    if first == second {
        return true;
    }
    let windows_path = |path: &str| {
        path.as_bytes().get(1) == Some(&b':') || path.starts_with("\\\\")
    };
    if cfg!(windows) || (windows_path(first) && windows_path(second)) {
        let normalized = |path: &str| {
            path.trim_start_matches("\\\\?\\").replace('/', "\\").to_ascii_lowercase()
        };
        normalized(first) == normalized(second)
    } else {
        false
    }
}
