use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use codeisland_domain::{
    PendingRequest, RequestKind, SessionStatus, SessionStore,
    follow_up::{Key, Kind, Origin, Reminder, Scheduler},
};

use crate::island::{IslandSurface, visible_requests};

#[derive(Default)]
pub(crate) struct FollowUps {
    pub scheduler: Scheduler,
    hinted: HashMap<Key, Option<String>>,
    pub hint_pulse: u32,
}

impl FollowUps {
    pub fn sync(&mut self, store: &SessionStore, minutes: u32, now: DateTime<Utc>) {
        self.scheduler
            .set_interval(Some(TimeDelta::minutes(i64::from(minutes))));
        if !self.scheduler.is_enabled() {
            self.hinted.clear();
            return;
        }
        let statuses: HashMap<_, _> = store.sessions().iter().map(|(id, session)| (id, session.status)).collect();
        let queued: Vec<_> = store.pending().values().map(|request| &request.session_id).collect();
        let mut waits: HashMap<Kind, Vec<String>> = codeisland_swift::call(c"push.displayWaits",
            serde_json::json!({ "sessions": statuses, "queued": queued }));
        for (kind, request_kind) in [(Kind::Approval, RequestKind::Permission), (Kind::Question, RequestKind::Question)] {
            let mut held = HashMap::new();
            for request in visible_requests(store, request_kind) {
                held.entry(request.session_id.clone())
                    .or_insert_with(|| Some(request.event_id.clone()));
            }
            let display_only = waits.remove(&kind).unwrap_or_default().into_iter().map(|id| (id, None)).collect();
            self.scheduler.sync(kind, Origin::Island, held, now);
            self.scheduler.sync(kind, Origin::DisplayOnly, display_only, now);
        }
        let completions = store.sessions().keys().map(String::as_str).filter(|id| completed(store, id)).collect::<Vec<_>>();
        self.scheduler.retain_completions(&completions);
        self.hinted.retain(|key, request_id| still_pending(store, key, request_id.as_deref()));
    }

    pub fn presented(&mut self, surface: &IslandSurface) {
        match surface {
            IslandSurface::SessionList => {
                self.hinted.clear();
                self.scheduler.retain_completions(&[]);
            }
            IslandSurface::ApprovalCard { session_id, .. } => {
                self.hinted.remove(&Key::new(Kind::Approval, session_id));
            }
            IslandSurface::QuestionCard { session_id, .. } => {
                self.hinted.remove(&Key::new(Kind::Question, session_id));
            }
            IslandSurface::CompletionCard { .. } | IslandSurface::Collapsed => {}
        }
    }

    #[cfg(any(windows, test))]
    pub fn restart_display_only(&mut self, kind: Kind, session_id: &str, now: DateTime<Utc>) {
        self.hinted.remove(&Key::new(kind, session_id));
        self.scheduler.restart_display_only(kind, session_id, now);
    }

    pub fn completion_seen(&mut self, session_id: &str) {
        self.hinted.remove(&Key::new(Kind::Completion, session_id));
        self.scheduler.silence_completion(session_id);
    }

    pub fn user_jumped(&mut self, session_id: &str) {
        self.hinted.retain(|key, _| key.session_id != session_id);
        self.scheduler.silence_session(session_id);
    }

    pub fn hint(&mut self, reminders: impl Iterator<Item = Reminder>) {
        let mut hinted = false;
        for reminder in reminders {
            self.hinted.insert(reminder.key, reminder.request_id);
            hinted = true;
        }
        if hinted {
            self.hint_pulse = self.hint_pulse.wrapping_add(1);
        }
    }

    pub fn hint_active(&self) -> bool {
        !self.hinted.is_empty()
    }
}

pub(crate) fn reminder_request<'a>(store: &'a SessionStore, reminder: &Reminder) -> Option<&'a PendingRequest> {
    let request = store.get_pending(reminder.request_id.as_deref()?)?;
    (!request.hidden).then_some(request)
}

pub(crate) fn still_pending(store: &SessionStore, key: &Key, request_id: Option<&str>) -> bool {
    let (request_kind, status) = match key.kind {
        Kind::Approval => (RequestKind::Permission, SessionStatus::WaitingPermission),
        Kind::Question => (RequestKind::Question, SessionStatus::WaitingAnswer),
        Kind::Completion => return completed(store, &key.session_id),
    };
    if let Some(request_id) = request_id {
        return visible_requests(store, request_kind)
            .find(|request| request.session_id == key.session_id)
            .is_some_and(|request| request.event_id == request_id);
    }
    store.get(&key.session_id).is_some_and(|session| session.status == status)
        && !store.pending().values().any(|request| request.session_id == key.session_id)
}

fn completed(store: &SessionStore, id: &str) -> bool {
    store.get(id).is_some_and(|session| {
        matches!(session.status, SessionStatus::Completed | SessionStatus::Failed)
    })
}
