use chrono::{DateTime, Utc};
use codeisland_domain::{
    AgentEvent, AgentKind, AppSettings, PendingRequest, PushKind, SessionSnapshot,
    follow_up::{Origin, Reminder},
};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Deserialize)]
pub(crate) struct PushText {
    pub title: String,
    pub headline: String,
    pub body: String,
}

#[derive(Deserialize)]
pub(crate) struct FormattedPush {
    pub full: PushText,
    pub brief: PushText,
}

pub(crate) enum Notice<'a> {
    Event(&'a AgentEvent),
    DisplayOnly(&'a AgentEvent),
    Reminder(&'a Reminder),
}

pub(crate) fn format_push(kind: PushKind, request: Option<&PendingRequest>, session: Option<&SessionSnapshot>,
    notice: Notice<'_>, settings: &AppSettings, now: DateTime<Utc>) -> FormattedPush
{
    let (event, display_only, since) = match notice {
        Notice::Event(event) => (Some(event), false, None),
        Notice::DisplayOnly(event) => (Some(event), true, None),
        Notice::Reminder(reminder) => (None, reminder.origin == Origin::DisplayOnly, Some(reminder.waiting_since)),
    };
    let id = event.map(|e| &e.session_id).or_else(|| session.map(|s| &s.session_id));
    let agent = event.map(|e| e.agent).or_else(|| session.map(|s| s.agent)).unwrap_or(AgentKind::Unknown);
    let text = event.and_then(|e| e.text.as_deref()).or_else(|| {
        matches!(kind, PushKind::Completion | PushKind::Error).then(|| session?.last_message.as_deref()).flatten()
    });
    let error = event.and_then(|e| e.payload.as_ref()).and_then(|p| p["error"].as_str().or_else(|| p["error"]["name"].as_str()));
    let terminal = session.filter(|s| !s.session_id.starts_with("ssh:")).and_then(|s| match s.terminal_kind.as_deref()? {
        "claude" => Some("Claude Desktop"),
        "codex" => Some("Codex"),
        "cursor" => Some("Cursor"),
        "windows-terminal" | "wt" => Some("Windows Terminal"),
        _ => None,
    });
    codeisland_swift::call(c"push.content", json!({
        "kind": kind, "id": id.map(String::as_str).unwrap_or_default(), "agent": agent.display_name(),
        "project": session.and_then(|s| s.project.as_deref()), "language": settings.language,
        "limit": settings.push_summary_length, "request": request, "tool": event.and_then(|e| e.tool.as_deref()),
        "text": text, "error": error, "terminal": terminal, "displayOnly": display_only,
        "since": since.map(|time| time.timestamp_millis()), "now": now.timestamp_millis(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEventType, SessionStore};

    #[test]
    fn secret_question_does_not_leave_the_device_in_a_push() {
        let mut event = AgentEvent::new("session", AgentKind::Codex, AgentEventType::Question);
        event.waits = true;
        event.title = Some("Secret title".into());
        event.text = Some("Secret prompt".into());
        event.payload = Some(json!({
            "questions": [{"question": "Secret prompt", "isSecret": true, "options": [{"label":"Secret option"}]}]
        }));
        let mut store = SessionStore::default();
        store.apply(&event);
        let settings = AppSettings { language: "en".into(), ..AppSettings::default() };
        let message = format_push(PushKind::Question, store.get_pending(&event.id),
            store.get("session"), Notice::Event(&event), &settings, event.timestamp);
        for text in [message.full, message.brief] {
            assert_eq!(text.headline, "Has a question");
            assert_eq!(text.body, "Sensitive prompt — answer it on your PC.");
        }
    }
}
