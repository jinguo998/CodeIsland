use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AgentEvent, AgentEventType, AgentTask, SessionSnapshot};

#[derive(Debug, Clone)]
pub enum TranscriptMetadata {
    Recap(String),
    Checklist(Value),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPresentation {
    pub prompt: Option<String>,
    pub reply: Option<String>,
    pub public_output: Option<String>,
    recap: Option<String>,
    checklist: bool,
}

impl TranscriptPresentation {
    pub fn read(row: &Value) -> Self {
        codeisland_swift::call(c"transcript.kind", row)
    }

    pub fn metadata(&self, row: &Value) -> Option<TranscriptMetadata> {
        self.recap.clone().map(TranscriptMetadata::Recap)
            .or_else(|| self.checklist.then(|| TranscriptMetadata::Checklist(row.clone())))
    }
}

pub fn claude_transcript_metadata(row: &Value) -> Option<TranscriptMetadata> {
    TranscriptPresentation::read(row).metadata(row)
}

pub fn codex_transcript_checklist(payload: &Value) -> Option<TranscriptMetadata> {
    claude_transcript_metadata(&json!({ "type": "response_item", "payload": payload }))
}

pub(crate) fn apply_transcript_checklist(
    core: &codeisland_swift::Context, session: &mut SessionSnapshot, row: &Value, at: DateTime<Utc>,
) -> bool {
    apply_checklist(core, session, json!({"row": row}), at)
}

pub(crate) fn apply_hook_checklist(core: &codeisland_swift::Context, session: &mut SessionSnapshot, event: &AgentEvent) {
    let name = match event.event_type {
        AgentEventType::ToolStart => "PreToolUse",
        AgentEventType::ToolEnd => "PostToolUse",
        AgentEventType::UserPrompt => "UserPromptSubmit",
        AgentEventType::TurnEnd | AgentEventType::TurnFailed => "Stop",
        AgentEventType::Interrupt => "Interrupt",
        _ => return,
    };
    apply_checklist(core, session, json!({
        "event": event.raw_event_name.as_deref().unwrap_or(name), "payload": event.payload,
        "tool": event.tool, "source":event.agent, "shared":event.agent_id.is_some(),
    }), event.timestamp);
}

fn apply_checklist(core: &codeisland_swift::Context, session: &mut SessionSnapshot, mut args: Value, at: DateTime<Utc>) -> bool {
    if session.tasks_updated_at.is_some_and(|previous| previous > at) { return false; }
    args["card"] = json!(session.session_id);
    args["tasks"] = json!(session.tasks);
    args["now"] = json!(at.timestamp_micros());
    let Some(rows): Option<Vec<AgentTask>> = core.call(c"session.checklist", args) else { return false };
    session.tasks_updated_at = Some(at);
    if session.tasks == rows { return false; }
    session.tasks = rows;
    true
}
