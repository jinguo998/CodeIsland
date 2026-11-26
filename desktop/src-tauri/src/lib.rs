mod apple_companion;
#[cfg(windows)]
mod away;
mod claude_quota;
mod commands;
mod diagnostics;
mod events;
#[cfg(windows)]
mod hover;
mod remote_connections;
#[cfg(windows)]
mod ssh_signals;
mod shortcuts;
mod sound_files;
mod terminal;
mod tray;
#[cfg(windows)]
mod wezterm;
#[cfg(windows)]
mod windows_terminal;
mod window;
mod workspace;
#[cfg(windows)]
mod wsl_console;

use std::sync::{Arc, Mutex, atomic::AtomicBool};

use codeisland_domain::AppSettings;
use codeisland_runtime::{Runtime, RuntimeHandle};
use tauri::{AppHandle, Manager};

/// Shared application state managed by Tauri and reachable from commands.
pub struct AppState {
    pub runtime: Arc<Runtime>,
    pub(crate) island_visibility: Mutex<window::IslandVisibility>,
    pub(crate) launch_sound_pending: AtomicBool,
    /// Compact shell dimensions in CSS pixels, supplied by the island webview.
    pub hover_strip_size: Mutex<Option<(u32, u32)>>,
    /// Expanded shell dimensions in CSS pixels, supplied by the island webview.
    pub hover_panel_size: Mutex<Option<(u32, u32)>>,
    /// Kept alive for the lifetime of the process so the IPC server, cleanup
    /// task and tailers started by `Runtime::start` are not stopped.
    pub runtime_handle: Mutex<Option<RuntimeHandle>>,
    /// ESP32 Buddy BLE connection, created on demand by Settings.
    pub buddy: tokio::sync::Mutex<Option<codeisland_integrations::ble::BuddyManager>>,
    pub(crate) apple_companion: apple_companion::Publisher,
    /// SSH connections keyed by the configured host id.
    pub(crate) ssh: remote_connections::Connections,
}

/// Mirrors `settings.startup` into the OS autostart registration.
pub(crate) fn apply_autostart(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;

    if is_isolated() {
        return Ok(());
    }
    let manager = app.autolaunch();
    let enabled = manager.is_enabled().map_err(|error| error.to_string())?;
    match (settings.startup, enabled) {
        (true, false) => manager.enable(),
        (false, true) => manager.disable(),
        _ => Ok(()),
    }.map_err(|error| error.to_string())
}

