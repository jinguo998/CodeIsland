pub mod approval;
pub mod attention;
pub mod claude_desktop_code;
pub mod cowork;
pub mod discovery;
pub mod follow_up;
mod hook_tool;
pub mod ordering;
pub mod push_text;
pub mod session_metadata;
pub mod settings;
pub mod store;
pub mod types;

pub use approval::ApprovalAlwaysScope;
pub use attention::{
    AttentionContext, AttentionDecision, AttentionIntents, AttentionSubject, CompletionDecision,
    CompletionIntent, CompletionPresentation, CompletionStyle, FollowUpIntent, PushDecision,
    PushIntent, PushKind, decide, first_of_burst, in_quiet_hours, intents_for_event, settle,
};
pub use ordering::{order_sessions, request_rank, status_tier};
pub use settings::{
    PushChannelSetting,
    AppSettings, CustomCliSetting, RemoteHostSetting, normalize_shortcut, parse_shortcut,
};
pub use store::SessionStore;
pub use types::*;

pub const CODEX_DESKTOP_SESSION_PREFIX: &str = "codexapp:";

pub fn codex_desktop_session_id(thread_id: &str) -> String {
    if thread_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX) {
        thread_id.to_owned()
    } else {
        format!("{CODEX_DESKTOP_SESSION_PREFIX}{thread_id}")
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use ts_rs::TS;

    #[test]
    fn export_typescript_bindings() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../src/lib/bindings");
        std::fs::create_dir_all(&dir).expect("create bindings dir");
        let cfg = ts_rs::Config::default()
            .with_out_dir(dir)
            .with_large_int("number");
        macro_rules! export {
            ($($ty:ty),* $(,)?) => {
                $(<$ty>::export(&cfg).expect("export type");)*
            };
        }
        export!(
            AgentKind,
            AgentEventType,
            AgentEvent,
            SessionStatus,
            TaskStatus,
            AgentTask,
            HistoryEntry,
            SessionSnapshot,
            RequestKind,
            QuestionItem,
            PendingRequest,
            ApprovalAlwaysScope,
            UserAction,
            Decision,
            StoreSnapshot,
            ApplyOutcome,
            AppSettings,
            PushChannelSetting,
            RemoteHostSetting,
            CustomCliSetting,
            CompletionStyle,
            CompletionPresentation,
            PushKind,
            FollowUpIntent,
            CompletionIntent,
            PushIntent,
            AttentionIntents,
            CompletionDecision,
            PushDecision,
            AttentionDecision,
        );
    }
}
