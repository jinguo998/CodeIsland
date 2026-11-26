use std::time::Duration;

use codeisland_domain::AppSettings;
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

pub const ISLAND_WIDTH: f64 = 780.0;
pub const ISLAND_HEIGHT: f64 = 500.0;

/// One connected display, shaped for the settings page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    /// Monitor interface identity, with the display device name as a fallback.
    pub id: String,
    pub legacy_id: String,
    pub name: String,
    pub is_primary: bool,
    pub width: u32,
    pub height: u32,
}

/// The work area and DPI scale of the display the island should live on.
#[derive(Debug, Clone, Copy)]
struct TargetMonitor {
    work_area: PhysicalRect<i32, u32>,
    scale: f64,
    width: f64,
}

/// Returns the existing island window (declared in `tauri.conf.json`) or
/// creates it when running without a configured window.
pub fn create_main_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window("main") {
        return Ok(window);
    }

    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("CodeIsland")
        .inner_size(ISLAND_WIDTH, ISLAND_HEIGHT)
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .visible(false)
        .focused(false)
        .build()
}

/// Applies Windows styles and restores the saved panel position and size.
pub fn prepare_main_window(app: &AppHandle, settings: &AppSettings) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };

    // The window stays at the expanded size so hover can animate the shell
    // with CSS instead of resizing WebView2 every frame (which blocks on
    // layout and made the island open in two visible steps).
    restore_position(&window, settings)?;
    win32::apply_island_styles(&window);
    win32::set_click_through(&window, settings.collapsed)
}

#[derive(Default)]
pub(crate) struct IslandVisibility {
    requested: bool,
    fullscreen: bool,
    revealed_for_shortcut: bool,
    compact_interactive: bool,
}

pub fn show_island(app: &AppHandle) {
    set_requested_visibility(app, true, false);
}

pub fn hide_island(app: &AppHandle) {
    set_requested_visibility(app, false, false);
}

pub(crate) fn bring_island_into_sight(app: &AppHandle) {
    set_requested_visibility(app, true, true);
}

pub(crate) fn is_island_away(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return true;
    };
    if !window.is_visible().unwrap_or(false) {
        return true;
    }
    #[cfg(windows)]
    if !window.is_focused().unwrap_or(false)
        && let Some((_, active_display)) = win32::select_monitor("auto", None)
        && let Some(island_display) = win32::window_monitor_device(&window)
    {
        return !active_display.eq_ignore_ascii_case(&island_display);
    }
    false
}

fn set_requested_visibility(app: &AppHandle, requested: bool, reveal: bool) {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    if let Ok(mut visibility) = state.island_visibility.lock() {
        visibility.requested = requested;
        visibility.revealed_for_shortcut = reveal;
    }
    if requested && let Some(window) = app.get_webview_window("main") {
        let mut settings = state.runtime.settings();
        if reveal && !window.is_focused().unwrap_or(false) {
            settings.display_choice = "auto".into();
            settings.display_mode = "auto".into();
            settings.allow_horizontal_drag = false;
        }
        let _ = restore_position(&window, &settings);
    }
    sync_island(app);
}

pub(crate) fn sync_island(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    let settings = state.runtime.settings();
    let idle = !state.runtime.has_sessions();
    let inactive = settings.hide_when_no_session && !state.runtime.has_active_sessions();
    let Ok(mut visibility) = state.island_visibility.lock() else {
        return;
    };
    if settings.collapsed {
        visibility.revealed_for_shortcut = false;
    }
    let visible = visibility.requested
        && !inactive
        && !(settings.hide_in_fullscreen
            && visibility.fullscreen
            && !visibility.revealed_for_shortcut);
    let click_through = (idle || settings.collapsed) && !visibility.compact_interactive;
    drop(visibility);

    if !visible {
        win32::hide(&window);
        let _ = window.emit("island://visibility", false);
        return;
    }
    if let Err(error) = win32::set_click_through(&window, click_through) {
        tracing::warn!(%error, "failed to set island click-through");
    }
    if !window.is_visible().unwrap_or(false) {
        win32::show_no_activate(&window);
    }
    let _ = window.emit("island://visibility", true);
}

