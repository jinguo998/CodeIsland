use serde::Deserialize;
use serde_json::json;

use crate::{AgentEvent, AgentEventType, SessionSnapshot, SessionStatus};

#[derive(Deserialize)]
struct ToolFields {
    tool: Option<String>,
    detail: Option<String>,
    id: Option<String>,
    executing: bool,
}

pub(crate) fn apply(session: &mut SessionSnapshot, event: &AgentEvent, id: Option<&str>) {
    if session.status == SessionStatus::Cancelled || session.status.needs_you() {
        return;
    }
    let step = match (event.event_type, event.raw_event_name.as_deref()) {
        (AgentEventType::ToolStart, _) => "PreToolUse",
        (_, Some("PermissionDenied" | "permission_denied")) => "PermissionDenied",
        (_, Some("PostToolUseFailure" | "post_tool_use_failure" | "postToolUseFailure")) => "PostToolUseFailure",
        _ => "PostToolUse",
    };
    let next: ToolFields = codeisland_swift::call(c"toolStep", json!({
        "payload": event.payload, "source": event.agent, "event": step, "step": step,
        "tool": event.tool, "tool_id": id,
        "previous": { "tool": session.active_tool, "detail": session.active_tool_detail,
                      "id": session.tool_use_id, "executing": session.is_executing_tool },
    }));
    if next.executing { session.status = SessionStatus::Running; }
    session.active_tool = next.tool;
    session.active_tool_detail = next.detail;
    session.tool_use_id = next.id;
    session.is_executing_tool = next.executing;
}
