//! Adapt per-user Windows settings to the original HookServer's routing policy.
use codeisland_domain::{AgentEvent, AgentEventType, AppSettings};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Policy { Exclude, Approve, Pass }

pub(crate) fn evaluate(settings: &AppSettings, source: &str, event: &AgentEvent) -> Policy {
    let name = if event.event_type == AgentEventType::PermissionRequest {
        "PermissionRequest"
    } else {
        // Compatibility events already carry their route in the v1 type.
        "Event"
    };
    let approved: Vec<_> = settings.auto_approve_sources.split(',')
        .map(|s| s.trim().trim_start_matches("codeisland-")).collect();
    codeisland_swift::call(c"hook.policy", json!({
        "event": name, "source": source.trim_start_matches("codeisland-"),
        "tool": event.tool, "cwd": event.cwd, "excluded": settings.excluded_hook_cwd_substrings,
        "tools": settings.auto_approve_tools, "approved": approved.join(","), "custom": settings.custom_clis,
    }))
}
