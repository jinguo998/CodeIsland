use codeisland_domain::{PendingRequest, RequestKind, SessionStore};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One surface can own the island at a time. Request cards carry the exact
/// event as well as the session so a replacement request cannot inherit it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IslandSurface {
    #[default]
    Collapsed,
    SessionList,
    ApprovalCard { event_id: String, session_id: String },
    QuestionCard { event_id: String, session_id: String },
    CompletionCard { session_id: String },
}

impl IslandSurface {
    pub fn is_collapsed(&self) -> bool {
        matches!(self, Self::Collapsed)
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Collapsed | Self::SessionList => None,
            Self::ApprovalCard { session_id, .. }
            | Self::QuestionCard { session_id, .. }
            | Self::CompletionCard { session_id } => Some(session_id),
        }
    }

    pub fn for_request(request: &PendingRequest) -> Self {
        let event_id = request.event_id.clone();
        let session_id = request.session_id.clone();
        match request.kind {
            RequestKind::Permission => Self::ApprovalCard {
                event_id,
                session_id,
            },
            RequestKind::Question => Self::QuestionCard {
                event_id,
                session_id,
            },
        }
    }

    pub(crate) fn current_request<'a>(&self, store: &'a SessionStore) -> Option<&'a PendingRequest> {
        let (event_id, session_id, kind) = match self {
            Self::ApprovalCard {
                event_id,
                session_id,
            } => (event_id, session_id, RequestKind::Permission),
            Self::QuestionCard {
                event_id,
                session_id,
            } => (event_id, session_id, RequestKind::Question),
            _ => return None,
        };
        store.get_pending(event_id).filter(|request| {
            !request.hidden && request.kind == kind && request.session_id == *session_id
        })
    }

    pub(crate) fn is_available(&self, store: &SessionStore) -> bool {
        match self {
            Self::ApprovalCard { .. } | Self::QuestionCard { .. } => {
                self.current_request(store).is_some()
            }
            Self::CompletionCard { session_id } => store.get(session_id).is_some(),
            Self::Collapsed | Self::SessionList => true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CardShortcut {
    Approve,
    AlwaysAllow,
    Deny,
    SkipQuestion,
}

impl CardShortcut {
    pub(crate) fn request_kind(self) -> RequestKind {
        match self {
            Self::Approve | Self::AlwaysAllow | Self::Deny => RequestKind::Permission,
            Self::SkipQuestion => RequestKind::Question,
        }
    }

    pub(crate) fn native_name(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::AlwaysAllow => "approveAlways",
            Self::Deny => "deny",
            Self::SkipQuestion => "skipQuestion",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CardShortcutOutcome {
    Acted,
    Opened(IslandSurface),
    Ignored,
}

/// Each session's head request is its card. A hidden head keeps any later
/// requests for the same session behind it until the head is handled.
pub(crate) fn visible_requests(
    store: &SessionStore,
    kind: RequestKind,
) -> impl Iterator<Item = &PendingRequest> {
    let mut seen = HashSet::new();
    store.pending().values().filter(move |request| {
        request.kind == kind && seen.insert(request.session_id.as_str()) && !request.hidden
    })
}
