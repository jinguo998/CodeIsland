use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use codeisland_domain::SessionSnapshot;
use serde::Deserialize;
use tokio::process::Command;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

#[derive(Deserialize)]
struct Pane {
    pane_id: u32,
    #[serde(default)]
    is_active: bool,
    cwd: Option<String>,
}

static ACTIVE: LazyLock<Mutex<Option<(Instant, Pane)>>> = LazyLock::new(|| Mutex::new(None));

pub(crate) fn spawn_probe() {
    let Some(binary) = binary() else { return };
    tauri::async_runtime::spawn(async move {
        loop {
            let active = if crate::terminal::foreground_is_wezterm() {
                list_panes(&binary)
                    .await
                    .and_then(|panes| panes.into_iter().find(|pane| pane.is_active))
                    .map(|pane| (Instant::now(), pane))
            } else {
                None
            };
            *ACTIVE.lock().unwrap_or_else(|error| error.into_inner()) = active;
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
    });
}

pub(crate) fn is_active(session: &SessionSnapshot) -> bool {
    let active = ACTIVE.lock().unwrap_or_else(|error| error.into_inner());
    let Some((updated, active)) = active.as_ref() else {
        return false;
    };
    if updated.elapsed() > Duration::from_millis(700) {
        return false;
    }
    session.wezterm_pane.map_or_else(
        || same_cwd(session.cwd.as_deref(), active.cwd.as_deref()),
        |pane| pane == active.pane_id,
    )
}

pub(crate) async fn activate(session: &SessionSnapshot) -> bool {
    let Some(binary) = binary() else { return false };
    if let Some(id) = session.wezterm_pane
        && activate_id(&binary, id).await
    {
        return true;
    }
    let Some(panes) = list_panes(&binary).await else {
        return false;
    };
    let mut matching = panes
        .into_iter()
        .filter(|pane| same_cwd(session.cwd.as_deref(), pane.cwd.as_deref()));
    let Some(pane) = matching.next() else {
        return false;
    };
    matching.next().is_none() && activate_id(&binary, pane.pane_id).await
}

pub(crate) async fn activate_wsl_pane(id: u32) -> bool {
    let Some(binary) = binary() else { return false };
    activate_id(&binary, id).await
        && list_panes(&binary).await.is_some_and(|panes| {
            panes.iter().any(|pane| pane.pane_id == id && pane.is_active)
        })
}

async fn activate_id(binary: &Path, id: u32) -> bool {
    command(
        binary,
        &["cli", "activate-pane", "--pane-id", &id.to_string()],
    )
    .await
    .is_some()
}

async fn list_panes(binary: &Path) -> Option<Vec<Pane>> {
    let output = command(binary, &["cli", "list", "--format", "json"]).await?;
    serde_json::from_slice(&output).ok()
}

async fn command(binary: &Path, args: &[&str]) -> Option<Vec<u8>> {
    use std::process::Stdio;

    let mut process = Command::new(binary);
    process
        .args(args)
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_millis(900), process.output())
        .await
        .ok()?
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn binary() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("wezterm.exe"))
        .chain(
            std::env::var_os("ProgramFiles")
                .map(|root| PathBuf::from(root).join("WezTerm/wezterm.exe")),
        )
        .chain(
            std::env::var_os("LOCALAPPDATA")
                .map(|root| PathBuf::from(root).join("Programs/WezTerm/wezterm.exe")),
        )
        .find(|path| path.is_file())
}

fn same_cwd(session: Option<&str>, pane: Option<&str>) -> bool {
    let (Some(session), Some(pane)) = (session, pane) else {
        return false;
    };
    let path = tauri::Url::parse(pane)
        .ok()
        .and_then(|url| url.to_file_path().ok());
    let pane = path.as_ref().map_or_else(
        || pane.to_owned(),
        |path| path.to_string_lossy().into_owned(),
    );
    let comparable = |value: &str| {
        value
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    comparable(session) == comparable(&pane)
}
