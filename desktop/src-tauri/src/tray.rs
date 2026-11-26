use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::{AppState, commands, diagnostics, window};

#[cfg(any(windows, test))]
#[path = "tray_icon.rs"]
mod tray_icon;

const TRAY_ID: &str = "codeisland";
#[cfg(windows)]
const TRAY_IMAGE_SIZE: u32 = 32;

#[cfg(windows)]
fn tray_image(foreground: u8) -> Image<'static> {
    Image::new_owned(
        tray_icon::render(TRAY_IMAGE_SIZE, foreground),
        TRAY_IMAGE_SIZE,
        TRAY_IMAGE_SIZE,
    )
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let language = app.state::<AppState>().runtime.settings().language;
    let menu = make_menu(app, &language)?;
    #[cfg(windows)]
    let foreground = taskbar_foreground();
    #[cfg(windows)]
    let icon = tray_image(foreground);
    #[cfg(not(windows))]
    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("CodeIsland")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => window::toggle_island(app),
            "settings" => {
                if let Err(error) = window::show_settings(app) {
                    tracing::warn!(%error, "failed to open settings window");
                }
            }
            "install_hooks" => commands::install_hooks_from_tray(app),
            "export_diagnostics" => {
                if diagnostics::export_default(app).is_none() {
                    tracing::warn!("failed to export diagnostics from tray");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::toggle_island(tray.app_handle());
            }
        })
        .build(app)?;

    #[cfg(windows)]
    watch_taskbar_theme(app.clone(), foreground);

    Ok(())
}

#[cfg(windows)]
const PERSONALIZE_KEY: windows::core::PCWSTR =
    windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

#[cfg(windows)]
fn taskbar_foreground() -> u8 {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;

    let mut light_mode = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the DWORD output buffer and its length are valid for this call.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE_KEY,
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut light_mode as *mut u32).cast()),
            Some(&mut size),
        )
    };
    if result == ERROR_SUCCESS && light_mode != 0 {
        0
    } else {
        255
    }
}

#[cfg(windows)]
fn watch_taskbar_theme(app: AppHandle, mut foreground: u8) {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RegCloseKey,
        RegNotifyChangeKeyValue, RegOpenKeyExW,
    };

    // The blocking registry notification belongs on a dedicated thread. The
    // tray API dispatches the icon update onto Tauri's main thread.
    let _ = std::thread::Builder::new()
        .name("codeisland-taskbar-theme".into())
        .spawn(move || {
            let mut key = HKEY::default();
            // SAFETY: key is initialized on success and closed below.
            if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PERSONALIZE_KEY, None, KEY_NOTIFY, &mut key) }
                != ERROR_SUCCESS
            {
                return;
            }
            loop {
                let next = taskbar_foreground();
                if next != foreground {
                    let Some(tray) = app.tray_by_id(TRAY_ID) else {
                        break;
                    };
                    if tray.set_icon(Some(tray_image(next))).is_err() {
                        break;
                    }
                    foreground = next;
                }
                // SAFETY: key stays open throughout the synchronous wait.
                if unsafe { RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false) }
                    != ERROR_SUCCESS
                {
                    break;
                }
            }
            // SAFETY: this thread owns the registry handle.
            let _ = unsafe { RegCloseKey(key) };
        });
}

pub fn update_language(app: &AppHandle, language: &str) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(make_menu(app, language)?))?;
    }
    Ok(())
}

fn make_menu(app: &AppHandle, language: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let resources: serde_json::Value =
        serde_json::from_str(locale_resource(language)).unwrap_or_default();
    let labels = &resources["tray"];
    let label = |key: &str, default: &'static str| labels[key].as_str().unwrap_or(default);
    let toggle = MenuItem::with_id(app, "toggle", label("toggle", "Show / Hide"), true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", label("settings", "Settings..."), true, None::<&str>)?;
    let install = MenuItem::with_id(app, "install_hooks", label("install", "Install hooks"), true, None::<&str>)?;
    let export = MenuItem::with_id(
        app, "export_diagnostics", label("diagnostics", "Export Diagnostics..."), true, None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", label("quit", "Quit"), true, None::<&str>)?;
    Menu::with_items(app, &[&toggle, &settings, &install, &export, &quit])
}

fn locale_resource(setting: &str) -> &'static str {
    let locale = if setting == "system" || setting.is_empty() {
        system_language()
    } else {
        setting.to_ascii_lowercase()
    };
    match locale.split(['-', '_']).next().unwrap_or("") {
        "zh" if ["hant", "tw", "hk", "mo"].iter().any(|&part| locale.contains(part)) => {
            include_str!("../../src/i18n/zh-TW.json")
        }
        "zh" => include_str!("../../src/i18n/zh-CN.json"),
        "de" => include_str!("../../src/i18n/de-DE.json"),
        "ja" => include_str!("../../src/i18n/ja-JP.json"),
        "ko" => include_str!("../../src/i18n/ko-KR.json"),
        "tr" => include_str!("../../src/i18n/tr-TR.json"),
        _ => include_str!("../../src/i18n/en-US.json"),
    }
}

#[cfg(windows)]
fn system_language() -> String {
    use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};

    let mut name = [0u16; 85];
    // SAFETY: the locale name buffer is the full Windows locale-name size.
    let length = unsafe {
        LCIDToLocaleName(u32::from(GetUserDefaultUILanguage()), Some(&mut name), 0)
    };
    let written = length.saturating_sub(1).max(0) as usize;
    String::from_utf16_lossy(name.get(..written).unwrap_or_default()).to_ascii_lowercase()
}

#[cfg(not(windows))]
fn system_language() -> String {
    std::env::var("LANG").unwrap_or_default().to_ascii_lowercase()
}
