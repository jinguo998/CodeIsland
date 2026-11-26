use std::path::Path;

use codeisland_domain::settings::SOUND_FILE_NAMES;
use tauri::{State, WebviewWindow, ipc::Response};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "m4a", "aac", "flac", "ogg", "oga", "opus", "webm", "wma", "aif", "aiff",
];

fn valid_cue(cue: &str) -> bool {
    SOUND_FILE_NAMES.contains(&cue)
}

fn is_audio_path(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

#[tauri::command]
pub async fn pick_custom_sound(
    window: WebviewWindow,
    state: State<'_, AppState>,
    cue: String,
    title: String,
) -> Result<Option<String>, String> {
    if window.label() != "settings" || !valid_cue(&cue) {
        return Err("Invalid sound selection".into());
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    window.dialog().file().set_parent(&window).set_title(&title)
        .add_filter(title, AUDIO_EXTENSIONS)
        .pick_file(move |selected| { let _ = send.send(selected); });
    let Some(selected) = receive.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };

    let path = selected.into_path().map_err(|error| error.to_string())?;
    if !is_audio_path(&path) || !path.is_file() {
        return Err("Cannot use the selected audio file".into());
    }
    let path = path.into_os_string().into_string().map_err(|_| "Invalid audio path")?;
    state.runtime.set_custom_sound_path(&cue, Some(path.clone())).map_err(|error| error.to_string())?;
    Ok(Some(path))
}

#[tauri::command]
pub fn reset_custom_sound(
    window: WebviewWindow,
    state: State<'_, AppState>,
    cue: String,
) -> Result<(), String> {
    if window.label() != "settings" || !valid_cue(&cue) {
        return Err("Invalid sound selection".into());
    }
    state.runtime.set_custom_sound_path(&cue, None).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn read_custom_sound(
    window: WebviewWindow,
    state: State<'_, AppState>,
    cue: String,
    path: String,
) -> Result<Response, String> {
    if !matches!(window.label(), "main" | "settings") || !valid_cue(&cue)
        || !state.runtime.settings().custom_sound_paths.get(&cue).is_some_and(|current| current == &path)
        || !is_audio_path(Path::new(&path))
    {
        return Err("The configured audio file is unavailable".into());
    }

    tauri::async_runtime::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(|error| error.to_string())?
        .map(Response::new)
        .map_err(|error| error.to_string())
}
