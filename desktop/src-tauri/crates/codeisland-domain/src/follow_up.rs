//! Follow-up timing. Callers provide the clock and decide how to deliver reminders.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Approval,
    Question,
    Completion,
}

impl Kind {
    pub fn max_attempts(self) -> u8 {
        match self {
            Self::Approval | Self::Question => 3,
            Self::Completion => 1,
        }
    }

    pub fn sound(self, turn_failed: bool) -> &'static str {
        match self {
            Self::Approval | Self::Question => "approval",
            Self::Completion if turn_failed => "error",
            Self::Completion => "completion",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Island,
    DisplayOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    OnTime,
    Deferred,
    CatchUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub key: Key,
    pub waiting_since: DateTime<Utc>,
    pub attempt: u8,
    pub origin: Origin,
    pub request_id: Option<String>,
    pub delivery: Delivery,
    pub turn_failed: bool,
}

struct Entry {
    waiting_since: DateTime<Utc>,
    anchor: DateTime<Utc>,
    delivered: u8,
    owed: bool,
    done: bool,
    origin: Origin,
    request_id: Option<String>,
    turn_failed: bool,
}

impl Entry {
    fn new(now: DateTime<Utc>, origin: Origin, request_id: Option<String>) -> Self {
        Self {
            waiting_since: now,
            anchor: now,
            delivered: 0,
            owed: false,
            done: false,
            origin,
            request_id,
            turn_failed: false,
        }
    }
}

#[derive(Default)]
pub struct Scheduler {
    interval: Option<TimeDelta>,
    entries: HashMap<Key, Entry>,
}

impl Scheduler {
    pub fn set_interval(&mut self, interval: Option<TimeDelta>) {
        self.interval = interval.filter(|duration| *duration > TimeDelta::zero());
        if self.interval.is_none() {
            self.entries.clear();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.interval.is_some()
    }

    pub fn sync(
        &mut self,
        kind: Kind,
        origin: Origin,
        waiting: HashMap<String, Option<String>>,
        now: DateTime<Utc>,
    ) {
        if !self.is_enabled() {
            return;
        }
        self.entries.retain(|key, entry| {
            key.kind != kind || entry.origin != origin || waiting.contains_key(&key.session_id)
        });
        for (session_id, request_id) in waiting {
            let key = Key::new(kind, session_id);
            if let Some(entry) = self.entries.get_mut(&key)
                && entry.request_id.as_ref().zip(request_id.as_ref()).is_none_or(|(a, b)| a == b)
            {
                entry.origin = origin;
                if request_id.is_some() {
                    entry.request_id = request_id;
                }
            } else {
                self.entries.insert(key, Entry::new(now, origin, request_id));
            }
        }
    }

    pub fn restart_display_only(&mut self, kind: Kind, session_id: &str, now: DateTime<Utc>) {
        if self.is_enabled() {
            self.entries.insert(Key::new(kind, session_id), Entry::new(now, Origin::DisplayOnly, None));
        }
    }

    pub fn track_completion(&mut self, session_id: &str, now: DateTime<Utc>, turn_failed: bool) {
        if self.is_enabled() {
            let mut entry = Entry::new(now, Origin::Island, None);
            entry.turn_failed = turn_failed;
            self.entries.insert(Key::new(Kind::Completion, session_id), entry);
        }
    }

    pub fn silence_session(&mut self, session_id: &str) {
        self.entries.retain(|key, entry| {
            if key.session_id != session_id {
                return true;
            }
            entry.done = true;
            entry.owed = false;
            entry.delivered = 0;
            key.kind != Kind::Completion
        });
    }

    pub fn silence_completion(&mut self, session_id: &str) {
        self.entries.remove(&Key::new(Kind::Completion, session_id));
    }

    pub fn retain_completions(&mut self, retain: impl Fn(&str) -> bool) {
        self.entries
            .retain(|key, _| key.kind != Kind::Completion || retain(&key.session_id));
    }

    pub fn has_owed(&self) -> bool {
        self.entries.values().any(|entry| entry.owed && !entry.done)
    }

    pub fn next_wake_date(&self) -> Option<DateTime<Utc>> {
        let interval = self.interval?;
        self.entries
            .values()
            .filter(|entry| !entry.done && !entry.owed)
            .map(|entry| entry.anchor + interval)
            .min()
    }

    pub fn is_current(&self, reminder: &Reminder) -> bool {
        self.entries.get(&reminder.key).is_some_and(|entry| {
            entry.waiting_since == reminder.waiting_since
                && entry.request_id == reminder.request_id
                && if reminder.delivery == Delivery::Deferred {
                    entry.owed && entry.delivered + 1 == reminder.attempt
                } else {
                    entry.delivered == reminder.attempt
                }
        })
    }

    /// A suppressed reminder that reached nobody should come due again.
    pub fn postpone(&mut self, reminder: &Reminder, now: DateTime<Utc>) {
        if let Some(entry) = self.entries.get_mut(&reminder.key)
            && entry.waiting_since == reminder.waiting_since
            && entry.request_id == reminder.request_id
            && entry.delivered == reminder.attempt
        {
            entry.delivered -= 1;
            entry.anchor = now;
            entry.owed = false;
            entry.done = false;
        }
    }

    pub fn collect_due(&mut self, now: DateTime<Utc>, held: bool) -> Vec<Reminder> {
        let Some(interval) = self.interval else {
            return Vec::new();
        };
        // Preserve a final completion until the next tick so it can be postponed.
        self.entries.retain(|key, entry| key.kind != Kind::Completion || !entry.done);
        let mut reminders = Vec::new();
        for (key, entry) in &mut self.entries {
            if entry.done || (held && entry.owed) || (!entry.owed && now < entry.anchor + interval) {
                continue;
            }
            let attempt = entry.delivered + 1;
            let delivery = if held {
                Delivery::Deferred
            } else if entry.owed {
                Delivery::CatchUp
            } else {
                Delivery::OnTime
            };
            entry.owed = held;
            if !held {
                entry.delivered = attempt;
                entry.done = attempt >= key.kind.max_attempts();
                entry.anchor = now;
            }
            reminders.push(Reminder {
                key: key.clone(),
                waiting_since: entry.waiting_since,
                attempt,
                origin: entry.origin,
                request_id: entry.request_id.clone(),
                delivery,
                turn_failed: entry.turn_failed,
            });
        }
        reminders.sort_by(|a, b| {
            a.key.kind.cmp(&b.key.kind)
                .then(a.waiting_since.cmp(&b.waiting_since))
                .then(a.key.session_id.cmp(&b.key.session_id))
        });
        reminders
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
