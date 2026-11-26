//! Agent integration layer.
//!
//! This crate owns three concerns that all other CodeIsland crates share:
//!
//! * static agent definitions ([`definitions`]),
//! * hook payload normalization ([`normalize`]) and response rendering
//!   ([`render`]),
//! * hook installation, detection and repair ([`install`], [`paths`],
//!   [`doctor`]).

pub mod codex_rules;
pub mod definitions;
pub mod doctor;
pub mod install;
pub mod normalize;
pub mod paths;
pub mod plugins;
pub mod render;

pub use definitions::{
    AgentDefinition, HookEventDef, HookFormat, ResponseFormat, custom_format_supported,
    definition, definition_for_agent_in, definition_for_source, definitions,
    definitions_for_agent,
};
pub use doctor::{ToolStatus, detect_all};
pub use install::{
    custom_status, detect, detect_at, hook_command, install, install_at, install_custom,
    merge_claude_json, merge_copilot_json, merge_cursor_json, merge_event_map_json,
    merge_generic_json, merge_kimi_toml, merge_yaml_block, remove_claude_json,
    remove_copilot_json, remove_cursor_json, remove_event_map_json, remove_generic_json,
    remove_kimi_toml, remove_yaml_block_text, repair, uninstall, uninstall_at, uninstall_custom,
};
pub use normalize::{normalize, normalize_with_ppid};
pub use render::render_response;

use thiserror::Error;

/// Errors produced by the agent layer.
///
/// `normalize` deliberately only fails with [`AgentError::InvalidJson`] so a
/// malformed hook payload can never take an agent down; the installation
/// paths use the remaining variants for actionable diagnostics.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("invalid hook payload: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("invalid config: {0}")]
    Config(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("agent {0} is not supported")]
    Unsupported(String),
    #[error("manual install required: {0}")]
    ManualInstall(String),
}
