use std::{io, path::Path, sync::atomic::Ordering};

use codeisland_agents::ToolStatus;
use codeisland_domain::{AgentKind, AppSettings, CustomCliSetting, Decision, UserAction};
use codeisland_runtime::UiState;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{diagnostics, shortcuts, window, workspace, AppState};

#[tauri::command]
pub fn get_state(state: State<'_, AppState>) -> UiState {
    state.runtime.state()
}

/// Delivers the manual-launch cue once the island can play it.
#[tauri::command]
pub fn take_launch_sound(window: tauri::WebviewWindow, state: State<'_, AppState>) -> bool {
    if window.label() != "main" || !state.launch_sound_pending.swap(false, Ordering::Relaxed) {
        return false;
    }
    if !state.runtime.settings().mute_when_away {
        return true;
    }
    // The island can become ready before the first background lock poll.
    #[cfg(windows)]
    let away = crate::away::is_away_now();
    #[cfg(not(windows))]
    let away = state.runtime.is_away();
    !away && !state.runtime.is_display_asleep()
}

#[tauri::command]
pub async fn test_push_channel(
    mut channel: codeisland_domain::PushChannelSetting,
    title: String,
    body: String,
) -> Result<(), String> {
    channel.enabled = true;
    channel.include_details = true;
    let notifier = codeisland_integrations::PushNotifier::configured(channel, 200)
        .ok_or_else(|| "Unknown push channel".to_owned())?;
    notifier.send_for(codeisland_domain::PushKind::Approval, "🔔 CodeIsland", &title, &body, true)
        .await.map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn ack_shortcut_card(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    event_id: String,
    surface_revision: u32,
) -> bool {
    window.label() == "main"
        && shortcuts::is_window_on_screen(&window)
        && state
            .runtime
            .acknowledge_surface(&event_id, surface_revision)
}

#[tauri::command(rename_all = "camelCase")]
pub fn open_pending_question(
    app: AppHandle,
    state: State<'_, AppState>,
    event_id: Option<String>,
) -> bool {
    open_pending_question_impl(&app, state.inner(), event_id.as_deref())
}

pub(crate) fn open_pending_question_impl(
    app: &AppHandle,
    state: &AppState,
    event_id: Option<&str>,
) -> bool {
    if !state.runtime.open_pending_question(event_id) {
        return false;
    }
    window::bring_island_into_sight(app);
    true
}

#[tauri::command]
pub fn set_shortcut_recording(window: tauri::WebviewWindow, recording: bool) -> bool {
    if window.label() != "settings" {
        return false;
    }
    shortcuts::set_recording(window.app_handle(), recording);
    true
}

#[tauri::command(rename_all = "camelCase")]
pub fn resolve_request(
    state: State<'_, AppState>,
    event_id: String,
    action: UserAction,
    text: Option<String>,
) -> Option<Decision> {
    state.runtime.resolve(&event_id, action, text)
}

#[tauri::command]
pub fn resolve_current(
    state: State<'_, AppState>,
    action: UserAction,
    text: Option<String>,
) -> Option<Decision> {
    state.runtime.resolve_current(action, text)
}

#[tauri::command(rename_all = "camelCase")]
pub fn hide_request(state: State<'_, AppState>, event_id: String) -> bool {
    state.runtime.hide(&event_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn unhide_request(state: State<'_, AppState>, event_id: String) -> bool {
    state.runtime.unhide(&event_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn remove_session(state: State<'_, AppState>, session_id: String) -> bool {
    state.runtime.remove_session(&session_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn move_session(
    state: State<'_, AppState>,
    session_id: String,
    before_id: String,
) -> bool {
    state.runtime.move_session(&session_id, &before_id)
}

#[tauri::command]
pub fn remove_idle_sessions(state: State<'_, AppState>, groups: Vec<Vec<String>>) -> usize {
    state.runtime.remove_idle_sessions(groups)
}

#[tauri::command]
pub fn dismiss_completed(state: State<'_, AppState>) -> usize {
    state.runtime.dismiss_completed()
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.runtime.settings()
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    apply_preferences(&app, state.inner(), |_| Ok(settings))
}

#[tauri::command]
pub fn patch_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, String> {
    apply_preferences(&app, state.inner(), |current| merge_preferences(current, patch))
}

fn merge_preferences(
    current: &AppSettings,
    patch: serde_json::Map<String, serde_json::Value>,
) -> io::Result<AppSettings> {
    let mut value = serde_json::to_value(current).map_err(io::Error::other)?;
    let object = value.as_object_mut().expect("settings serialize as an object");
    for (key, value) in patch {
        if !object.contains_key(&key) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unknown setting {key}")));
        }
        object.insert(key, value);
    }
    serde_json::from_value(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn apply_preferences(
    app: &AppHandle,
    state: &AppState,
    change: impl FnOnce(&AppSettings) -> io::Result<AppSettings>,
) -> Result<AppSettings, String> {
    let mut previous_startup = None;
    let updated = state.runtime.try_update_settings(|current| {
        let mut settings = change(current)?;
        normalize_display_preference(app, &mut settings);
        if current.panel_edge != settings.panel_edge {
            settings.panel_x = None;
            settings.panel_y = None;
        }
        if current.startup != settings.startup {
            crate::apply_autostart(app, &settings).map_err(io::Error::other)?;
            previous_startup = Some(current.startup);
        }
        Ok(settings)
    });
    match updated {
        Ok(settings) => Ok(settings),
        Err(error) => {
            if let Some(startup) = previous_startup {
                let settings = AppSettings { startup, ..state.runtime.settings() };
                if let Err(error) = crate::apply_autostart(app, &settings) {
                    tracing::warn!(%error, "failed to restore autostart after a settings write error");
                }
            }
            Err(error.to_string())
        }
    }
}

fn normalize_display_preference(app: &AppHandle, settings: &mut AppSettings) {
    // Mirror legacy selectors so saved preferences work across versions.
    match settings.display_choice.as_str() {
        "cursor" => settings.display_mode = "cursor".into(),
        "primary" => settings.display_mode = "primary".into(),
        "auto" | "pinned" => {
            if settings.display_choice == "pinned" {
                settings.display_mode = "pinned".into();
            }
            if settings.display_mode == "pinned"
                && settings.pinned_display.as_deref().is_none_or(|id| id.trim().is_empty())
            {
                settings.pinned_display = app.get_webview_window("main")
                    .and_then(|island| window::monitor_device_name(&island));
            }
        }
        id => {
            settings.display_mode = "pinned".into();
            settings.pinned_display = Some(id.strip_prefix("display:").unwrap_or(id).to_owned());
        }
    }
}

#[tauri::command]
pub async fn claude_config_path(preference: String) -> String {
    let paths = codeisland_agents::paths::Paths::current();
    let path = paths.resolve_claude_config_dir(&preference);
    match path.strip_prefix(paths.home()) {
        Ok(relative) => std::path::Path::new("~").join(relative).display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

#[tauri::command]
pub async fn hook_status(state: State<'_, AppState>) -> Result<Vec<ToolStatus>, String> {
    hook_action_on_worker(state.runtime.settings(), None, None).await
}

#[tauri::command]
pub async fn install_hooks(
    state: State<'_, AppState>,
    agent: Option<AgentKind>,
) -> Result<Vec<ToolStatus>, String> {
    hook_action_on_worker(state.runtime.settings(), agent, Some(HookAction::Install)).await
}

#[tauri::command]
pub async fn repair_hooks(
    state: State<'_, AppState>,
    agent: Option<AgentKind>,
) -> Result<Vec<ToolStatus>, String> {
    hook_action_on_worker(state.runtime.settings(), agent, Some(HookAction::Repair)).await
}

#[tauri::command]
pub async fn uninstall_hooks(
    state: State<'_, AppState>,
    agent: Option<AgentKind>,
) -> Result<Vec<ToolStatus>, String> {
    hook_action_on_worker(state.runtime.settings(), agent, Some(HookAction::Uninstall)).await
}

/// Validate a Settings entry and turn it into an installer definition.
/// Validation happens at the command boundary because the config path is
/// user-supplied and ends up in a file write.
fn custom_definition(
    setting: &CustomCliSetting,
) -> Result<codeisland_agents::AgentDefinition, String> {
    let name = setting.name.trim();
    let source = setting.source.trim().to_ascii_lowercase();
    let config_path = setting.config_path.trim();
    let config_key = setting.config_key.trim();
    if name.is_empty() {
        return Err("name cannot be empty".into());
    }
    if source.is_empty()
        || !source
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("source must use [a-z0-9_-]".into());
    }
    if codeisland_agents::definition_for_source(&source).is_some() {
        return Err(format!("source '{source}' is already built-in"));
    }
    if config_path.is_empty() {
        return Err("config path cannot be empty".into());
    }
    if config_key.is_empty() {
        return Err("config key cannot be empty".into());
    }
    let format = codeisland_agents::HookFormat::parse(&setting.format)
        .filter(|format| codeisland_agents::custom_format_supported(*format))
        .ok_or_else(|| "unsupported template".to_owned())?;
    Ok(codeisland_agents::AgentDefinition::custom(
        name,
        source,
        config_path,
        format,
        config_key,
    ))
}

#[tauri::command]
pub fn custom_cli_status(setting: CustomCliSetting) -> Result<ToolStatus, String> {
    let def = custom_definition(&setting)?;
    Ok(codeisland_agents::custom_status(
        &codeisland_agents::paths::Paths::current(),
        &def,
    ))
}

#[tauri::command]
pub fn install_custom_cli(setting: CustomCliSetting) -> Result<ToolStatus, String> {
    let def = custom_definition(&setting)?;
    codeisland_agents::install_custom(&codeisland_agents::paths::Paths::current(), &def)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn uninstall_custom_cli(setting: CustomCliSetting) -> Result<ToolStatus, String> {
    let def = custom_definition(&setting)?;
    codeisland_agents::uninstall_custom(&codeisland_agents::paths::Paths::current(), &def)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn export_diagnostics(app: AppHandle, state: State<'_, AppState>, path: String) -> String {
    match diagnostics::export(&app, state.inner(), Path::new(&path)) {
        Ok(exported) => exported,
        Err(error) => {
            tracing::error!(%error, "failed to export diagnostics");
            let _ = app.emit(
                "island://toast",
                json!({
                    "kind": "error",
                    "message": format!("Failed to export diagnostics: {error}"),
                }),
            );
            path
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn open_workspace(state: State<'_, AppState>, session_id: String) -> Result<bool, String> {
    let snapshot = state.runtime.state();
    let session = snapshot
        .store
        .sessions
        .iter()
        .find(|session| session.session_id == session_id);
    if let Some(session) = session
        && workspace::launch_for_session(session).await
    {
        state.runtime.user_jumped_to_session(&session_id);
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn focus_session_terminal(state: State<'_, AppState>, session_id: String) -> Result<bool, String> {
    if session_id.starts_with("ssh:") {
        return Ok(false);
    }
    let snapshot = state.runtime.state();
    let Some(session) = snapshot.store.sessions.iter().find(|session| session.session_id == session_id) else {
        return Ok(false);
    };
    if workspace::focus_session_terminal(session).await != Some(true) {
        return Ok(false);
    }
    state.runtime.user_jumped_to_session(&session_id);
    Ok(true)
}

pub(crate) fn toggle_collapsed_impl(app: &AppHandle, state: &AppState) -> bool {
    let target = if state.runtime.surface().is_collapsed() {
        codeisland_runtime::IslandSurface::SessionList
    } else {
        codeisland_runtime::IslandSurface::Collapsed
    };
    set_island_surface_impl(app, state, target);
    state.runtime.surface().is_collapsed()
}

pub(crate) fn set_island_surface_impl(
    app: &AppHandle,
    state: &AppState,
    surface: codeisland_runtime::IslandSurface,
) -> bool {
    let applied = state.runtime.set_surface(surface);
    if applied {
        window::sync_island(app);
    }
    applied
}

#[tauri::command]
pub fn toggle_collapsed(app: AppHandle, state: State<'_, AppState>) -> bool {
    toggle_collapsed_impl(&app, state.inner())
}

#[tauri::command]
pub fn set_island_surface(
    app: AppHandle,
    state: State<'_, AppState>,
    surface: codeisland_runtime::IslandSurface,
) -> bool {
    set_island_surface_impl(&app, state.inner(), surface)
}

#[tauri::command]
pub fn set_hover_panel_size(
    state: State<'_, AppState>,
    width: u32,
    height: u32,
    collapsed: bool,
) {
    let size = if collapsed {
        &state.hover_strip_size
    } else {
        &state.hover_panel_size
    };
    if width > 0 && height > 0 && let Ok(mut size) = size.lock() {
        *size = Some((width, height));
    }
}

#[tauri::command]
pub async fn open_settings(app: AppHandle) -> Result<(), String> {
    // The async command posts to the event loop after the WebView2 IPC
    // callback has returned, so creating the second webview can pump messages.
    let (done, result) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = done.send(window::show_settings(&handle));
    })
    .map_err(|error| error.to_string())?;
    result
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// Connected displays for the settings page, shaped `{id,name,isPrimary}`.
#[tauri::command]
pub fn list_displays(app: AppHandle) -> Vec<window::DisplayInfo> {
    window::list_displays(&app)
}

/// Foreground check for the active session or an explicitly supplied process.
#[tauri::command(rename_all = "camelCase")]
pub fn terminal_in_front(
    state: State<'_, AppState>,
    pid: Option<u32>,
    project: Option<String>,
) -> bool {
    let settings = state.runtime.settings();
    let result = match (pid, project.as_deref()) {
        (None, None) => state.runtime.current_session_in_front(),
        (pid, _) => pid.is_some_and(crate::terminal::is_foreground_process),
    };
    // Smart Suppress is what gives this signal meaning; without it the
    // runtime must not treat the terminal as "the user is already looking".
    state
        .runtime
        .set_session_in_front(result && settings.smart_suppress);
    result
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[derive(Clone, Copy)]
enum HookAction {
    Install,
    Repair,
    Uninstall,
}

async fn hook_action_on_worker(
    settings: AppSettings,
    agent: Option<AgentKind>,
    action: Option<HookAction>,
) -> Result<Vec<ToolStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        static HOOK_OPERATIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _operation = HOOK_OPERATIONS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let paths = codeisland_agents::paths::Paths::current().with_settings(&settings);
        let statuses = codeisland_agents::doctor::detect_all_with(&paths);
        let Some(action) = action else {
            return Ok(statuses);
        };
        let targets: Vec<AgentKind> = match agent {
            Some(agent) => vec![agent],
            None => statuses.into_iter().filter(|status| status.installed)
                .map(|status| status.agent).collect(),
        };
        let mut failures = Vec::new();
        for target in targets {
            let result = match action {
                HookAction::Install | HookAction::Repair => {
                    codeisland_agents::install::install_with(&paths, target)
                }
                HookAction::Uninstall => codeisland_agents::install::uninstall_with(&paths, target),
            };
            if let Err(error) = result {
                failures.push(format!("{}: {error}", target.tag()));
            }
        }
        if failures.is_empty() {
            Ok(codeisland_agents::doctor::detect_all_with(&paths))
        } else {
            Err(failures.join("\n"))
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Invoked by the tray "Install hooks" item.
pub(crate) fn install_hooks_from_tray(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let settings = state.runtime.settings();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (kind, message) = match hook_action_on_worker(settings, None, Some(HookAction::Install)).await {
            Ok(statuses) => {
                let installed = statuses.iter().filter(|status| status.hook_installed).count();
                ("info", format!("Hooks installed for {installed} agent(s)"))
            }
            Err(error) => ("error", error),
        };
        let _ = app.emit("island://toast", json!({ "kind": kind, "message": message }));
    });
}

#[tauri::command]
pub async fn buddy_connect(state: State<'_, AppState>) -> Result<String, String> {
    let mut guard = state.buddy.lock().await;
    if guard.as_ref().is_some_and(|manager| manager.is_connected()) {
        return Ok("connected".to_owned());
    }
    let mut manager = codeisland_integrations::ble::BuddyManager::new()
        .await
        .map_err(|error| error.to_string())?;
    manager
        .scan_and_connect()
        .await
        .map_err(|error| error.to_string())?;
    *guard = Some(manager);
    Ok("connected".to_owned())
}

#[tauri::command]
pub async fn buddy_disconnect(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.buddy.lock().await;
    if let Some(manager) = guard.as_mut() {
        manager
            .disconnect()
            .await
            .map_err(|error| error.to_string())?;
    }
    *guard = None;
    Ok(())
}

#[tauri::command]
pub async fn buddy_status(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state.buddy.lock().await;
    Ok(match guard.as_ref() {
        Some(manager) if manager.is_connected() => "connected",
        Some(_) => "disconnected",
        None => "not_connected",
    }
    .to_owned())
}

#[tauri::command]
pub fn apple_companion_status(state: State<'_, AppState>) -> crate::apple_companion::Status {
    state.apple_companion.status()
}

#[tauri::command]
pub fn apple_companion_restart(state: State<'_, AppState>) {
    state.apple_companion.restart();
}


#[tauri::command]
pub async fn quota_snapshot(
    state: State<'_, AppState>,
) -> Result<Option<codeisland_integrations::quota::QuotaSnapshot>, String> {
    // Opening the panel requests usage. Scanning local transcripts must
    // leave the UI thread free to process hover transitions and clicks.
    let settings = state.runtime.settings();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        local_usage_snapshot(&settings)
    })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to scan local usage");
            None
        });
    Ok(snapshot)
}

pub(crate) fn local_usage_snapshot(settings: &AppSettings) -> Option<codeisland_integrations::quota::QuotaSnapshot> {
    let paths = codeisland_agents::paths::Paths::current().with_settings(settings);
    codeisland_integrations::quota::scan_roots_cached(&paths.claude_config_dirs(), chrono::Utc::now())
}

#[tauri::command]
pub async fn ssh_connect(
    state: State<'_, AppState>,
    host_id: String,
) -> Result<String, String> {
    let host = state
        .runtime
        .settings()
        .remote_hosts
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| "remote host not found".to_owned())?;
    state.ssh.connect(host, state.runtime.clone()).await
}

#[tauri::command]
pub async fn ssh_disconnect(
    state: State<'_, AppState>,
    host_id: String,
) -> Result<(), String> {
    state.ssh.disconnect(&host_id).await;
    Ok(())
}

#[tauri::command]
pub async fn ssh_status(state: State<'_, AppState>, host_id: String) -> Result<String, String> {
    Ok(state.ssh.status(&host_id).to_owned())
}

#[tauri::command]
pub async fn ssh_forward(
    state: State<'_, AppState>,
    host_id: String,
    local_port: Option<u16>,
    remote_host: String,
    remote_port: u16,
) -> Result<u16, String> {
    let manager = state.ssh.manager(&host_id)?;
    manager.lock().await
        .forward_local(local_port.unwrap_or(0), &remote_host, remote_port)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ssh_deploy_hook(
    state: State<'_, AppState>,
    host_id: String,
    remote_dir: Option<String>,
) -> Result<codeisland_integrations::ssh::DeployResult, String> {
    const REMOTE_HOOK: &[u8] = include_bytes!(
        "../../../Sources/CodeIsland/Resources/codeisland-remote-hook.py"
    );
    const OPENCODE_PLUGIN: &str = include_str!(
        "../../../Sources/CodeIsland/Resources/codeisland-opencode-remote.js"
    );
    let dir = remote_hook_dir(remote_dir.as_deref());
    let custom = remote_custom_clis(&state.runtime.settings());
    let manager = state.ssh.manager(&host_id)?;
    let mut manager = manager.lock().await;
    let handler = codeisland_runtime::remote_hook_handler(state.runtime.clone(), host_id);
    let mut deployed = manager
        .deploy_remote_hook(REMOTE_HOOK, dir, handler)
        .await
        .map_err(|error| error.to_string())?;
    deployed.summary = Some(manager
        .configure_remote_hooks(&deployed, OPENCODE_PLUGIN, custom)
        .await
        .map_err(|error| error.to_string())?);
    Ok(deployed)
}

#[tauri::command]
pub async fn ssh_uninstall_hooks(
    state: State<'_, AppState>,
    host_id: String,
    remote_dir: Option<String>,
) -> Result<String, String> {
    let manager = state.ssh.manager(&host_id)?;
    manager.lock().await.uninstall_remote_hooks(remote_hook_dir(remote_dir.as_deref()))
        .await
        .map_err(|error| error.to_string())
}

fn remote_hook_dir(remote_dir: Option<&str>) -> &str {
    remote_dir.map(str::trim).filter(|dir| !dir.is_empty()).unwrap_or("~/.codeisland")
}

fn remote_custom_clis(settings: &AppSettings) -> codeisland_integrations::ssh::RemoteHookConfig {
    use codeisland_agents::HookFormat;
    use codeisland_integrations::ssh::{RemoteCustomCli, RemoteHookConfig};

    let paths = codeisland_agents::paths::Paths::current();
    let home = paths.home().to_string_lossy().replace('\\', "/");
    let mut custom = RemoteHookConfig::default();
    for setting in &settings.custom_clis {
        let Ok(definition) = custom_definition(setting) else {
            custom.unsupported_custom_clis.push(setting.name.clone());
            continue;
        };
        let format = match definition.format {
            HookFormat::ClaudeJson => "claude",
            HookFormat::EventMapJson => "nested",
            _ => {
                custom.unsupported_custom_clis.push(setting.name.clone());
                continue;
            }
        };
        let path = setting.config_path.trim().replace('\\', "/");
        let local_home = path.get(..home.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(&home));
        let path = if local_home {
            path[home.len()..].strip_prefix('/').unwrap_or(&path)
        } else {
            ["~/", "%USERPROFILE%/", "$HOME/"].into_iter()
                .find_map(|prefix| path.strip_prefix(prefix))
                .unwrap_or(&path)
        };
        if path.is_empty() || path.as_bytes().get(1) == Some(&b':') || path.starts_with("//") {
            custom.unsupported_custom_clis.push(setting.name.clone());
            continue;
        }
        custom.custom_clis.push(RemoteCustomCli {
            name: definition.display_name,
            source: definition.source,
            config_path: path.to_owned(),
            config_key: definition.config_key,
            format,
            events: definition.events.into_iter().map(|event| (event.name, event.timeout_ms / 1000)).collect(),
        });
    }
    custom
}
