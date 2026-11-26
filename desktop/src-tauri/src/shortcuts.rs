use codeisland_domain::{parse_shortcut, AppSettings};
use codeisland_runtime::{CardShortcut, CardShortcutOutcome, IslandSurface};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};


#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConflict {
    pub action: String,
    pub shortcut: String,
    pub error: String,
}

#[derive(Clone, Copy)]
enum ShortcutAction {
    Toggle,
    Card(CardShortcut),
    JumpToTerminal,
}

fn bindings(settings: &AppSettings) -> impl Iterator<Item = (&'static str, &str, ShortcutAction)> {
    [
        ("toggle", settings.toggle_shortcut.as_str(), settings.toggle_shortcut_enabled, ShortcutAction::Toggle),
        ("approve", settings.approve_shortcut.as_str(), settings.approve_shortcut_enabled, ShortcutAction::Card(CardShortcut::Approve)),
        ("approve_always", settings.approve_always_shortcut.as_str(), settings.approve_always_shortcut_enabled, ShortcutAction::Card(CardShortcut::AlwaysAllow)),
        ("deny", settings.deny_shortcut.as_str(), settings.deny_shortcut_enabled, ShortcutAction::Card(CardShortcut::Deny)),
        ("skip_question", settings.skip_question_shortcut.as_str(), settings.skip_question_shortcut_enabled, ShortcutAction::Card(CardShortcut::SkipQuestion)),
        ("jump_to_terminal", settings.jump_to_terminal_shortcut.as_str(), settings.jump_to_terminal_shortcut_enabled, ShortcutAction::JumpToTerminal),
    ]
    .into_iter()
    .filter_map(|(name, binding, enabled, action)| enabled.then_some((name, binding, action)))
}

pub(crate) fn settings_changed(before: &AppSettings, after: &AppSettings) -> bool {
    bindings(before).map(|(action, key, _)| (action, key))
        .ne(bindings(after).map(|(action, key, _)| (action, key)))
}

/// Parses a settings shortcut string into a plugin shortcut. The domain
/// parser normalizes modifiers and key names; we translate the names into the
/// plugin's `Code`/`Modifiers` types.
pub fn shortcut_from_settings(value: &str) -> Option<Shortcut> {
    let parts = parse_shortcut(value)?;
    let mut modifiers = Modifiers::empty();
    let mut key = None;

    for part in parts {
        match part.as_str() {
            "Ctrl" => modifiers |= Modifiers::CONTROL,
            "Shift" => modifiers |= Modifiers::SHIFT,
            "Alt" => modifiers |= Modifiers::ALT,
            "Meta" => modifiers |= Modifiers::SUPER,
            other => {
                key = code_for(other);
                if key.is_none() {
                    return None;
                }
            }
        }
    }

    let key = key?;
    Some(Shortcut::new(
        if modifiers.is_empty() {
            None
        } else {
            Some(modifiers)
        },
        key,
    ))
}

/// Replaces the registered shortcuts with the ones from `settings`.
/// Conflicts (shortcut already owned by another app, invalid strings, or
/// duplicates within the settings) are reported and surfaced as a toast.
pub fn register_shortcuts(app: &AppHandle, settings: &AppSettings) -> Vec<ShortcutConflict> {
    let manager = app.global_shortcut();
    if let Err(error) = manager.unregister_all() {
        tracing::warn!(%error, "failed to unregister global shortcuts");
    }
    if app
        .try_state::<crate::AppState>()
        .is_some_and(|state| state.runtime.is_shortcut_recording())
    {
        return Vec::new();
    }

    let mut conflicts = Vec::new();
    for (action, value, _) in bindings(settings) {
        let Some(shortcut) = shortcut_from_settings(value) else {
            conflicts.push(ShortcutConflict {
                action: action.to_owned(),
                shortcut: value.to_owned(),
                error: "invalid shortcut".to_owned(),
            });
            continue;
        };

        if let Err(error) = manager.register(shortcut) {
            conflicts.push(ShortcutConflict {
                action: action.to_owned(),
                shortcut: value.to_owned(),
                error: error.to_string(),
            });
        }
    }

    for conflict in &conflicts {
        tracing::warn!(
            action = %conflict.action,
            shortcut = %conflict.shortcut,
            error = %conflict.error,
            "global shortcut unavailable"
        );
    }

    conflicts
}

/// Global shortcut handler shared by every registered accelerator.
pub fn handle_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state != ShortcutState::Pressed {
        return;
    }

    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    let settings = state.runtime.settings();

    let Some((_, _, action)) = bindings(&settings).find(|(_, binding, _)| {
        shortcut_from_settings(binding).is_some_and(|registered| registered == *shortcut)
    }) else {
        return;
    };
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || execute_shortcut(&handle, action)) {
        tracing::warn!(%error, "failed to dispatch island shortcut");
    }
}

