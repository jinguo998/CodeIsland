//! Hook installation status for the Settings UI, CLI and diagnostics.

use codeisland_domain::AgentKind;
use serde::{Deserialize, Serialize};

use crate::definitions::definitions;
use crate::install;
use crate::paths::Paths;

/// One agent's hook integration status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStatus {
    pub agent: AgentKind,
    pub display_name: String,
    pub installed: bool,
    pub hook_installed: bool,
    pub healthy: bool,
    pub config_path: Option<String>,
    pub command: Option<String>,
    pub problem: Option<String>,
}

/// Status for every built-in agent, sorted by display name.
///
/// Several definitions can share one [`AgentKind`] (Pi/Oh My Pi,
/// AntiGravity/Google Antigravity, …). The Hooks UI keys rows by agent, so
/// each kind appears once; `definition(agent)` picks the product actually
/// present on this machine.
pub fn detect_all() -> Vec<ToolStatus> {
    detect_all_with(&Paths::current())
}

pub fn detect_all_with(paths: &Paths) -> Vec<ToolStatus> {
    let mut agents: Vec<AgentKind> = definitions()
        .iter()
        .map(|def| def.agent)
        .collect();
    agents.sort_by_key(|agent| agent.tag());
    agents.dedup();
    let mut statuses: Vec<ToolStatus> = agents
        .into_iter()
        .map(|agent| install::detect_with(paths, agent))
        .collect();
    statuses.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.agent.tag().cmp(right.agent.tag()))
    });
    statuses
}