pub(crate) fn is_isolated() -> bool {
    std::env::var("CODEISLAND_ISOLATED").as_deref() == Ok("1")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let isolated = is_isolated();
    if isolated && codeisland_ipc::endpoint::endpoint_override().is_none() {
        eprintln!("CODEISLAND_ISOLATED=1 requires a dedicated CODEISLAND_SOCKET_PATH");
        return;
    }
    let builder = tauri::Builder::default();
    let builder = if isolated {
        builder
    } else {
        // Must be registered before the other plugins.
        builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch opens Settings, the way macOS does. If the
            // settings window cannot be created, reveal the island instead.
            // Queue the work: the callback runs inside a window procedure.
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Err(error) = window::show_settings(&handle) {
                    tracing::warn!(%error, "failed to open settings on second launch");
                    window::show_island(&handle);
                }
            });
        }))
    };
    builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(shortcuts::handle_shortcut)
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--login-launch"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("codeisland".into()),
                    },
                ))
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let handle = app.handle().clone();

            let mut settings = codeisland_runtime::persistence::load_settings()
                .map(codeisland_domain::AppSettings::validate)
                .unwrap_or_default();
            // The starting surface is collapsed. The launch acknowledgment
            // opens it briefly once the window has painted.
            settings.collapsed = true;
            if let Err(error) = apply_autostart(&handle, &settings) {
                tracing::warn!(%error, "failed to update autostart registration");
            }

            let runtime = Runtime::new(settings.clone());
            runtime.set_terminal_visibility_detector(terminal::is_frontmost);
            runtime.set_terminal_tab_visibility_detector(terminal::is_tab_visible);
            #[cfg(windows)]
            away::spawn(&runtime);
            #[cfg(windows)]
            if !isolated {
                wezterm::spawn_probe();
            }
            app.manage(runtime.clone());
            app.manage(claude_quota::PlanQuota::default());
            app.manage(AppState {
                runtime: runtime.clone(),
                island_visibility: Mutex::new(window::IslandVisibility::default()),
                launch_sound_pending: AtomicBool::new(
                    !std::env::args_os().any(|arg| arg == "--login-launch"),
                ),
                hover_strip_size: Mutex::new(None),
                hover_panel_size: Mutex::new(None),
                runtime_handle: Mutex::new(None),
                buddy: tokio::sync::Mutex::new(None),
                apple_companion: apple_companion::Publisher::default(),
                ssh: remote_connections::Connections::default(),
            });

            // The island window is declared in tauri.conf.json. Settings
            // is created on demand.
            window::create_main_window(&handle)?;
            #[cfg(windows)]
            hover::spawn(handle.clone());
            window::spawn_fullscreen_watcher(handle.clone());
            window::spawn_display_watcher(handle.clone());
            // The settings window is created on demand by `show_settings`;
            // creating it hidden at launch still flashed on some WebView2
            // versions.
            if let Err(error) = window::prepare_main_window(&handle, &settings) {
                tracing::warn!(%error, "failed to position island window");
            }

            if let Err(error) = tray::setup(&handle) {
                tracing::warn!(%error, "failed to create tray icon");
            }

            for conflict in shortcuts::register_shortcuts(&handle, &settings) {
                tracing::warn!(
                    action = %conflict.action,
                    shortcut = %conflict.shortcut,
                    error = %conflict.error,
                    "global shortcut unavailable"
                );
            }

            // Start the IPC server, cleanup task and tailers once the windows
            // exist so the webview never misses the first state update.
            claude_quota::spawn(handle.clone(), runtime.clone());
            if settings.show_usage_stats && !isolated {
                let settings = settings.clone();
                tauri::async_runtime::spawn_blocking(move || commands::local_usage_snapshot(&settings));
            }
            let runtime_handle = runtime.start();
            if let Some(state) = handle.try_state::<AppState>() {
                if let Ok(mut guard) = state.runtime_handle.lock() {
                    *guard = Some(runtime_handle);
                }
                if !isolated {
                    state.apple_companion.start(runtime.clone());
                    #[cfg(windows)]
                    ssh_signals::register(state.ssh.recovery_signals());
                    state.ssh.start_auto(runtime.clone());
                }
            }

            events::spawn_forwarder(&handle, runtime.clone());
            window::show_island_when_ready(&handle);

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "settings"
                && matches!(
                    event,
                    tauri::WindowEvent::Focused(false)
                        | tauri::WindowEvent::Destroyed
                        | tauri::WindowEvent::CloseRequested { .. }
                )
            {
                shortcuts::stop_shortcut_recording(window.app_handle());
            }
            if window.label() == "settings"
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                // Let in-flight and queued preference writes finish after X.
                api.prevent_close();
                let _ = window.hide();
            }
            if window.label() == "main" {
                match event {
                    // Closing the island hides it; the app keeps running in the tray.
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        window::hide_island(window.app_handle());
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::take_launch_sound,
            commands::test_push_channel,
            sound_files::pick_custom_sound,
            sound_files::reset_custom_sound,
            sound_files::read_custom_sound,
            commands::resolve_request,
            commands::resolve_current,
            commands::hide_request,
            commands::unhide_request,
            commands::remove_session,
            commands::remove_idle_sessions,
            commands::move_session,
            commands::dismiss_completed,
            commands::get_settings,
            commands::update_settings,
            commands::patch_settings,
            commands::hook_status,
            commands::install_hooks,
            commands::repair_hooks,
            commands::uninstall_hooks,
            commands::custom_cli_status,
            commands::install_custom_cli,
            commands::uninstall_custom_cli,
            commands::export_diagnostics,
            diagnostics::export_diagnostics_as,
            commands::open_workspace,
            commands::focus_session_terminal,
            commands::toggle_collapsed,
            commands::set_island_surface,
            commands::ack_shortcut_card,
            commands::open_pending_question,
            commands::set_shortcut_recording,
            commands::set_hover_panel_size,
            commands::open_settings,
            commands::list_displays,
            commands::terminal_in_front,
            commands::quit_app,
            commands::buddy_connect,
            commands::buddy_disconnect,
            commands::buddy_status,
            commands::apple_companion_status,
            commands::apple_companion_restart,
            commands::quota_snapshot,
            claude_quota::claude_plan_quota,
            commands::claude_config_path,
            commands::ssh_connect,
            commands::ssh_disconnect,
            commands::ssh_status,
            commands::ssh_forward,
            commands::ssh_deploy_hook,
            commands::ssh_uninstall_hooks,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