pub(crate) fn set_compact_interactive(app: &AppHandle, interactive: bool) {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    if let Ok(mut visibility) = state.island_visibility.lock() {
        visibility.compact_interactive = interactive;
    }
    sync_island(app);
}

pub fn toggle_island(app: &AppHandle) {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    let requested = state.island_visibility.lock().is_ok_and(|visibility| visibility.requested);
    set_requested_visibility(app, !requested, false);
}

/// The original launch acknowledgment opens briefly after the shell is ready.
pub fn show_island_when_ready(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            show_island(&handle);
            let Some(state) = handle.try_state::<crate::AppState>() else {
                return;
            };
            if !state.runtime.surface().is_collapsed() {
                return;
            }
            let next_revision = state.runtime.state().surface_revision.wrapping_add(1);
            let _ = handle.emit_to("main", "island://launch-pop", next_revision);
            if !crate::commands::set_island_surface_impl(
                &handle,
                state.inner(),
                codeisland_runtime::IslandSurface::SessionList,
            ) {
                return;
            }

            let close_app = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let handle = close_app.clone();
                let _ = close_app.run_on_main_thread(move || {
                    if let Some(state) = handle.try_state::<crate::AppState>()
                        && state.runtime.surface() == codeisland_runtime::IslandSurface::SessionList
                    {
                        crate::commands::set_island_surface_impl(
                            &handle,
                            state.inner(),
                            codeisland_runtime::IslandSurface::Collapsed,
                        );
                    }
                });
            });
        });
    });
}

pub fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("settings") {
        if window.is_minimized()? {
            window.unminimize()?;
        }
        window.show()?;
        return window.set_focus();
    }

    // Settings is created on demand. Show it during creation so WebView2
    // initializes against a visible HWND, and keep it above the island.
    WebviewWindowBuilder::new(
        app,
        "settings",
        WebviewUrl::App("index.html?window=settings".into()),
    )
    .title("CodeIsland Settings")
    .inner_size(860.0, 640.0)
    .min_inner_size(640.0, 480.0)
    .center()
    .resizable(true)
    .always_on_top(true)
    .build()?
    .set_focus()
}

/// Connected displays for the settings page. `id` is the device name stored
/// in `pinned_display`, `name` is the monitor's friendly name when Windows
/// reports one.
pub fn list_displays(app: &AppHandle) -> Vec<DisplayInfo> {
    #[cfg(windows)]
    {
        let _ = app;
        win32::list_displays()
    }
    #[cfg(not(windows))]
    {
        let Some(window) = app.get_webview_window("main") else {
            return Vec::new();
        };
        let primary = window.primary_monitor().ok().flatten();
        let primary_name = primary.as_ref().and_then(|monitor| monitor.name().cloned());
        window
            .available_monitors()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                let id = monitor
                    .name()
                    .cloned()
                    .unwrap_or_else(|| format!("display-{index}"));
                DisplayInfo {
                    is_primary: primary_name.as_deref() == Some(id.as_str()),
                    name: id.clone(),
                    legacy_id: id.clone(),
                    id,
                    width: monitor.size().width,
                    height: monitor.size().height,
                }
            })
            .collect()
    }
}

#[cfg(windows)]
pub(crate) fn drag_island_horizontally(hwnd: isize, x: i32) -> Option<(i32, i32)> {
    win32::drag_island_horizontally(hwnd, x)
}

/// Device name of the display currently hosting the island. Used to fill in
/// `pinned_display` the first time the user chooses the pinned mode.
pub fn monitor_device_name(window: &WebviewWindow) -> Option<String> {
    #[cfg(windows)]
    {
        win32::window_monitor_pinned_id(window)
    }
    #[cfg(not(windows))]
    {
        window
            .current_monitor()
            .ok()
            .flatten()
            .and_then(|monitor| monitor.name().cloned())
    }
}

