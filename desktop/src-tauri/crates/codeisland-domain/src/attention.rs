use std::collections::HashSet;

use chrono::Timelike;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{
    AgentEvent, AgentEventType, ApplyOutcome, PendingRequest, RequestKind,
};

pub const CUE_REQUEST: &str = "PermissionRequest";
pub const CUE_SESSION_START: &str = "SessionStart";
pub const CUE_PROMPT: &str = "UserPromptSubmit";
pub const CUE_TURN_END: &str = "Stop";
pub const CUE_TURN_FAILED: &str = "StopFailure";
pub const CUE_LAUNCH: &str = "AppLaunch";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStyle {
    Expand,
    Glance,
    Off,
}

impl Default for CompletionStyle {
    fn default() -> Self {
        Self::Expand
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPresentation {
    StatusOnly,
    Glance,
    Card,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PushKind {
    Error,
    Completion,
    Approval,
    Question,
    DisplayOnlyWait,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpIntent {
    TrackCompletion { turn_failed: bool },
    RestartWait,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CompletionIntent {
    pub turn_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PushIntent {
    pub kind: PushKind,
    pub smart_suppressible: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AttentionIntents {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<PushIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<FollowUpIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CompletionDecision {
    pub turn_failed: bool,
    pub presentation: CompletionPresentation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PushDecision {
    pub kind: PushKind,
    pub smart_suppressed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AttentionDecision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<PushDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<FollowUpIntent>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AttentionSubject {
    pub is_child: bool,
    pub interrupted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttentionContext {
    pub sounds_enabled: bool,
    pub audible_cues: HashSet<String>,
    pub quiet_hours: bool,
    pub mute_when_away: bool,
    pub away: bool,
    pub completion_style: CompletionStyle,
    pub push_enabled: bool,
    pub session_in_front: bool,
}

impl Default for AttentionContext {
    fn default() -> Self {
        Self {
            sounds_enabled: true,
            audible_cues: HashSet::new(),
            quiet_hours: false,
            mute_when_away: false,
            away: false,
            completion_style: CompletionStyle::Expand,
            push_enabled: false,
            session_in_front: false,
        }
    }
}

impl AttentionContext {
    pub fn holds_sounds(&self) -> bool {
        codeisland_swift::call(c"attention.hold", self)
    }
}

/// True when no non-hidden request of this kind is already queued. The first
/// approval (or question) of a burst rings; the ones behind it stay silent.
pub fn first_of_burst(pending: &IndexMap<String, PendingRequest>, kind: RequestKind) -> bool {
    !pending
        .values()
        .any(|request| !request.hidden && request.kind == kind)
}

fn event_arguments(
    event: &AgentEvent,
    outcome: &ApplyOutcome,
    subject: AttentionSubject,
    first: bool,
) -> serde_json::Value {
    use AgentEventType::*;
    let kind = event.event_type;
    let raw = event.raw_event_name.as_deref();
    let moment = match kind {
        SessionEnd => "silent",
        SubagentStart | SubagentEnd if event.parent_session_id.is_none() => "silent",
        _ if !outcome.applied => "silent",
        _ if event.is_interrupted_turn() => "stopped",
        PermissionRequest => "approval",
        Question if raw == Some("CodexAppServerQuestion") => "appServerQuestion",
        Question => "question",
        Interrupt => "interrupted",
        TurnEnd if raw.is_none() => "finished",
        TurnFailed | Error if raw.is_none() => "failed",
        _ => "hook",
    };
    // Events synthesized by the Windows watchers have no wire name.
    let name = raw.unwrap_or(match kind {
        SessionStart => "SessionStart",
        SubagentStart => "SubagentStart",
        SubagentEnd => "SubagentStop",
        UserPrompt => "UserPromptSubmit",
        ToolStart => "PreToolUse",
        ToolEnd => "PostToolUse",
        TurnEnd => "Stop",
        TurnFailed | Error => "StopFailure",
        Notification => "Notification",
        _ => "",
    });
    serde_json::json!({
        "moment": moment, "event": name, "source": event.agent, "payload": event.payload,
        "first": first, "applied": outcome.applied, "ended": outcome.ended_turn,
        "child": subject.is_child, "interrupted": subject.interrupted, "agent_id": event.agent_id,
    })
}

pub fn sound_cue(event: &AgentEvent) -> Option<String> {
    let outcome = ApplyOutcome { applied: true, session_id: event.session_id.clone(),
        is_new_session: false, ended_turn: false, turn_failed: false, request: None };
    intents_for_event(event, &outcome, AttentionSubject::default()).sound
}

pub fn intents_for_event(event: &AgentEvent, outcome: &ApplyOutcome, subject: AttentionSubject) -> AttentionIntents {
    codeisland_swift::call(c"attention.intents", event_arguments(event, outcome, subject, true))
}

pub fn settle(intents: AttentionIntents, context: &AttentionContext) -> AttentionDecision {
    codeisland_swift::call(c"attention.settle", serde_json::json!({"intents": intents, "context": context}))
}

pub fn decide(event: &AgentEvent, outcome: &ApplyOutcome, subject: AttentionSubject, context: &AttentionContext) -> AttentionDecision {
    decide_for_burst(event, outcome, subject, context, true)
}

pub fn decide_for_burst(event: &AgentEvent, outcome: &ApplyOutcome, subject: AttentionSubject, context: &AttentionContext, first: bool) -> AttentionDecision {
    let mut args = event_arguments(event, outcome, subject, first);
    args["context"] = serde_json::json!(context);
    codeisland_swift::call(c"attention.decide", args)
}

/// Parses "HH:MM" and reports whether `now` falls inside a quiet-hours window.
pub fn in_quiet_hours(now: chrono::DateTime<chrono::Local>, start: &str, end: &str) -> bool {
    let Some(start) = parse_hhmm(start) else {
        return false;
    };
    let Some(end) = parse_hhmm(end) else {
        return false;
    };
    let minutes = now.time().hour() * 60 + now.time().minute();
    codeisland_swift::call(c"sound.quiet", serde_json::json!({ "minute": minutes, "start": start, "end": end }))
}

fn parse_hhmm(value: &str) -> Option<u32> {
    let (hour, minute) = value.trim().split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour * 60 + minute)
}
