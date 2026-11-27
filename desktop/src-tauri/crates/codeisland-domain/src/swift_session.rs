use serde::Deserialize;
use serde_json::json;

use crate::{AgentEvent, AgentEventType, AgentTask, SessionSnapshot, SessionStatus};

#[derive(Deserialize)]
pub(crate) struct Reduction {
    pub removed: bool,
    pub applied: bool,
    pub ended: bool,
    pub failed: bool,
    status: SessionStatus,
    tool: Option<String>,
    id: Option<String>,
    detail: Option<String>,
    executing: bool,
    prompt: Option<String>,
    reply: Option<String>,
    live: Option<String>,
    interrupted: bool,
    tasks: Vec<AgentTask>,
}

impl Reduction {
    pub(crate) fn project(self, session: &mut SessionSnapshot, event: &AgentEvent) {
        session.status = self.status;
        session.active_tool = self.tool;
        session.tool_use_id = self.id;
        session.active_tool_detail = self.detail;
        session.is_executing_tool = self.executing;
        session.last_prompt = self.prompt;
        session.last_message = self.reply;
        session.live_codex_output = self.live;
        session.interrupted = self.interrupted;
        session.last_turn_failed = self.failed;
        if session.tasks != self.tasks {
            session.tasks = self.tasks;
            session.tasks_updated_at = Some(event.timestamp);
        }
    }
}

pub(crate) fn reduce(
    core: &codeisland_swift::Context, session: &SessionSnapshot, event: &AgentEvent,
    tool_id: Option<&str>, fresh: bool, limit: usize,
) -> Option<Reduction> {
    // Watchers, app-server consent and custom CLI adapters also publish
    // desktop-only messages. Hook events carry their provider's wire name.
    let raw = event.raw_event_name.as_deref()?;
    if event.event_type.is_blocking() || matches!(event.event_type, AgentEventType::SubagentStart | AgentEventType::SubagentEnd) {
        return None;
    }
    core.call(c"session.reduce", json!({
        "event": raw, "source": event.agent, "card": event.session_id, "fresh": fresh,
        "payload": event.payload, "cwd": event.cwd, "tool": event.tool, "tool_id": tool_id,
        "prompt": (event.event_type == AgentEventType::UserPrompt).then_some(event.text.as_deref()).flatten(),
        "answer": matches!(event.event_type, AgentEventType::Message | AgentEventType::TurnEnd)
            .then_some(event.text.as_deref()).flatten(),
        "limit": limit, "previous": {
            "status":session.status, "executing":session.is_executing_tool, "tool":session.active_tool,
            "detail":session.active_tool_detail, "id":session.tool_use_id, "prompt":session.last_prompt,
            "reply":session.last_message, "live":session.live_codex_output, "interrupted":session.interrupted,
            "failed":session.last_turn_failed, "tasks":session.tasks,
        },
    }))
}
