//! CodeIsland runtime: session store orchestration, attention decisions, persistence,
//! transcript tailing and IPC glue.

#[cfg(any(windows, test))]
mod codex_app_server;
#[cfg(any(windows, test))]
mod codex_host;
#[cfg(any(windows, test))]
mod cowork_host;
#[cfg(any(windows, test))]
mod cowork_watcher;
mod follow_up;
#[cfg(any(windows, test))]
mod provider_discovery;
mod hook_policy;
mod ipc_handler;
mod island;
pub mod persistence;
pub mod process;
mod push_format;
mod runtime;
mod tailer;
#[cfg(any(windows, test))]
mod wsl;
#[cfg(any(windows, test))]
mod wsl_process;

#[cfg(windows)]
pub use codex_host::is_codex_desktop_process;
#[cfg(windows)]
pub use cowork_host::is_claude_desktop_process;
#[cfg(windows)]
pub use wsl_process::{wsl_process_is_current, wsl_session_is_current};
pub use island::{CardShortcut, CardShortcutOutcome, IslandSurface};
pub use ipc_handler::remote_hook_handler;
pub use runtime::{Runtime, RuntimeHandle, RuntimeUpdate, UiState};
