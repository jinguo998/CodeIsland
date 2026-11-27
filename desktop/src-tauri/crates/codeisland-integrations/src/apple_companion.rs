//! State projection and wire frames for the read-only Apple Companion BLE service.

use chrono::{DateTime, SecondsFormat, Utc};
use codeisland_domain::{
    PendingRequest, RequestKind, SessionSnapshot, SessionStatus, StoreSnapshot, project_name,
};
use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation;

pub const SERVICE_UUID: u128 = 0x6D951BA3_8F41_4C45_9D8A_12085E0D7A10;
pub const NOTIFY_UUID: u128 = 0x25C1B67B_E903_4A0C_8A78_3EE8AB7317B7;
#[cfg(test)]
const HEADER_BYTES: usize = 15;
#[cfg(test)]
const MAX_CHUNKS: usize = 64;
const SECRET_PROMPT: &str = "Sensitive prompt — answer on this computer";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompanionStatus {
    Idle,
    Processing,
    Running,
    WaitingApproval,
    WaitingQuestion,
}

impl CompanionStatus {
    fn of(session: &SessionSnapshot) -> Self {
        match session.status {
            SessionStatus::WaitingPermission => Self::WaitingApproval,
            SessionStatus::WaitingAnswer => Self::WaitingQuestion,
            SessionStatus::Running if session.is_executing_tool => Self::Running,
            SessionStatus::Starting | SessionStatus::Running => Self::Processing,
            _ => Self::Idle,
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::WaitingApproval => 5,
            Self::WaitingQuestion => 4,
            Self::Running => 3,
            Self::Processing => 2,
            Self::Idle => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingAction {
    Approval,
    Question,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessagePreview {
    pub role: MessageRole,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
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
        let visible_request = |kind| snapshot.pending.iter()
            .find(|request| request.kind == kind && !request.hidden);
        let permission = visible_request(RequestKind::Permission);
        let question = visible_request(RequestKind::Question);
        let request = permission.or(question);
        let session = request
            .map(|request| request.session_id.as_str())
            .or(snapshot.current_session_id.as_deref())
            .and_then(|id| snapshot.sessions.iter().find(|session| session.session_id == id))
            .or_else(|| snapshot.sessions.iter().min_by_key(|session| &session.session_id));
        let id = request.map(|request| request.session_id.clone())
            .or_else(|| session.map(|session| session.session_id.clone()));
        let status = match request.map(|request| request.kind) {
            Some(RequestKind::Permission) => CompanionStatus::WaitingApproval,
            Some(RequestKind::Question) => CompanionStatus::WaitingQuestion,
            None => session.map_or(CompanionStatus::Idle, CompanionStatus::of),
        };
        let source = if status == CompanionStatus::Idle {
            default_source
        } else if let Some(request) = request {
            request.agent.tag()
        } else {
            session.map_or(default_source, |session| session.agent.tag())
        };
        let tool = match request {
            Some(request) if request.kind == RequestKind::Question => {
                request.tool.clone().or_else(|| Some("AskUserQuestion".into()))
            }
            Some(request) => request.tool.clone().or_else(|| session.and_then(|session| session.active_tool.clone())),
            None if status != CompanionStatus::Idle => session.and_then(|session| session.active_tool.clone()),
            None => None,
        };
        let mut messages = session.map_or_else(Vec::new, |session| history(session, 3));
        if let Some(request) = request
            && let Some(text) = request_message(request)
            && messages.last().is_none_or(|last| last.text != text)
        {
            if messages.len() == 3 {
                messages.remove(0);
            }
            let role = if request.kind == RequestKind::Question { MessageRole::User } else { MessageRole::Assistant };
            messages.push(MessagePreview { role, text });
        }
        let mut sessions: Vec<_> = snapshot.sessions.iter().collect();
        sessions.sort_by(|left, right| {
            let primary = |session: &SessionSnapshot| id.as_deref() == Some(session.session_id.as_str());
            primary(right).cmp(&primary(left))
                .then_with(|| CompanionStatus::of(right).priority().cmp(&CompanionStatus::of(left).priority()))
                .then_with(|| right.last_activity.cmp(&left.last_activity))
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Self {
            version: 1,
            sequence,
            session_id: id,
            source: source.to_owned(),
            status,
            tool_name: tool,
            workspace_name: session.and_then(workspace),
            messages,
            pending_action: request.map(|request| match request.kind {
                RequestKind::Permission => PendingAction::Approval,
                RequestKind::Question => PendingAction::Question,
            }),
            question: question.and_then(question_payload),
            sessions: sessions.into_iter().take(5).map(|session| {
                let messages = history(session, 2);
                let pending = [RequestKind::Question, RequestKind::Permission].into_iter().find_map(|kind| {
                    snapshot.pending.iter().find(|request| {
                        request.session_id == session.session_id && request.kind == kind && !request.hidden
                    })
                });
                let status = CompanionStatus::of(session);
                let message = pending.and_then(request_message)
                    .or_else(|| messages.last().map(|last| last.text.clone()));
                SessionPreview {
                    session_id: Some(session.session_id.clone()),
                    source: session.agent.tag().to_owned(),
                    status,
                    tool_name: (status != CompanionStatus::Idle).then(|| session.active_tool.clone()).flatten(),
                    workspace_name: workspace(session),
                    message,
                    messages,
                    updated_at: date(session.last_activity),
                }
            }).collect(),
            updated_at: date(now),
        }
    }

    /// The iPhone central accepts at most 64 chunks. Fit the available ATT
    /// size by dropping optional previews only if a client negotiated a small MTU.
    pub fn bluetooth_frames(&self, max_notification_bytes: usize) -> Vec<Vec<u8>> {
        codeisland_swift::call(c"companion.frames", serde_json::json!({
            "payload": self, "maximum": max_notification_bytes.min(i32::MAX as usize),
        }))
    }
}

fn workspace(session: &SessionSnapshot) -> Option<String> {
    session.project.clone().or_else(|| project_name(session.cwd.as_deref()))
}

fn history(session: &SessionSnapshot, limit: usize) -> Vec<MessagePreview> {
    let mut previews: Vec<_> = session.history.iter().rev().filter_map(|event| {
        let role = match event.kind.as_str() {
            "userprompt" => MessageRole::User,
            "message" | "turnend" | "turnfailed" => MessageRole::Assistant,
            _ => return None,
        };
        Some(MessagePreview { role, text: preview(event.text.as_deref()?)? })
    }).take(limit).collect();
    if previews.is_empty() {
        for (role, text) in [
            (MessageRole::Assistant, session.last_message.as_deref()),
            (MessageRole::User, session.last_prompt.as_deref()),
        ] {
            if let Some(text) = text.and_then(preview) {
                previews.push(MessagePreview { role, text });
            }
        }
    }
    previews.reverse();
    previews
}

fn request_message(request: &PendingRequest) -> Option<String> {
    if request.kind == RequestKind::Question {
        return question_payload(request).and_then(|question| preview(&question.question));
    }
    request.body.as_deref().or(request.title.as_deref()).and_then(preview)
}

fn question_payload(request: &PendingRequest) -> Option<Question> {
    if let Some(item) = request.questions.first() {
        return Some(Question {
            header: item.header.clone(),
            question: if item.is_secret { SECRET_PROMPT.to_owned() } else { item.question.clone() },
            options: if item.is_secret { Vec::new() } else { item.options.clone() },
            descriptions: if item.is_secret { Vec::new() } else { item.option_descriptions.clone() },
            index: 1,
            total: request.questions.len(),
            allows_multiple_selection: item.multi_select,
        });
    }
    Some(Question {
        header: None,
        question: request.body.clone().or_else(|| request.title.clone())?,
        options: request.options.clone(),
        descriptions: Vec::new(),
        index: 1,
        total: 1,
        allows_multiple_selection: false,
    })
}

fn preview(text: &str) -> Option<String> {
    let one_line = text.replace('\n', " ");
    let trimmed = one_line.trim();
    (!trimmed.is_empty()).then(|| truncate(trimmed, 240, "..."))
}

fn truncate(text: &str, limit: usize, suffix: &str) -> String {
    if text.graphemes(true).take(limit + 1).count() <= limit {
        return text.to_owned();
    }
    let kept = limit.saturating_sub(suffix.graphemes(true).count());
    text.graphemes(true).take(kept).chain(std::iter::once(suffix)).collect()
}

fn date(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
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
