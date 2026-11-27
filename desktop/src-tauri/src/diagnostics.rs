use std::io;
use std::path::Path;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

/// Markers that identify secrets in arbitrary strings.
const SECRET_MARKERS: [&str; 4] = ["sk-", "bearer ", "token", "api_key"];

const MAX_LOG_LINES: usize = 200;

/// Writes the diagnostics document to `path` and returns the written path.
pub fn export(app: &AppHandle, state: &AppState, path: &Path) -> io::Result<String> {
    let document = build(app, state);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let payload = serde_json::to_vec_pretty(&document)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary = path.with_extension(format!(
        "json.tmp-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&temporary, payload)?;
    std::fs::rename(&temporary, path)?;

    Ok(path.display().to_string())
}

/// Lets the user select where the redacted diagnostics file should go.
#[tauri::command]
pub async fn export_diagnostics_as(
    window: WebviewWindow,
    title: String,
) -> Result<Option<String>, String> {
    if window.label() != "settings" {
        return Err("Diagnostics destination must be selected in Settings".into());
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let file_name = format!("codeisland-diagnostics-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title(title)
        .set_file_name(file_name)
        .add_filter("JSON", &["json"])
        .save_file(move |selected| { let _ = send.send(selected); });
    let Some(selected) = receive.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|error| error.to_string())?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        export(&app, state.inner(), &path)
            .map(Some)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Tray helper: writes to the app data directory and reports through a toast.
pub fn export_default(app: &AppHandle) -> Option<String> {
    let directory = app.path().app_data_dir().ok()?;
    let path = directory.join(format!(
        "diagnostics-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    let state = app.try_state::<AppState>()?;
    let exported = export(app, state.inner(), &path).ok()?;

    let _ = app.emit(
        "island://toast",
        json!({
            "kind": "info",
            "message": format!("Diagnostics exported to {exported}"),
        }),
    );
    Some(exported)
}

fn build(app: &AppHandle, state: &AppState) -> Value {
    let mut document = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "os": os_description(),
        "settings": state.runtime.settings(),
        "sessions": state.runtime.state().store,
        "hooks": codeisland_agents::detect_all(),
        "logs": recent_logs(app),
    });

    redact_push_settings(&mut document["settings"]);
    redact_value(&mut document);
    codeisland_swift::call(c"desktop.diagnostics", document)
}

fn redact_push_settings(settings: &mut Value) {
    fn redact_field(value: &mut Value, field: &str) {
        if value.get(field).and_then(Value::as_str).is_some_and(|text| !text.is_empty()) {
            value[field] = json!("[REDACTED]");
        }
    }

    redact_field(settings, "push_target");
    if let Some(channels) = settings["push_channels"].as_array_mut() {
        for channel in channels {
            for field in ["endpoint", "target", "token", "secret"] {
                redact_field(channel, field);
            }
        }
    }
}

fn os_description() -> String {
    let name = std::env::var("OS").unwrap_or_else(|_| std::env::consts::OS.to_owned());
    format!("{name} {} ({})", std::env::consts::ARCH, std::env::consts::OS)
}

fn recent_logs(app: &AppHandle) -> Vec<String> {
    let Ok(directory) = app.path().app_log_dir() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(directory.join("codeisland.log")) else {
        return Vec::new();
    };

    let mut lines: Vec<String> = text
        .lines()
        .rev()
        .take(MAX_LOG_LINES)
        .map(ToOwned::to_owned)
        .collect();
    lines.reverse();
    lines
}

fn redact_value(value: &mut Value) {
    match value {
        Value::String(text) => {
            let redacted = redact_string(text);
            *text = redacted;
        }
        Value::Array(items) => items.iter_mut().for_each(redact_value),
        Value::Object(map) => map.values_mut().for_each(redact_value),
        _ => {}
    }
}

fn redact_string(value: &str) -> String {
    if is_secret(value) {
        return "[REDACTED]".to_owned();
    }

    let mut redacted = value.to_owned();
    for profile in profile_paths() {
        redacted = redacted.replace(&profile, "%USERPROFILE%");
    }
    redacted
}

fn is_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    SECRET_MARKERS.iter().any(|marker| lower.contains(marker))
}

fn profile_paths() -> Vec<String> {
    ["USERPROFILE", "HOME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .filter(|path| !path.is_empty())
        .collect()
}
