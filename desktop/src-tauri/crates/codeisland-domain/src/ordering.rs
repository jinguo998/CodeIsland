use std::collections::HashMap;

use indexmap::IndexMap;
use serde::Serialize;

use crate::types::{PendingRequest, RequestKind, SessionSnapshot, SessionStatus};

pub fn status_tier(status: SessionStatus) -> u8 {
    codeisland_swift::call(c"session.tier", status)
}

#[derive(Serialize)]
struct Queues<'a> {
    permissions: Vec<&'a str>,
    questions: Vec<&'a str>,
}

fn queues(pending: &IndexMap<String, PendingRequest>) -> Queues<'_> {
    let mut result = Queues { permissions: Vec::new(), questions: Vec::new() };
    for request in pending.values() {
        match request.kind {
            RequestKind::Permission => result.permissions.push(&request.session_id),
            RequestKind::Question => result.questions.push(&request.session_id),
        }
    }
    result
}

pub fn request_rank(pending: &IndexMap<String, PendingRequest>) -> HashMap<String, usize> {
    codeisland_swift::call(c"session.rank", queues(pending))
}

fn list_arguments(sessions: &IndexMap<String, SessionSnapshot>, pending: &IndexMap<String, PendingRequest>) -> serde_json::Value {
    let cards: Vec<_> = sessions.iter().map(|(id, session)| serde_json::json!({
        "id": id, "status": session.status, "start": session.started_at.timestamp_micros(),
        "activity": session.last_activity.timestamp_micros(), "parent": session.parent_session_id,
    })).collect();
    serde_json::json!({ "cards": cards, "queues": queues(pending) })
}

pub fn order_sessions(sessions: &IndexMap<String, SessionSnapshot>, pending: &IndexMap<String, PendingRequest>) -> Vec<String> {
    codeisland_swift::call(c"session.order", list_arguments(sessions, pending))
}

pub(crate) fn idle_groups(core: &codeisland_swift::Context, sessions: &IndexMap<String, SessionSnapshot>, pending: &IndexMap<String, PendingRequest>) -> Vec<Vec<String>> {
    core.call(c"session.idleGroups", list_arguments(sessions, pending))
}
