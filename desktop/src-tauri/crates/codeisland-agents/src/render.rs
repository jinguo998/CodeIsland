//! Translate a named-pipe decision to the provider's stdout contract.
use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, Decision};
use serde_json::json;

use crate::definitions::{definition_for_source, ResponseFormat};

pub fn render_response(
    agent: AgentKind, event: &AgentEvent, decision: Option<&Decision>, fallback: &str,
) -> (String, i32) {
    let Some(decision) = decision else { return (fallback.to_owned(), 0) };
    let source = event.payload.as_ref().and_then(|p| p["_source"].as_str())
        .unwrap_or(agent.hook_source());
    let format = definition_for_source(source).map(|d| d.response).unwrap_or(ResponseFormat::Plain);
    let answer: Option<String> = codeisland_swift::call(c"hook.answer", json!({
        "format": format, "action": decision.action, "text": decision.text,
        "question": event.event_type == AgentEventType::Question,
        "event": event.raw_event_name.as_deref().unwrap_or("PermissionRequest"),
        "source": source, "tool": event.tool, "payload": event.payload,
    }));
    (answer.unwrap_or_else(|| fallback.to_owned()), 0)
}
