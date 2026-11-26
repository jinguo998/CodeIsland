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

#[derive(Debug, Clone)]
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
        self.quiet_hours || (self.mute_when_away && self.away)
    }
}

/// True when no non-hidden request of this kind is already queued. The first
/// approval (or question) of a burst rings; the ones behind it stay silent.
pub fn first_of_burst(pending: &IndexMap<String, PendingRequest>, kind: RequestKind) -> bool {
    !pending
        .values()
        .any(|request| !request.hidden && request.kind == kind)
}

pub fn sound_cue(event: &AgentEvent) -> Option<String> {
    // A turn the user stopped makes no noise.
    if event.is_interrupted_turn() {
        return None;
    }
    match event.event_type {
        AgentEventType::SessionStart | AgentEventType::SubagentStart => {
            Some(CUE_SESSION_START.to_owned())
        }
        AgentEventType::UserPrompt => Some(CUE_PROMPT.to_owned()),
        AgentEventType::PermissionRequest | AgentEventType::Question => {
            Some(CUE_REQUEST.to_owned())
        }
        AgentEventType::TurnEnd | AgentEventType::SubagentEnd => {
            Some(CUE_TURN_END.to_owned())
        }
        AgentEventType::TurnFailed | AgentEventType::Error => {
            Some(CUE_TURN_FAILED.to_owned())
        }
        AgentEventType::Interrupt | AgentEventType::SessionEnd => None,
        _ => None,
    }
}

/// What the moment calls for on its own, before settings and the room.
pub fn intents_for_event(
    event: &AgentEvent,
    outcome: &ApplyOutcome,
    subject: AttentionSubject,
) -> AttentionIntents {
    let mut intents = AttentionIntents::default();

    // A CLI exiting is not a moment: the store has already removed its card
    // and queued requests, so nothing should ring, pop or push.
    if event.event_type == AgentEventType::SessionEnd {
        return intents;
    }

    // Subagent events without a parent are heartbeats; they never mint a card.
    if matches!(
        event.event_type,
        AgentEventType::SubagentStart | AgentEventType::SubagentEnd
    ) && event.parent_session_id.is_none()
    {
        return intents;
    }

    // A turn the user stopped is not news: no sound, push, completion or reminder.
    if event.is_interrupted_turn() {
        return intents;
    }

    // A declared child never alerts; its parent card carries the news.
    if subject.is_child {
        return intents;
    }

    match event.event_type {
        AgentEventType::PermissionRequest => {
            intents.sound = Some(CUE_REQUEST.to_owned());
            intents.push = Some(PushIntent {
                kind: PushKind::Approval,
                smart_suppressible: false,
            });
            return intents;
        }
        AgentEventType::Question => {
            intents.sound = Some(CUE_REQUEST.to_owned());
            intents.push = Some(PushIntent {
                kind: PushKind::Question,
                smart_suppressible: true,
            });
            return intents;
        }
        _ => {}
    }

    if outcome.applied {
        intents.sound = sound_cue(event);
    }

    match event.event_type {
        AgentEventType::TurnEnd | AgentEventType::SubagentEnd => {
            intents.completion = Some(CompletionIntent { turn_failed: false });
            intents.push = Some(PushIntent {
                kind: PushKind::Completion,
                smart_suppressible: true,
            });
            if !subject.interrupted {
                intents.follow_up = Some(FollowUpIntent::TrackCompletion { turn_failed: false });
            }
        }
        AgentEventType::TurnFailed | AgentEventType::Error => {
            intents.completion = Some(CompletionIntent { turn_failed: true });
            intents.push = Some(PushIntent {
                kind: PushKind::Error,
                smart_suppressible: true,
            });
            if !subject.interrupted {
                intents.follow_up = Some(FollowUpIntent::TrackCompletion { turn_failed: true });
            }
        }
        AgentEventType::Interrupt => {
            // The card pops, but a turn the user stopped is not news: no sound, push or reminder.
            intents.sound = None;
            intents.completion = Some(CompletionIntent { turn_failed: false });
        }
        _ => {}
    }
    intents
}

/// What actually happens, given the user's settings and the room.
pub fn settle(intents: AttentionIntents, context: &AttentionContext) -> AttentionDecision {
    let mut decision = AttentionDecision::default();

    if let Some(cue) = intents.sound {
        if context.sounds_enabled
            && context.audible_cues.contains(&cue)
            && !context.holds_sounds()
        {
            decision.sound = Some(cue);
        }
    }

    if let Some(completion) = intents.completion {
        let presentation = match context.completion_style {
            CompletionStyle::Off => CompletionPresentation::StatusOnly,
            CompletionStyle::Glance => CompletionPresentation::Glance,
            CompletionStyle::Expand => CompletionPresentation::Card,
        };
        decision.completion = Some(CompletionDecision {
            turn_failed: completion.turn_failed,
            presentation,
        });
    }

    if let Some(push) = intents.push {
        if context.push_enabled {
            decision.push = Some(PushDecision {
                kind: push.kind,
                smart_suppressed: push.smart_suppressible && context.session_in_front,
            });
        }
    }

    decision.follow_up = match intents.follow_up {
        Some(FollowUpIntent::TrackCompletion { .. })
            if context.completion_style == CompletionStyle::Off =>
        {
            None
        }
        other => other,
    };

    decision
}

pub fn decide(
    event: &AgentEvent,
    outcome: &ApplyOutcome,
    subject: AttentionSubject,
    context: &AttentionContext,
) -> AttentionDecision {
    settle(intents_for_event(event, outcome, subject), context)
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
    if start == end {
        return false;
    }
    if start < end {
        minutes >= start && minutes < end
    } else {
        minutes >= start || minutes < end
    }
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