/// Re-resolves the display when the foreground window or cursor moves across
/// monitors, and when the selected display's work area changes.
#[cfg(windows)]
pub fn spawn_display_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let mut previous = None;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let Some(state) = app.try_state::<crate::AppState>() else {
                continue;
            };
            if state.island_visibility.lock().is_ok_and(|visibility| visibility.revealed_for_shortcut) {
                previous = None;
                continue;
            }
            let settings = state.runtime.settings();
            let (mode, pinned) = display_selection(&settings);
            let next = win32::select_monitor(mode, pinned).map(|(area, device)| {
                (device, area.position.x, area.position.y, area.size.width, area.size.height)
            });
            if next == previous {
                continue;
            }
            previous = next;
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(window) = handle.get_webview_window("main") else {
                    return;
                };
                let Some(state) = handle.try_state::<crate::AppState>() else {
                    return;
                };
                let settings = state.runtime.settings();
                if let Err(error) = restore_position(&window, &settings) {
                    tracing::warn!(%error, "failed to re-position island after display change");
                }
            });
        }
    });
}

#[cfg(not(windows))]
pub fn spawn_display_watcher(_: AppHandle) {}

/// Hide in Fullscreen: poll the foreground window and take the island off
/// screen while a fullscreen window covers its monitor. The island comes back
/// when the fullscreen window goes away (or the setting is turned off).
pub fn spawn_fullscreen_watcher(app: AppHandle) {
    const POLL: Duration = Duration::from_millis(1500);
    std::thread::spawn(move || {
        let mut previous = false;
        loop {
            let fullscreen = foreground_is_fullscreen(&app);
            if fullscreen != previous {
                previous = fullscreen;
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    if let Some(state) = handle.try_state::<crate::AppState>() {
                        if let Ok(mut visibility) = state.island_visibility.lock() {
                            visibility.fullscreen = fullscreen;
                        }
                        sync_island(&handle);
                    }
                });
            }
            std::thread::sleep(POLL);
        }
    });
}

fn foreground_is_fullscreen(app: &AppHandle) -> bool {
    #[cfg(windows)]
    {
        win32::foreground_is_fullscreen(app)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        false
    }
}

/// Places the island on the work area of the display chosen by
/// `display_choice` (falling back to the primary display). Saved coordinates
/// apply while horizontal dragging is enabled.
pub fn restore_position(window: &WebviewWindow, settings: &AppSettings) -> tauri::Result<()> {
    let Some(target) = target_monitor(window, settings)? else {
        return Ok(());
    };
    let size = physical_island_size(target.scale);
    let previous_size = window.outer_size()?;
    let center = centered_x(&target.work_area, size);
    let offset = saved_horizontal_offset(window, settings).unwrap_or_default();
    // The Swift contour meets the top edge. The work area also accommodates
    // a Windows taskbar docked there.
    let (x, y) = clamp_to_work_area(
        center + (offset * target.scale).round() as i32,
        target.work_area.position.y,
        &target.work_area,
        size,
    );
    let position = PhysicalPosition::new(x, y);
    if window.outer_position()? != position {
        window.set_position(position)?;
    }
    if previous_size != size {
        window.set_size(size)?;
    }
    let _ = window.emit("island://display-width", target.width);
    Ok(())
}

/// Preserve the logical offset from screen center across monitor and DPI
/// changes while keeping compatibility with saved physical coordinates.
fn saved_horizontal_offset(window: &WebviewWindow, settings: &AppSettings) -> Option<f64> {
    let (x, y) = match (settings.allow_horizontal_drag, settings.panel_x, settings.panel_y) {
        (true, Some(x), Some(y)) => (x, y),
        _ => return None,
    };
    let monitors = window.available_monitors().ok()?;
    let source = monitors.iter().find(|monitor| {
        let left = f64::from(monitor.position().x);
        let top = f64::from(monitor.position().y);
        x >= left
            && x < left + f64::from(monitor.size().width)
            && y >= top
            && y < top + f64::from(monitor.size().height)
    })?;
    let scale = source.scale_factor();
    let source_size = physical_island_size(scale);
    Some((x - f64::from(centered_x(source.work_area(), source_size))) / scale)
}

