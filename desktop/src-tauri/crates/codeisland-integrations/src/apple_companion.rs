//! State projection and wire frames for the read-only Apple Companion BLE service.

use chrono::{DateTime, Utc};
use codeisland_domain::{SessionSnapshot, StoreSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::json;
#[cfg(test)]
use unicode_segmentation::UnicodeSegmentation;

pub const SERVICE_UUID: u128 = 0x6D951BA3_8F41_4C45_9D8A_12085E0D7A10;
pub const NOTIFY_UUID: u128 = 0x25C1B67B_E903_4A0C_8A78_3EE8AB7317B7;
#[cfg(test)]
const HEADER_BYTES: usize = 15;
#[cfg(test)]
const MAX_CHUNKS: usize = 64;
#[cfg(test)]
const SECRET_PROMPT: &str = "Sensitive prompt — answer on this computer";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompanionStatus {
    Idle,
    Processing,
    Running,
    WaitingApproval,
    WaitingQuestion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingAction {
    Approval,
    Question,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessagePreview {
    pub role: MessageRole,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<String>,
    pub descriptions: Vec<String>,
    pub index: usize,
    pub total: usize,
    pub allows_multiple_selection: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPreview {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub source: String,
    pub status: CompanionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub messages: Vec<MessagePreview>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionState {
    pub version: u8,
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub source: String,
    pub status: CompanionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    pub messages: Vec<MessagePreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_action: Option<PendingAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<Question>,
    pub sessions: Vec<SessionPreview>,
    pub updated_at: String,
}

impl CompanionState {
    pub fn from_snapshot(
        snapshot: &StoreSnapshot,
        default_source: &str,
        sequence: u64,
        now: DateTime<Utc>,
    ) -> Self {
        let cards: Vec<_> = snapshot.sessions.iter().map(|session| json!({
            "id": session.session_id, "source": session.agent.tag(), "status": session.status,
            "executing": session.is_executing_tool, "activity": session.last_activity.timestamp_millis() as f64 / 1000.0,
            "cwd": session.cwd.as_ref().or(session.project.as_ref()), "tool": session.active_tool,
            "detail": session.active_tool_detail, "prompt": session.last_prompt, "reply": session.last_message,
            "messages": recent_messages(session),
        })).collect();
        codeisland_swift::call(c"companion.state", json!({
            "cards": cards, "requests": snapshot.pending, "current": snapshot.current_session_id,
            "source": default_source, "sequence": sequence, "now": now.timestamp_millis() as f64 / 1000.0,
        }))
    }

    /// The iPhone central accepts at most 64 chunks. Fit the available ATT
    /// size by dropping optional previews only if a client negotiated a small MTU.
    pub fn bluetooth_frames(&self, max_notification_bytes: usize) -> Vec<Vec<u8>> {
        codeisland_swift::call(c"companion.frames", serde_json::json!({
            "payload": self, "maximum": max_notification_bytes.min(i32::MAX as usize),
        }))
    }
}

/// The persisted Windows history interleaves UI events and chat messages.
fn recent_messages(session: &SessionSnapshot) -> Vec<serde_json::Value> {
    let mut messages: Vec<_> = session.history.iter().rev().filter_map(|event| {
        let user = match event.kind.as_str() {
            "userprompt" => true,
            "message" | "turnend" | "turnfailed" => false,
            _ => return None,
        };
        let text = event.text.as_deref().filter(|s| !s.trim().is_empty())?;
        Some(json!({"user": user, "text": text}))
    }).take(3).collect();
    messages.reverse();
    if messages.is_empty() {
        for (user, text) in [(true, &session.last_prompt), (false, &session.last_message)] {
            if let Some(text) = text { messages.push(json!({"user": user, "text": text})); }
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, SessionStore};
    use serde_json::{Value, json};

    fn decoded(frames: &[Vec<u8>], sequence: u64, max: usize) -> Value {
        assert!(!frames.is_empty());
        assert!(frames.len() <= MAX_CHUNKS);
        let mut data = Vec::new();
        for (index, frame) in frames.iter().enumerate() {
            assert!(frame.len() <= max);
            assert_eq!(&frame[..3], &[0x43, 0x49, 0x01]);
            assert_eq!(&frame[3..11], &sequence.to_be_bytes());
            assert_eq!(&frame[11..13], &(index as u16).to_be_bytes());
            assert_eq!(&frame[13..15], &(frames.len() as u16).to_be_bytes());
            data.extend_from_slice(&frame[HEADER_BYTES..]);
        }
        serde_json::from_slice(&data).unwrap()
    }

    #[test]
    fn publishes_the_swift_summary_and_never_leaks_a_secret_question() {
        let mut store = SessionStore::default();
        let mut event = AgentEvent::new("question", AgentKind::Codex, AgentEventType::Question);
        event.text = Some("private question text".into());
        event.cwd = Some(r"C:\dev\project".into());
        event.payload = Some(json!({
            "questions": [{
                "header": "Credentials", "question": "private question text",
                "options": ["private option"], "isSecret": true
            }]
        }));
        store.apply(&event);
        let now = "2026-11-11T08:00:00Z".parse().unwrap();
        let state = CompanionState::from_snapshot(&store.snapshot(), "claude", 257, now);
        let value = decoded(&state.bluetooth_frames(135), 257, 135);
        assert_eq!(value["version"], 1);
        assert_eq!(value["status"], "waitingQuestion");
        assert_eq!(value["pendingAction"], "question");
        assert_eq!(value["source"], "codex");
        assert_eq!(value["questionHeader"], "Credentials");
        assert_eq!(value["questionText"], SECRET_PROMPT);
        assert_eq!(value["workspaceName"], "project");
        assert_eq!(value["updatedAt"], "2026-11-11T08:00:00Z");
        assert_eq!(value["sessions"][0]["message"], SECRET_PROMPT);
        let serialized = serde_json::to_string(&state).unwrap();
        assert!(!serialized.contains("private question text") && !serialized.contains("private option"));
    }

    #[test]
    fn fits_small_notifications_without_breaking_the_ios_chunk_limit() {
        let mut store = SessionStore::default();
        for index in 0..7 {
            let mut event = AgentEvent::new(format!("session-{index}"), AgentKind::Claude, AgentEventType::Message);
            event.text = Some("👨‍👩‍👧‍👦".repeat(300));
            store.apply(&event);
        }
        let state = CompanionState::from_snapshot(&store.snapshot(), "claude", 6, Utc::now());
        assert_eq!(state.sessions.len(), 5);
        let large = decoded(&state.bluetooth_frames(135), 6, 135);
        assert_eq!(large["message"].as_str().unwrap().graphemes(true).count(), 220);
        let small = decoded(&state.bluetooth_frames(20), 6, 20);
        assert_eq!(small["sequence"], 6);
        assert_eq!(small["status"], large["status"]);
    }
}