fn execute_shortcut(app: &AppHandle, action: ShortcutAction) {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    if state.runtime.is_shortcut_recording() {
        return;
    }
    let on_screen = app
        .get_webview_window("main")
        .is_some_and(|window| is_window_on_screen(&window));

    match action {
        ShortcutAction::Toggle => {
            let current = state.runtime.surface();
            let surface = if current.is_collapsed() {
                IslandSurface::SessionList
            } else if crate::window::is_island_away(app) {
                current
            } else {
                IslandSurface::Collapsed
            };
            let reveal = !surface.is_collapsed();
            if crate::commands::set_island_surface_impl(app, state.inner(), surface) && reveal {
                crate::window::bring_island_into_sight(app);
            }
        }
        ShortcutAction::Card(shortcut) => {
            if let CardShortcutOutcome::Opened(surface) =
                state.runtime.perform_card_shortcut(shortcut, on_screen)
                && crate::commands::set_island_surface_impl(app, state.inner(), surface)
            {
                crate::window::bring_island_into_sight(app);
            }
        }
        ShortcutAction::JumpToTerminal => {
            let ui = state.runtime.state();
            let current = ui.surface.session_id().or(ui.store.current_session_id.as_deref());
            if let Some(session) = ui.store.sessions.into_iter().find(|s| Some(s.session_id.as_str()) == current) {
                let runtime = state.runtime.clone();
                tauri::async_runtime::spawn(async move {
                    if crate::workspace::launch_for_session(&session).await {
                        runtime.user_jumped_to_session(&session.session_id);
                    }
                });
            }
        }
    }
}

pub(crate) fn set_recording(app: &AppHandle, recording: bool) {
    if let Some(state) = app.try_state::<crate::AppState>()
        && state.runtime.set_shortcut_recording(recording)
    {
        register_shortcuts(app, &state.runtime.settings());
    }
}

pub(crate) fn stop_shortcut_recording(app: &AppHandle) {
    set_recording(app, false);
}

pub(crate) fn is_window_on_screen(window: &tauri::WebviewWindow) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};

        // SAFETY: the handle belongs to the live webview window.
        window.hwnd().is_ok_and(|hwnd| unsafe {
            IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool()
        })
    }
    #[cfg(not(windows))]
    {
        window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(true)
    }
}

fn code_for(key: &str) -> Option<Code> {
    let upper = key.to_ascii_uppercase();
    Some(match upper.as_str() {
        "A" => Code::KeyA,
        "B" => Code::KeyB,
        "C" => Code::KeyC,
        "D" => Code::KeyD,
        "E" => Code::KeyE,
        "F" => Code::KeyF,
        "G" => Code::KeyG,
        "H" => Code::KeyH,
        "I" => Code::KeyI,
        "J" => Code::KeyJ,
        "K" => Code::KeyK,
        "L" => Code::KeyL,
        "M" => Code::KeyM,
        "N" => Code::KeyN,
        "O" => Code::KeyO,
        "P" => Code::KeyP,
        "Q" => Code::KeyQ,
        "R" => Code::KeyR,
        "S" => Code::KeyS,
        "T" => Code::KeyT,
        "U" => Code::KeyU,
        "V" => Code::KeyV,
        "W" => Code::KeyW,
        "X" => Code::KeyX,
        "Y" => Code::KeyY,
        "Z" => Code::KeyZ,
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "F13" => Code::F13,
        "F14" => Code::F14,
        "F15" => Code::F15,
        "F16" => Code::F16,
        "F17" => Code::F17,
        "F18" => Code::F18,
        "F19" => Code::F19,
        "F20" => Code::F20,
        "F21" => Code::F21,
        "F22" => Code::F22,
        "F23" => Code::F23,
        "F24" => Code::F24,
        "ESCAPE" => Code::Escape,
        "ENTER" => Code::Enter,
        "SPACE" => Code::Space,
        "TAB" => Code::Tab,
        "BACKSPACE" => Code::Backspace,
        "DELETE" => Code::Delete,
        "INSERT" => Code::Insert,
        "HOME" => Code::Home,
        "END" => Code::End,
        "PAGEUP" => Code::PageUp,
        "PAGEDOWN" => Code::PageDown,
        "ARROWUP" | "UP" => Code::ArrowUp,
        "ARROWDOWN" | "DOWN" => Code::ArrowDown,
        "ARROWLEFT" | "LEFT" => Code::ArrowLeft,
        "ARROWRIGHT" | "RIGHT" => Code::ArrowRight,
        "MINUS" | "-" => Code::Minus,
        "EQUAL" | "=" => Code::Equal,
        "COMMA" | "," => Code::Comma,
        "PERIOD" | "." => Code::Period,
        "SLASH" | "/" => Code::Slash,
        "SEMICOLON" | ";" => Code::Semicolon,
        "QUOTE" | "'" => Code::Quote,
        "BACKQUOTE" | "`" => Code::Backquote,
        "BRACKETLEFT" | "[" => Code::BracketLeft,
        "BRACKETRIGHT" | "]" => Code::BracketRight,
        "BACKSLASH" | "\\" => Code::Backslash,
        _ => return None,
    })
}
