//! Render the JSON a hook prints on stdout.
//!
//! The schemas are agent-specific and deliberately strict: an unknown key can
//! make Claude, Codex or ZCode ignore the whole decision. The renderer always
//! returns either a schema-valid JSON string or the caller's fallback.

use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, Decision, UserAction};
use serde_json::{Map, Value, json};

use crate::definitions::{ResponseFormat, definition};

/// Render the hook response for `decision`.
///
/// Returns `(stdout, exit_code)`. The exit code is always 0 because every
/// supported agent resolves approvals through stdout, not through the process
/// status.
pub fn render_response(
    agent: AgentKind,
    event: &AgentEvent,
    decision: Option<&Decision>,
    fallback: &str,
) -> (String, i32) {
    let Some(decision) = decision else {
        return (fallback.to_owned(), 0);
    };

    let response = definition(agent)
        .map(|def| def.response)
        .unwrap_or(ResponseFormat::Plain);
    let text = decision.text.clone().unwrap_or_default();

    let rendered = match response {
        ResponseFormat::Claude => Some(claude_response(event, decision, &text)),
        ResponseFormat::Codex => Some(codex_response(decision)),
        ResponseFormat::Notification => Some(notification_response(event, decision, &text)),
        ResponseFormat::Gemini | ResponseFormat::Cursor | ResponseFormat::Plain => {
            Some(plain_response(decision))
        }
    };

    match rendered {
        Some(value) => match serde_json::to_string(&value) {
            Ok(text) => (text, 0),
            Err(_) => (fallback.to_owned(), 0),
        },
        None => (fallback.to_owned(), 0),
    }
}

fn claude_response(event: &AgentEvent, decision: &Decision, text: &str) -> Value {
    match event.event_type {
        AgentEventType::Question => claude_question(event, decision, text),
        AgentEventType::PermissionRequest => claude_permission(event, decision),
        _ if is_notification(event) => {
            if is_denied(decision.action) {
                json!({ "hookSpecificOutput": { "hookEventName": "Notification" } })
            } else {
                notification_answer(text)
            }
        }
        _ => claude_permission(event, decision),
    }
}

fn claude_permission(event: &AgentEvent, decision: &Decision) -> Value {
    if is_denied(decision.action) {
        return json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "deny" }
            }
        });
    }

    let mut body = Map::new();
    body.insert("behavior".to_owned(), json!("allow"));

    if decision.action == UserAction::AlwaysAllow && codeisland_domain::approval::agent_keeps_always_rule(event.agent) {
        let tool = event.tool.clone().unwrap_or_default();
        let is_mcp = tool.starts_with("mcp__");
        let mut rule = Map::new();
        rule.insert("toolName".to_owned(), json!(tool));
        // MCP tools reject a `ruleContent` specifier: `mcp__server__tool(*)`
        // never matches an actual MCP call, so "always allow" would silently
        // fail to persist. MiniMax's Claude-compatible reader is the same:
        // it only understands a bare tool name. Every other tool keeps `*`.
        if !is_mcp && event.agent != AgentKind::MiniMax {
            rule.insert("ruleContent".to_owned(), json!("*"));
        }
        body.insert(
            "updatedPermissions".to_owned(),
            json!([{
                "type": "addRules",
                "rules": [Value::Object(rule)],
                "behavior": "allow",
                "destination": "session"
            }]),
        );
    }

    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": Value::Object(body)
        }
    })
}

fn claude_question(event: &AgentEvent, decision: &Decision, text: &str) -> Value {
    // A Notification question has no tool_input; Claude reads a different
    // response shape for it. A denial/skip closes the notification without an
    // answer.
    if is_notification(event) && tool_input(event).is_none() {
        if is_denied(decision.action) {
            return json!({ "hookSpecificOutput": { "hookEventName": "Notification" } });
        }
        return notification_answer(text);
    }
    if is_denied(decision.action) {
        return claude_permission(event, decision);
    }

    let mut updated = match tool_input(event) {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    // `questions` must always be present: Claude calls `.map()` on it while
    // mapping the tool result, and a missing key crashes the turn.
    let questions = updated
        .get("questions")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let answer_key = updated
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|questions| questions.first())
        .and_then(|question| question.get("header").or_else(|| question.get("question")))
        .and_then(Value::as_str)
        .unwrap_or("answer")
        .to_owned();
    let answers = serde_json::from_str::<Map<String, Value>>(text)
        .ok()
        .filter(|map| !map.is_empty())
        .unwrap_or_else(|| {
            let mut map = Map::new();
            map.insert(answer_key, json!(text));
            map
        });
    updated.insert("questions".to_owned(), questions);
    updated.insert("answers".to_owned(), Value::Object(answers));
    if !matches!(
        event.agent,
        AgentKind::Qoder | AgentKind::QoderCn | AgentKind::QoderWork
    ) {
        updated.insert("answer".to_owned(), json!(text));
    }

    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": {
                "behavior": "allow",
                "updatedInput": Value::Object(updated)
            }
        }
    })
}

fn codex_response(decision: &Decision) -> Value {
    // The runtime stages and saves the rule. Codex's hook gets plain allow.
    let behavior = match decision.action {
        UserAction::Deny | UserAction::Skip => "deny",
        _ => "allow",
    };
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": { "behavior": behavior }
        }
    })
}

fn notification_response(event: &AgentEvent, decision: &Decision, text: &str) -> Value {
    if event.event_type == AgentEventType::Question {
        if is_denied(decision.action) {
            json!({ "hookSpecificOutput": { "hookEventName": "Notification" } })
        } else {
            notification_answer(text)
        }
    } else if is_notification(event) && decision.action == UserAction::Answer {
        notification_answer(text)
    } else {
        plain_response(decision)
    }
}

fn plain_response(decision: &Decision) -> Value {
    let verdict = if is_denied(decision.action) {
        "deny"
    } else {
        "allow"
    };
    json!({ "decision": verdict })
}

fn notification_answer(text: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "Notification",
            "answer": text
        }
    })
}

fn is_denied(action: UserAction) -> bool {
    matches!(action, UserAction::Deny | UserAction::Skip)
}

fn is_notification(event: &AgentEvent) -> bool {
    event
        .raw_event_name
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case("Notification"))
}

fn tool_input(event: &AgentEvent) -> Option<&Value> {
    let payload = event.payload.as_ref()?;
    payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))
}
