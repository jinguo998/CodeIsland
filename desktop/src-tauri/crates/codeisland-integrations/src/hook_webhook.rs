//! Optional forwarding of live hook events in the macOS webhook envelope.

use std::sync::OnceLock;
use std::time::Duration;

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
    codeisland_swift::call(c"hook.webhook", json!({
        "filter": filter, "event": event.raw_event_name.as_deref()?, "card": event.session_id,
        "source": source.trim_start_matches("codeisland-"), "cwd": event.cwd,
        "tool": event.tool, "payload": event.payload,
    }))
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
