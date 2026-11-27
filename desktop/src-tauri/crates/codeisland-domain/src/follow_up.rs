//! Follow-up timing. Callers provide the clock and decide how to deliver reminders.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Serialize, Deserialize};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Approval,
    Question,
    Completion,
}

impl Kind {
    pub fn sound(self, turn_failed: bool) -> &'static str {
        match self {
            Self::Approval | Self::Question => "approval",
            Self::Completion if turn_failed => "error",
            Self::Completion => "completion",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Island,
    DisplayOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Delivery {
    OnTime,
    Deferred,
    CatchUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Key {
    pub kind: Kind,
    pub session_id: String,
}

impl Key {
    pub fn new(kind: Kind, session_id: impl Into<String>) -> Self {
        Self {
            kind,
            session_id: session_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub key: Key,
    #[serde(with = "chrono::serde::ts_microseconds")]
    pub waiting_since: DateTime<Utc>,
    pub attempt: u8,
    pub origin: Origin,
    pub request_id: Option<String>,
    pub delivery: Delivery,
    pub turn_failed: bool,
}

#[derive(Default)]
pub struct Scheduler(codeisland_swift::Context);

impl Scheduler {
    pub fn set_interval(&mut self, interval: Option<TimeDelta>) {
        self.0.call(c"reminder.interval", interval.map(|interval| interval.as_seconds_f64()))
    }

    pub fn is_enabled(&self) -> bool { self.0.call(c"reminder.enabled", ()) }
    pub fn has_owed(&self) -> bool { self.0.call(c"reminder.owed", ()) }

    pub fn sync(&mut self, kind: Kind, origin: Origin, waiting: HashMap<String, Option<String>>, now: DateTime<Utc>) {
        self.0.call(c"reminder.sync", json!({ "kind": kind, "origin": origin, "waiting": waiting, "now": now.timestamp_micros() }))
    }

    pub fn restart_display_only(&mut self, kind: Kind, session_id: &str, now: DateTime<Utc>) {
        self.0.call(c"reminder.restart", json!({ "kind": kind, "id": session_id, "now": now.timestamp_micros(), "failed": false }))
    }

    pub fn track_completion(&mut self, session_id: &str, now: DateTime<Utc>, turn_failed: bool) {
        self.0.call(c"reminder.track", json!({ "kind": Kind::Completion, "id": session_id, "now": now.timestamp_micros(), "failed": turn_failed }))
    }

    pub fn silence_session(&mut self, session_id: &str) { self.0.call(c"reminder.silenceSession", session_id) }
    pub fn silence_completion(&mut self, session_id: &str) { self.0.call(c"reminder.silenceCompletion", session_id) }

    pub fn retain_completions(&mut self, ids: &[&str]) {
        self.0.call(c"reminder.keepCompletions", ids)
    }

    pub fn next_wake_date(&self) -> Option<DateTime<Utc>> {
        self.0.call::<Option<i64>>(c"reminder.next", ()).and_then(DateTime::from_timestamp_micros)
    }

    pub fn is_current(&self, reminder: &Reminder) -> bool { self.0.call(c"reminder.current", reminder) }

    pub fn postpone(&mut self, reminder: &Reminder, now: DateTime<Utc>) {
        self.0.call(c"reminder.postpone", json!({ "item": reminder, "now": now.timestamp_micros() }))
    }

    pub fn collect_due(&mut self, now: DateTime<Utc>, held: bool) -> Vec<Reminder> {
        self.0.call(c"reminder.collect", json!({ "now": now.timestamp_micros(), "held": held }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minute() -> TimeDelta {
        TimeDelta::minutes(1)
    }

    fn request(id: &str) -> HashMap<String, Option<String>> {
        HashMap::from([("session".into(), Some(id.into()))])
    }

    #[test]
    fn spends_three_attempts_and_restarts_only_for_a_replacement() {
        let start = DateTime::UNIX_EPOCH;
        let mut scheduler = Scheduler::default();
        scheduler.set_interval(Some(minute()));
        scheduler.sync(Kind::Approval, Origin::Island, request("first"), start);
        for attempt in 1..=3 {
            let now = start + minute() * attempt;
            assert_eq!(scheduler.collect_due(now, false)[0].attempt, attempt as u8);
            scheduler.sync(Kind::Approval, Origin::Island, request("first"), now);
        }
        assert_eq!(scheduler.next_wake_date(), None);
        let changed = start + minute() * 4;
        scheduler.sync(Kind::Approval, Origin::Island, request("second"), changed);
        assert!(scheduler.collect_due(changed, false).is_empty());
        let next = scheduler.collect_due(changed + minute(), false);
        assert_eq!(next[0].attempt, 1);
        assert_eq!(next[0].request_id.as_deref(), Some("second"));
    }

    #[test]
    fn holds_one_attempt_and_catches_up_in_urgency_order() {
        let start = DateTime::UNIX_EPOCH;
        let mut scheduler = Scheduler::default();
        scheduler.set_interval(Some(minute()));
        scheduler.track_completion("session", start, true);
        scheduler.sync(Kind::Question, Origin::Island, request("question"), start);
        scheduler.sync(Kind::Approval, Origin::Island, request("approval"), start);
        let deferred = scheduler.collect_due(start + minute(), true);
        assert_eq!(deferred.iter().map(|r| r.key.kind).collect::<Vec<_>>(),
            [Kind::Approval, Kind::Question, Kind::Completion]);
        assert!(deferred.iter().all(|r| r.delivery == Delivery::Deferred));
        assert!(scheduler.collect_due(start + minute() * 4, true).is_empty());
        let catch_up = scheduler.collect_due(start + minute() * 4, false);
        assert!(catch_up.iter().all(|r| r.delivery == Delivery::CatchUp && r.attempt == 1));
        assert!(catch_up[2].turn_failed);
        assert_eq!(scheduler.next_wake_date(), Some(start + minute() * 5));
    }

    #[test]
    fn postpones_unheard_attempt_and_silences_seen_or_removed_items() {
        let start = DateTime::UNIX_EPOCH;
        let mut scheduler = Scheduler::default();
        scheduler.set_interval(Some(minute()));
        scheduler.track_completion("session", start, false);
        let due = scheduler.collect_due(start + minute(), false).remove(0);
        scheduler.postpone(&due, start + minute());
        assert_eq!(scheduler.next_wake_date(), Some(start + minute() * 2));
        scheduler.silence_completion("session");
        scheduler.sync(Kind::Approval, Origin::Island, request("first"), start);
        scheduler.silence_session("session");
        scheduler.sync(Kind::Approval, Origin::Island, request("first"), start);
        assert!(scheduler.collect_due(start + minute() * 5, false).is_empty());
        scheduler.sync(Kind::Approval, Origin::Island, HashMap::new(), start);
        scheduler.sync(Kind::Approval, Origin::Island, request("first"), start);
        assert_eq!(scheduler.collect_due(start + minute(), false).len(), 1);
        scheduler.set_interval(None);
        assert_eq!(scheduler.next_wake_date(), None);
    }

    #[test]
    fn changing_interval_retimes_and_display_only_wait_adopts_attempts() {
        let start = DateTime::UNIX_EPOCH;
        let mut scheduler = Scheduler::default();
        scheduler.set_interval(Some(minute()));
        scheduler.sync(Kind::Question, Origin::DisplayOnly,
            HashMap::from([("session".into(), None)]), start);
        assert_eq!(scheduler.collect_due(start + minute(), false)[0].attempt, 1);
        scheduler.sync(Kind::Question, Origin::Island, request("question"), start + minute());
        scheduler.sync(Kind::Question, Origin::DisplayOnly, HashMap::new(), start + minute());
        scheduler.set_interval(Some(minute() * 3));
        assert_eq!(scheduler.next_wake_date(), Some(start + minute() * 4));
        let due = scheduler.collect_due(start + minute() * 4, false);
        assert_eq!(due[0].attempt, 2);
        assert_eq!(due[0].origin, Origin::Island);
    }
}
