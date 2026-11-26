//! Optional forwarding of live hook events in the macOS webhook envelope.

use std::sync::OnceLock;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use codeisland_domain::{AgentEvent, AppSettings};
use serde_json::{Value, json};

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

pub fn forward(settings: &AppSettings, source: &str, event: &AgentEvent) {
    if !settings.webhook_enabled {
        return;
    }
    let Ok(endpoint) = reqwest::Url::parse(settings.webhook_url.trim()) else {
        return;
    };
    if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host().is_none() {
        return;
    }
    let Some(body) = envelope(&settings.webhook_event_filter, source, event) else {
        return;
    };

    tokio::spawn(async move {
        // A receiver that is slow or offline must never hold the agent hook.
        let _ = HTTP
            .get_or_init(reqwest::Client::new)
            .post(endpoint)
            .timeout(Duration::from_secs(5))
            .header("User-Agent", "CodeIsland-Webhook/1.0")
            .json(&body)
            .send()
            .await;
    });
}

fn envelope(filter: &str, source: &str, event: &AgentEvent) -> Option<Value> {
    let raw_event = event.raw_event_name.as_deref()?;
    let normalized = normalize_hook_name(raw_event);
    if !filter.trim().is_empty()
        && !filter
            .split(',')
            .map(str::trim)
            .any(|name| name == normalized || name == raw_event)
    {
        return None;
    }
    let raw = event.payload.as_ref().cloned().unwrap_or_else(|| json!({}));
    let source = source.strip_prefix("codeisland-").unwrap_or(source);
    Some(json!({
        "event": normalized,
        "raw_event": raw_event,
        "session_id": event.session_id,
        "source": source,
        "cwd": event.cwd.as_deref().unwrap_or_default(),
        "tool_name": event.tool.as_deref().unwrap_or_default(),
        "timestamp": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "raw": raw,
    }))
}

/// Keep raw-event filters compatible with CodeIslandCore's EventNormalizer.
fn normalize_hook_name(name: &str) -> &str {
    match name {
        "beforeSubmitPrompt" | "userPromptSubmitted" | "userPromptSubmit"
        | "user_prompt_submit" | "pre_llm_call" | "TaskResume" => "UserPromptSubmit",
        "beforeShellExecution" | "beforeReadFile" | "beforeMCPExecution" | "preToolUse"
        | "pre_tool_use" | "pre_tool_call" => "PreToolUse",
        "afterShellExecution" | "afterFileEdit" | "afterMCPExecution" | "AfterTool"
        | "postToolUse" | "post_tool_use" | "post_tool_call" => "PostToolUse",
        "afterAgentThought" | "errorOccurred" | "notification" => "Notification",
        "afterAgentResponse" => "AfterAgentResponse",
        "stop" | "stop_failure" | "StopFailure" | "on_session_end" => "Stop",
        "BeforeTool" | "permission_request" => "PermissionRequest",
        "BeforeAgent" | "subagent_start" => "SubagentStart",
        "AfterAgent" | "subagent_stop" => "SubagentStop",
        "sessionStart" | "agentSpawn" | "session_start" | "on_session_start" | "TaskStart" => {
            "SessionStart"
        }
        "sessionEnd" | "session_end" | "on_session_finalize" | "on_session_reset" => "SessionEnd",
        "post_tool_use_failure" => "PostToolUseFailure",
        "permission_denied" => "PermissionDenied",
        "pre_compact" => "PreCompact",
        "post_compact" => "PostCompact",
        "post_llm_call" => "AgentTurnSettled",
        "TaskComplete" | "TaskCancel" => "TaskRoundComplete",
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEventType, AgentKind};

    #[test]
    fn accepts_raw_and_normalized_filters_and_preserves_the_payload() {
        let mut event = AgentEvent::new("session-1", AgentKind::Cursor, AgentEventType::ToolStart);
        event.raw_event_name = Some("beforeShellExecution".to_owned());
        event.tool = Some("shell".to_owned());
        event.payload = Some(json!({"command": "echo hi"}));

        let body = envelope("SessionEnd, PreToolUse", "codeisland-cursor", &event).unwrap();
        assert_eq!(body["event"], "PreToolUse");
        assert_eq!(body["raw_event"], "beforeShellExecution");
        assert_eq!(body["session_id"], "session-1");
        assert_eq!(body["source"], "cursor");
        assert_eq!(body["cwd"], "");
        assert_eq!(body["tool_name"], "shell");
        assert_eq!(body["raw"]["command"], "echo hi");
        assert!(envelope("beforeShellExecution", "cursor", &event).is_some());
        assert!(envelope("SessionEnd", "cursor", &event).is_none());
    }
}