fn physical_island_size(scale: f64) -> PhysicalSize<u32> {
    PhysicalSize::new(
        (ISLAND_WIDTH * scale).round() as u32,
        (ISLAND_HEIGHT * scale).round() as u32,
    )
}

fn current_monitor(window: &WebviewWindow) -> tauri::Result<Option<tauri::window::Monitor>> {
    Ok(window.current_monitor()?.or(window.primary_monitor()?))
}

/// Resolves the display the island should be on. `display_choice` is the new
/// selector: "auto", "cursor", "primary", or a monitor id from
/// `list_displays`. "auto" falls back to the legacy `display_mode` /
/// `pinned_display` pair so existing settings keep working.
fn display_selection(settings: &AppSettings) -> (&str, Option<&str>) {
    match settings.display_choice.as_str() {
        "auto" => match settings.display_mode.as_str() {
            "cursor" => ("cursor", None),
            "pinned" => ("pinned", settings.pinned_display.as_deref()),
            _ => ("auto", None),
        },
        "cursor" => ("cursor", None),
        "primary" => ("primary", None),
        "pinned" => ("pinned", settings.pinned_display.as_deref()),
        id => {
            // The frontend may tag a monitor id macOS-style ("display:<id>").
            let id = id.strip_prefix("display:").unwrap_or(id);
            ("pinned", Some(id))
        }
    }
}

/// Resolves the target monitor for the current display selection, with the
/// Tauri monitor as a fallback when Win32 enumeration comes back empty.
fn target_monitor(
    window: &WebviewWindow,
    settings: &AppSettings,
) -> tauri::Result<Option<TargetMonitor>> {
    #[cfg(windows)]
    {
        let (mode, pinned) = display_selection(settings);
        if let Some((work_area, device)) = win32::select_monitor(mode, pinned) {
            let monitor = window
                .available_monitors()
                .ok()
                .and_then(|monitors| {
                    monitors
                        .into_iter()
                        .find(|monitor| {
                            monitor
                                .name()
                                .is_some_and(|name| name.eq_ignore_ascii_case(&device))
                        })
                });
            let scale = monitor.as_ref().map(|monitor| monitor.scale_factor())
                .or_else(|| window.scale_factor().ok())
                .unwrap_or(1.0);
            let width = monitor.as_ref().map_or(work_area.size.width, |monitor| monitor.size().width);
            return Ok(Some(TargetMonitor { work_area, scale, width: f64::from(width) / scale }));
        }
    }

    #[cfg(not(windows))]
    let _ = settings;

    Ok(current_monitor(window)?.map(|monitor| TargetMonitor {
        work_area: *monitor.work_area(),
        scale: monitor.scale_factor(),
        width: f64::from(monitor.size().width) / monitor.scale_factor(),
    }))
}

fn centered_x(work_area: &PhysicalRect<i32, u32>, size: PhysicalSize<u32>) -> i32 {
    work_area.position.x + (work_area.size.width as i32 - size.width as i32) / 2
}

fn clamp_to_work_area(
    x: i32,
    y: i32,
    work_area: &PhysicalRect<i32, u32>,
    size: PhysicalSize<u32>,
) -> (i32, i32) {
    let min_x = work_area.position.x;
    let min_y = work_area.position.y;
    let max_x = min_x + work_area.size.width as i32 - size.width as i32;
    let max_y = min_y + work_area.size.height as i32 - size.height as i32;
    (
        x.clamp(min_x, max_x.max(min_x)),
        y.clamp(min_y, max_y.max(min_y)),
    )
}

