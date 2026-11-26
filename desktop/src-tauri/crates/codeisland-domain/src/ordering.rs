use std::collections::HashMap;

use indexmap::IndexMap;

use crate::types::{PendingRequest, RequestKind, SessionSnapshot, SessionStatus};

pub fn status_tier(status: SessionStatus) -> u8 {
    if status.needs_you() {
        0
    } else if status.is_active() {
        1
    } else {
        2
    }
}

/// Request queue position per session: approvals before questions, each in arrival order.
pub fn request_rank(pending: &IndexMap<String, PendingRequest>) -> HashMap<String, usize> {
    let mut ranks = HashMap::new();
    let approvals = pending
        .values()
        .filter(|request| request.kind == RequestKind::Permission);
    let questions = pending
        .values()
        .filter(|request| request.kind == RequestKind::Question);
    for (index, request) in approvals.chain(questions).enumerate() {
        ranks.entry(request.session_id.clone()).or_insert(index);
    }
    ranks
}

/// Needs-you first (request queue order), then working (newest session first),
/// then the rest by most recent activity.
pub fn order_sessions(
    sessions: &IndexMap<String, SessionSnapshot>,
    pending: &IndexMap<String, PendingRequest>,
) -> Vec<String> {
    let ranks = request_rank(pending);
    let mut ids: Vec<&String> = sessions.keys().collect();
    ids.sort_by(|a, b| {
        let sa = &sessions[*a];
        let sb = &sessions[*b];
        let ta = status_tier(sa.status);
        let tb = status_tier(sb.status);
        if ta != tb {
            return ta.cmp(&tb);
        }
        match ta {
            0 => {
                let ra = ranks.get(*a).copied().unwrap_or(usize::MAX);
                let rb = ranks.get(*b).copied().unwrap_or(usize::MAX);
                if ra != rb {
                    return ra.cmp(&rb);
                }
                sb.last_activity
                    .cmp(&sa.last_activity)
                    .then_with(|| a.cmp(b))
            }
            1 => sb
                .started_at
                .cmp(&sa.started_at)
                .then_with(|| a.cmp(b)),
            _ => sb
                .last_activity
                .cmp(&sa.last_activity)
                .then_with(|| a.cmp(b)),
        }
    });
    ids.into_iter().cloned().collect()
}
