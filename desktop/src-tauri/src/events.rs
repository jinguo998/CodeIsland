use std::sync::Arc;

use codeisland_runtime::{Runtime, RuntimeUpdate};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

/// Forwards the runtime broadcast channel to the webview.
pub fn spawn_forwarder(app: &AppHandle, runtime: Arc<Runtime>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut receiver = runtime.subscribe();
        let mut window_state = None;
        let mut previous_settings = runtime.settings();
        loop {
            match receiver.recv().await {
                Ok(RuntimeUpdate::State(_) | RuntimeUpdate::AwayChanged) => {
                    // Re-read the aggregate so the webview always receives a
                    // complete `UiState`, not just the store snapshot.
                    emit_state(&app, runtime.as_ref(), &mut window_state);
                }
                Ok(RuntimeUpdate::Attention {
                    session_id,
                    decision,
                }) => {
                    let _ = app.emit(
                        "island://attention",
                        json!({ "sessionId": session_id, "decision": decision }),
                    );
                }
                Ok(RuntimeUpdate::Settings(settings)) => {
                    let refresh_tray = previous_settings.language != settings.language;
                    let refresh_shortcuts = crate::shortcuts::settings_changed(&previous_settings, &settings);
                    let reposition = previous_settings.display_choice != settings.display_choice
                        || previous_settings.display_mode != settings.display_mode
                        || previous_settings.pinned_display != settings.pinned_display
                        || previous_settings.panel_edge != settings.panel_edge
                        || previous_settings.allow_horizontal_drag != settings.allow_horizontal_drag;
                    previous_settings = settings.clone();
                    if !(refresh_tray || refresh_shortcuts || reposition) {
                        continue;
                    }
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if refresh_shortcuts {
                            let conflicts = crate::shortcuts::register_shortcuts(&handle, &settings);
                            if let Some(conflict) = conflicts.first() {
                                let _ = handle.emit_to("settings", "island://toast", json!({
                                    "kind": "error",
                                    "message": format!("{}: {}", conflict.shortcut, conflict.error),
                                }));
                            }
                        }
                        if reposition
                            && let Some(window) = handle.get_webview_window("main")
                            && let Err(error) = crate::window::restore_position(&window, &settings)
                        {
                            tracing::warn!(%error, "failed to apply display settings");
                        }
                        if refresh_tray {
                            if let Err(error) = crate::tray::update_language(&handle, &settings.language) {
                                tracing::warn!(%error, "failed to update tray language");
                            }
                        }
                    });
                }
                Ok(RuntimeUpdate::Toast { kind, message }) => {
                    let _ = app.emit(
                        "island://toast",
                        json!({ "kind": kind, "message": message }),
                    );
                }
                Ok(RuntimeUpdate::LocalClaudeStop) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Serializes to a JSON value first so the emitted payload does not depend on
/// `UiState` implementing `Clone`.
fn emit_state(
    app: &AppHandle,
    runtime: &Runtime,
    window_state: &mut Option<(bool, bool, bool, bool)>,
) {
    let state = runtime.state();
    let window_conditions = (
        state.surface.is_collapsed(),
        state.store.sessions.is_empty(),
        state.settings.hide_when_no_session && !runtime.has_active_sessions(),
        state.settings.hide_in_fullscreen,
    );
    if *window_state != Some(window_conditions) {
        *window_state = Some(window_conditions);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || crate::window::sync_island(&handle));
    }
    if let Ok(payload) = serde_json::to_value(state) {
        let _ = app.emit("island://state", payload);
    }
}
