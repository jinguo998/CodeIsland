//! On-disk persistence for session state and settings.
//!
//! State lives in `%LOCALAPPDATA%\CodeIsland\state` on Windows and in
//! `$XDG_DATA_HOME/CodeIsland/state` (falling back to `~/.local/share/CodeIsland/state`)
//! on Unix. `CODEISLAND_STATE_DIR` overrides the directory for tests and diagnostics.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use codeisland_domain::{AppSettings, PersistedState};

/// Directory holding `sessions.json` and `settings.json`.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CODEISLAND_STATE_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(dir);
    }
    if std::env::var("CODEISLAND_ISOLATED").as_deref() == Ok("1") {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        codeisland_ipc::endpoint_name().hash(&mut hasher);
        return std::env::temp_dir().join(format!("codeisland-isolated-{:016x}", hasher.finish()));
    }
    platform_state_dir()
}

/// Path of the persisted session snapshot.
pub fn state_path() -> PathBuf {
    state_dir().join("sessions.json")
}

/// Path of the persisted application settings.
pub fn settings_path() -> PathBuf {
    #[cfg(windows)]
    if std::env::var_os("CODEISLAND_STATE_DIR").is_none_or(|dir| dir.is_empty())
        && std::env::var("CODEISLAND_ISOLATED").as_deref() != Ok("1")
        && let Some(appdata) = std::env::var_os("APPDATA").filter(|dir| !dir.is_empty())
    {
        return PathBuf::from(appdata).join("CodeIsland").join("settings.json");
    }
    state_dir().join("settings.json")
}

#[cfg(windows)]
fn platform_state_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join("AppData").join("Local")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CodeIsland")
        .join("state")
}

#[cfg(not(windows))]
fn platform_state_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CodeIsland")
        .join("state")
}

/// Loads the persisted sessions, returning an empty state when the file is missing or
/// unreadable. A corrupt primary file falls back to the `.bak` copy.
pub fn load_state() -> PersistedState {
    load_state_from(&state_path()).unwrap_or_default()
}

/// Loads a session snapshot from an explicit path (primary, then `.bak`).
pub fn load_state_from(path: &Path) -> Option<PersistedState> {
    read_json(path).or_else(|| read_json(&backup_path(path)))
}

/// Atomically writes the session snapshot next to the default state path.
pub fn save_state(state: &PersistedState) -> io::Result<()> {
    save_state_to(&state_path(), state)
}

/// Atomically writes the session snapshot to an explicit path.
pub fn save_state_to(path: &Path, state: &PersistedState) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

/// Loads settings, returning `None` when they are missing or unreadable.
pub fn load_settings() -> Option<AppSettings> {
    let path = settings_path();
    load_settings_from(&path).or_else(|| {
        // Older Windows releases also wrote a copy next to the session state.
        let legacy = state_dir().join("settings.json");
        (legacy != path).then(|| load_settings_from(&legacy)).flatten()
    })
}

pub fn load_settings_from(path: &Path) -> Option<AppSettings> {
    read_json(path).or_else(|| read_json(&backup_path(path)))
}

/// Atomically writes settings.
pub fn save_settings(settings: &AppSettings) -> io::Result<()> {
    save_settings_to(&settings_path(), settings)
}

pub fn save_settings_to(path: &Path, settings: &AppSettings) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let data = fs::read_to_string(path).ok()?;
    if data.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&data).ok()
}

fn backup_path(path: &Path) -> PathBuf {
    with_suffix(path, ".bak")
}

fn temp_path(path: &Path) -> PathBuf {
    with_suffix(path, ".tmp")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state.json".to_owned());
    path.with_file_name(format!("{file_name}{suffix}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "state path has no parent directory")
    })?;
    fs::create_dir_all(parent)?;

    let temp = temp_path(path);
    let backup = backup_path(path);

    {
        use std::io::Write;
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    if path.exists() {
        // Keep the previous version around so a corrupt write can be recovered.
        let _ = fs::copy(path, &backup);
    }

    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, SessionStore};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "codeisland-persistence-{label}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn snapshot_with(session_id: &str) -> PersistedState {
        let mut store = SessionStore::new(8, 200);
        let event = AgentEvent::new(session_id, AgentKind::Claude, AgentEventType::UserPrompt);
        store.apply(&event);
        store.persisted()
    }

    #[test]
    fn state_round_trip() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("sessions.json");
        let state = snapshot_with("session-one");

        save_state_to(&path, &state).expect("save");
        assert!(path.exists());

        let loaded = load_state_from(&path).expect("load");
        assert_eq!(loaded.sessions.len(), 1);
        assert!(loaded.sessions.contains_key("session-one"));
    }

    #[test]
    fn second_write_creates_backup_and_recovers() {
        let dir = temp_dir("backup");
        let path = dir.join("sessions.json");

        save_state_to(&path, &snapshot_with("first")).expect("first save");
        save_state_to(&path, &snapshot_with("second")).expect("second save");
        assert!(backup_path(&path).exists());

        // Corrupt the primary; loading should fall back to the backup.
        fs::write(&path, b"{ this is not json").expect("corrupt");
        let loaded = load_state_from(&path).expect("load backup");
        assert!(loaded.sessions.contains_key("first"));
    }

    #[test]
    fn missing_and_corrupt_files_are_tolerated() {
        let dir = temp_dir("tolerant");
        assert!(load_state_from(&dir.join("missing.json")).is_none());

        let corrupt = dir.join("corrupt.json");
        fs::write(&corrupt, b"not json at all").expect("write");
        assert!(load_state_from(&corrupt).is_none());

        let empty = dir.join("empty.json");
        fs::write(&empty, b"   \n").expect("write");
        assert!(load_state_from(&empty).is_none());
    }

    #[test]
    fn settings_round_trip() {
        let dir = temp_dir("settings");
        // Exercise the helpers directly so the test never touches the user's real state dir.
        let path = dir.join("settings.json");
        let settings = AppSettings::default();
        let bytes = serde_json::to_vec_pretty(&settings).expect("serialize");
        write_atomic(&path, &bytes).expect("write");
        let loaded: AppSettings = read_json(&path).expect("read");
        assert_eq!(loaded, settings);
    }
}
