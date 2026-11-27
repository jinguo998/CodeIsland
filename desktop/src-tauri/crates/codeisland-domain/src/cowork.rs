//! Claude Desktop's local Cowork/Chat metadata and append-only audit stream.

use chrono::{DateTime, Utc};
use serde::Deserialize;

pub const PREFIX: &str = "cowork:";
pub const STORE_DIRECTORY: &str = "local-agent-mode-sessions";

pub fn valid_id(id: &str) -> bool {
    codeisland_swift::call(c"cowork.id", id)
}

pub fn deep_link(session_key: &str) -> Option<String> {
    codeisland_swift::call(c"cowork.link", session_key.strip_prefix(PREFIX)?)
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Metadata {
    pub id: String,
    pub cli_id: Option<String>,
    pub title: Option<String>,
    pub initial_prompt: Option<String>,
    pub host_cwd: Option<String>,
    pub model: Option<String>,
    #[serde(default, with = "chrono::serde::ts_microseconds_option")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, with = "chrono::serde::ts_microseconds_option")]
    pub last_activity: Option<DateTime<Utc>>,
    pub archived: bool,
    pub session_type: Option<String>,
}

impl Metadata {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        codeisland_swift::call_bytes(c"cowork.metadata", bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
    }

    pub fn trackable(&self) -> bool {
        codeisland_swift::call(c"cowork.trackable", serde_json::json!({"id":self.id,"archived":self.archived,"type":self.session_type}))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Tool {
    pub name: String,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Running,
    WaitingApproval,
    WaitingQuestion,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Audit {
    pub phase: Phase,
    pub prompt_count: u64,
    pub end_count: u64,
    pub request_count: u64,
    pub prompt: Option<String>,
    pub result: Option<String>,
    pub failed: bool,
    pub interrupted: bool,
    pub current_tool: Option<Tool>,
    permission: Option<Tool>,
}

impl Audit {
    pub fn active_permission(&self) -> Option<&Tool> { self.permission.as_ref() }
}

#[derive(Deserialize)]
pub struct AuditBatch {
    pub live: bool,
    pub prompts: bool,
    pub turns: bool,
    pub permissions: bool,
    pub audit: Audit,
}

#[derive(Default)]
pub struct AuditStream(codeisland_swift::Context);

impl AuditStream {
    /// Input contains complete JSONL records observed on this file generation.
    pub fn append(&mut self, records: &[u8]) -> AuditBatch {
        self.0.call_bytes(c"cowork.audit", records.strip_prefix(b"\xef\xbb\xbf").unwrap_or(records))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conversation {
    pub prompt: Option<String>,
    pub response: Option<String>,
    pub model: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Update {
    pub metadata: Metadata,
    pub audit: Audit,
    pub conversation: Conversation,
    pub transcript_path: Option<String>,
    pub last_activity: DateTime<Utc>,
    pub restore: bool,
    pub live: bool,
    pub settle: bool,
    pub prompts_started: bool,
    pub turns_completed: bool,
    pub permissions_requested: bool,
}