/// All Win32 calls live here so unsafe code stays isolated and documented.
#[cfg(windows)]
mod win32 {
    use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewWindow};
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Foundation::{
        GetLastError, HWND, LPARAM, POINT, RECT, SetLastError, TRUE, WIN32_ERROR,
    };
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint,
        MonitorFromWindow, DISPLAY_DEVICEW, HDC, HMONITOR, MONITORINFOEXW,
        MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsZoomed, SetWindowLongPtrW, SetWindowPos,
        ShowWindow, EDD_GET_DEVICE_INTERFACE_NAME, GWL_EXSTYLE, HWND_TOPMOST,
        MONITORINFOF_PRIMARY, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT,
    };

    use super::DisplayInfo;

    #[derive(Clone, Debug)]
    struct MonitorRecord {
        handle: HMONITOR,
        /// Full monitor bounds (physical pixels).
        monitor: RECT,
        /// Work area, i.e. the monitor minus the taskbar.
        work: RECT,
        primary: bool,
        device: String,
    }

    fn hwnd_of(window: &WebviewWindow) -> Option<HWND> {
        window.hwnd().ok().map(|hwnd| HWND(hwnd.0))
    }

    fn wide_string(buf: &[u16]) -> String {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    fn rect_from(rect: RECT) -> PhysicalRect<i32, u32> {
        PhysicalRect {
            position: PhysicalPosition::new(rect.left, rect.top),
            size: PhysicalSize::new(
                (rect.right - rect.left).max(0) as u32,
                (rect.bottom - rect.top).max(0) as u32,
            ),
        }
    }

    /// Fills `MONITORINFOEXW` for one monitor. `GetMonitorInfoW` takes the
    /// base struct, so `cbSize` names the extended size and the base struct
    /// sits at the front of the extended one.
    fn monitor_record(monitor: HMONITOR) -> Option<MonitorRecord> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `GetMonitorInfoW` writes at most `cbSize` bytes into the
        // `MONITORINFO` that starts `info`; no Rust reference escapes.
        let ok = unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool();
        if !ok {
            return None;
        }
        Some(MonitorRecord {
            handle: monitor,
            monitor: info.monitorInfo.rcMonitor,
            work: info.monitorInfo.rcWork,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            device: wide_string(&info.szDevice),
        })
    }

    /// SAFETY: `data` is the `Vec<MonitorRecord>` pointer passed to
    /// `EnumDisplayMonitors`; the vector outlives the synchronous enumeration.
    unsafe extern "system" fn enum_monitor(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: see the function comment; the callback only pushes.
        let out = unsafe { &mut *(data.0 as *mut Vec<MonitorRecord>) };
        if let Some(record) = monitor_record(monitor) {
            out.push(record);
        }
        TRUE
    }

    fn monitors() -> Vec<MonitorRecord> {
        let mut out: Vec<MonitorRecord> = Vec::new();
        // SAFETY: `out` outlives this synchronous call and `EnumDisplayMonitors`
        // does not retain the pointer after it returns.
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(enum_monitor),
                LPARAM(&mut out as *mut Vec<MonitorRecord> as isize),
            );
        }
        out
    }

    /// The interface path identifies the connected monitor across changes to
    /// its transient DISPLAYn name. Some drivers only expose the latter.
    fn monitor_identity(device: &str) -> (String, String) {
        let wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
        let mut info = DISPLAY_DEVICEW::default();
        info.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // SAFETY: `wide` is NUL-terminated and `info.cb` describes `info`.
        let ok = unsafe {
            EnumDisplayDevicesW(
                PCWSTR(wide.as_ptr()),
                0,
                &mut info,
                EDD_GET_DEVICE_INTERFACE_NAME,
            )
        }
        .as_bool();
        let value = |buffer: &[u16]| {
            let value = if ok { wide_string(buffer) } else { String::new() };
            if value.trim().is_empty() { device.to_owned() } else { value }
        };
        (value(&info.DeviceID), value(&info.DeviceString))
    }

    pub fn list_displays() -> Vec<DisplayInfo> {
        monitors()
            .into_iter()
            .map(|record| {
                let (id, name) = monitor_identity(&record.device);
                let size = rect_from(record.monitor).size;
                DisplayInfo {
                    id,
                    legacy_id: record.device,
                    name,
                    is_primary: record.primary,
                    width: size.width,
                    height: size.height,
                }
            })
            .collect()
    }

    /// A disconnected pin retains its identity and temporarily follows Auto.
    pub fn select_monitor(
        mode: &str,
        pinned: Option<&str>,
    ) -> Option<(PhysicalRect<i32, u32>, String)> {
        let records = monitors();
        let primary = records
            .iter()
            .find(|record| record.primary)
            .or_else(|| {
                // SAFETY: `MonitorFromPoint` only reads the POINT value.
                let monitor = unsafe {
                    MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
                };
                records.iter().find(|record| record.handle == monitor)
            })
            .or_else(|| records.first())?;

        let auto = || {
            // SAFETY: these calls only read the current foreground window.
            let foreground = unsafe { GetForegroundWindow() };
            if foreground.is_invalid() {
                primary
            } else {
                let monitor = unsafe { MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST) };
                records.iter().find(|record| record.handle == monitor).unwrap_or(primary)
            }
        };
        let chosen = match mode {
            "cursor" => {
                let mut point = POINT::default();
                // SAFETY: both calls only read global cursor state and write
                // a stack POINT.
                let cursor = unsafe { GetCursorPos(&mut point) }.is_ok();
                if cursor {
                    // SAFETY: `MonitorFromPoint` only reads the POINT value.
                    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
                    records
                        .iter()
                        .find(|record| record.handle == monitor)
                        .unwrap_or(primary)
                } else {
                    primary
                }
            }
            "pinned" => pinned
                .and_then(|name| {
                    let name = name.strip_prefix("display:").unwrap_or(name);
                    if let Ok(index) = name.strip_prefix("screen_").unwrap_or(name).parse::<usize>() {
                        return records.get(index);
                    }
                    records.iter().find(|record| {
                        let (id, friendly) = monitor_identity(&record.device);
                        record.device.eq_ignore_ascii_case(name)
                            || id.eq_ignore_ascii_case(name)
                            || friendly.eq_ignore_ascii_case(name)
                    })
                })
                .unwrap_or_else(auto),
            "auto" => auto(),
            _ => primary,
        };

        Some((rect_from(chosen.work), chosen.device.clone()))
    }

    pub fn window_monitor_device(window: &WebviewWindow) -> Option<String> {
        let hwnd = hwnd_of(window)?;
        // SAFETY: `hwnd` is a live window; `MonitorFromWindow` only reads it.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        monitor_record(monitor).map(|record| record.device)
    }

    pub fn window_monitor_pinned_id(window: &WebviewWindow) -> Option<String> {
        window_monitor_device(window).map(|device| monitor_identity(&device).0)
    }

    pub fn drag_island_horizontally(hwnd: isize, x: i32) -> Option<(i32, i32)> {
        let hwnd = HWND(hwnd as *mut _);
        let mut bounds = RECT::default();
        // SAFETY: hwnd is the live island window, and bounds is a stack RECT.
        unsafe { GetWindowRect(hwnd, &mut bounds).ok()? };
        let work = monitor_record(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })?.work;
        let max_x = (work.right - (bounds.right - bounds.left)).max(work.left);
        let x = x.clamp(work.left, max_x);
        let y = work.top;
        if x != bounds.left || y != bounds.top {
            // SAFETY: the move keeps the island's existing size, z-order and activation.
            unsafe {
                SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)
                    .ok()?;
            }
        }
        Some((x, y))
    }

    /// Whether the foreground window is a fullscreen window on the island's
    /// own monitor. Maximized windows (work area, not full bounds), the
    /// island/settings, and the shell's desktop windows never count.
    pub fn foreground_is_fullscreen(app: &AppHandle) -> bool {
        let Some(island) = app.get_webview_window("main").and_then(|window| window.hwnd().ok())
        else {
            return false;
        };

        // SAFETY: all calls only read window/process state; handles come from
        // the OS and no Rust-owned memory is passed in.
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground.is_invalid() || foreground == island {
                return false;
            }

            let mut pid = 0u32;
            GetWindowThreadProcessId(foreground, Some(&mut pid as *mut u32));
            if pid == std::process::id() {
                // The island or the settings window.
                return false;
            }

            let mut class = [0u16; 64];
            let len = GetClassNameW(foreground, &mut class);
            if len > 0 {
                let class = wide_string(&class[..len as usize]);
                if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
                    return false;
                }
            }

            if IsZoomed(foreground).as_bool() {
                // A maximized window covers the work area, not the monitor.
                return false;
            }

            let mut rect = RECT::default();
            if GetWindowRect(foreground, &mut rect).is_err() {
                return false;
            }

            let foreground_monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
            let island_monitor = MonitorFromWindow(island, MONITOR_DEFAULTTONEAREST);
            if foreground_monitor != island_monitor {
                // A fullscreen app on another display leaves the island alone.
                return false;
            }

            let Some(info) = monitor_record(foreground_monitor) else {
                return false;
            };
            let bounds = info.monitor;
            rect.left <= bounds.left
                && rect.top <= bounds.top
                && rect.right >= bounds.right
                && rect.bottom >= bounds.bottom
        }
    }

    /// The island must never steal focus or appear in Alt+Tab / the taskbar.
    pub fn apply_island_styles(window: &WebviewWindow) {
        let Some(hwnd) = hwnd_of(window) else {
            return;
        };

        // SAFETY: `hwnd` came from the live Tauri window and the style flags
        // are plain window attributes; no memory is dereferenced.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            // Keep the island out of Alt+Tab but allow a click to focus it,
            // so question cards can be typed into. Showing it still uses
            // SW_SHOWNOACTIVATE, so launch never steals focus.
            let style = WINDOW_EX_STYLE(style as u32 | WS_EX_TOOLWINDOW.0);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style.0 as isize);

            // Keep the island above normal windows without activating it.
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    /// Update only the cursor styles. The island is shown directly with Win32
    /// to avoid activation, so Tao still tracks it as hidden and would hide
    /// it again when applying a change through `set_ignore_cursor_events`.
    pub fn set_click_through(window: &WebviewWindow, enabled: bool) -> tauri::Result<()> {
        let hwnd = window.hwnd()?;
        let flags = (WS_EX_TRANSPARENT | WS_EX_LAYERED).0 as isize;
        // SAFETY: hwnd is a live window handle. These calls read and update
        // its extended styles without changing its visibility or focus.
        unsafe {
            let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let style = if enabled { current | flags } else { current & !flags };
            if style != current {
                SetLastError(WIN32_ERROR(0));
                let previous = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
                if previous == 0 && GetLastError() != WIN32_ERROR(0) {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
        }
        Ok(())
    }

    pub fn hide(window: &WebviewWindow) {
        if let Some(hwnd) = hwnd_of(window) {
            // SAFETY: hwnd is a live window handle owned by this process.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    /// Shows the island without giving it foreground focus.
    pub fn show_no_activate(window: &WebviewWindow) {
        let Some(hwnd) = hwnd_of(window) else {
            return;
        };

        // SAFETY: `hwnd` is a live window handle; both calls only manipulate
        // visibility/z-order and never touch Rust-owned memory.
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

#[cfg(not(windows))]
mod win32 {
    use tauri::WebviewWindow;

    pub fn apply_island_styles(_window: &WebviewWindow) {}

    pub fn set_click_through(window: &WebviewWindow, enabled: bool) -> tauri::Result<()> {
        window.set_ignore_cursor_events(enabled)
    }

    pub fn hide(window: &WebviewWindow) {
        let _ = window.hide();
    }

    pub fn show_no_activate(window: &WebviewWindow) {
        let _ = window.show();
    }
}
