//! Cursor hover, outside-click and foreground tracking for the island.
//!
//! WebView2 hover events are unreliable for a tool window shown without
//! activation, so the macOS hover flow is reproduced by polling the cursor
//! against the visible shell bounds. The native poll owns the dwell and sends
//! pointer edges to React for prehover feedback.
//! The same thread collapses the expanded island when a click lands outside
//! it, and keeps the runtime's Smart Suppress flag in step with the
//! foreground window.

use std::time::{Duration, Instant};

use codeisland_runtime::IslandSurface;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect, IsWindowVisible};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const COLLAPSE_DELAY: Duration = Duration::from_millis(500);
/// How often the foreground window is checked (~500 ms at the poll interval).
const FRONT_TICKS: u32 = 10;
const DRAG_THRESHOLD: f64 = 5.0;

struct HorizontalDrag {
    mouse_x: i32,
    window_x: i32,
    threshold: i32,
    folded: bool,
    position: Option<(i32, i32)>,
}

pub fn spawn(app: AppHandle) {
    let Some(hwnd) = app.get_webview_window("main").and_then(|window| window.hwnd().ok()) else {
        return;
    };
    // The main HWND stays alive until shutdown. Its close action hides it.
    let hwnd = hwnd.0 as isize;
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd as *mut _);
        let mut inside = false;
        let mut prehover = false;
        let mut idle_hovered = false;
        let mut idle_leave = None;
        let mut compact_interactive = false;
        let mut drag: Option<HorizontalDrag> = None;
        let mut pending_hover = None;
        let mut button_was_down = (false, false);
        let mut terminal_front = None;
        let mut effective_front = None;
        let mut ticks = 0u32;

        loop {
            std::thread::sleep(POLL_INTERVAL);

            let (pressed, left_held) = mouse_buttons_down();
            let left_click = pressed.0 && !button_was_down.0;
            let right_click = pressed.1 && !button_was_down.1;
            button_was_down = pressed;

            let Some(state) = app.try_state::<crate::AppState>() else {
                continue;
            };
            let settings = state.runtime.settings();

            // Keep attention in step with the active terminal even when the
            // island is hidden.
            if ticks % FRONT_TICKS == 0 {
                let in_front = state.runtime.current_session_in_front();
                let effective = settings.smart_suppress && in_front;
                if terminal_front != Some(in_front) {
                    terminal_front = Some(in_front);
                    let _ = app.emit("island://front", json!({ "inFront": in_front }));
                }
                if effective_front != Some(effective) {
                    state.runtime.set_session_in_front(effective);
                    effective_front = Some(effective);
                }
            }
            ticks = ticks.wrapping_add(1);

            // SAFETY: The main HWND is owned by this process.
            if !unsafe { IsWindowVisible(hwnd).as_bool() } {
                if inside {
                    state.runtime.set_pointer_over_island(false);
                }
                if prehover || idle_hovered {
                    emit_hover(&app, false, false);
                }
                if compact_interactive {
                    set_compact_interactive(&app, false);
                }
                inside = false;
                prehover = false;
                idle_hovered = false;
                idle_leave = None;
                compact_interactive = false;
                drag = None;
                pending_hover = None;
                continue;
            }

            let idle = !state.runtime.has_sessions();
            let surface = state.runtime.surface();
            let prehover_before_surface = prehover;
            if !surface.is_collapsed() {
                prehover = false;
            }
            let folded = idle || surface.is_collapsed();
            let size = if folded {
                state.hover_strip_size.lock().ok().and_then(|size| *size)
                    .or(Some((306, 25)))
                    .map(|(w, h)| (w + if prehover { 8 } else { 0 }, h))
            } else {
                state.hover_panel_size.lock().ok().and_then(|size| *size)
            };
            let Some(sample) = sample_cursor(hwnd, size) else {
                continue;
            };
            let now_inside = sample.inside;
            let now = Instant::now();
            let mut changed = prehover != prehover_before_surface;
            let mut target = None;

            if !settings.allow_horizontal_drag {
                drag = None;
            }
            // Use the center of the top bar. Its edge buttons and the
            // session list's reorder grips keep their normal pointer behavior.
            let notch_height = if settings.notch_height_mode == "custom" {
                settings.custom_notch_height
            } else {
                25.0
            };
            let header_bottom = sample.window_top + (notch_height * sample.scale).round() as i32;
            if settings.allow_horizontal_drag
                && left_click
                && left_held
                && now_inside
                && sample.position.y < header_bottom
                && sample.in_drag_region()
            {
                drag = Some(HorizontalDrag {
                    mouse_x: sample.position.x,
                    window_x: sample.window_left,
                    threshold: (DRAG_THRESHOLD * sample.scale).round() as i32,
                    folded,
                    position: None,
                });
            }
            let released_drag = if left_held { None } else { drag.take() };
            if let Some(drag) = drag.as_mut() {
                let delta = sample.position.x - drag.mouse_x;
                if (drag.position.is_some() || delta.abs() > drag.threshold)
                    && let Some(position) = crate::window::drag_island_horizontally(
                        hwnd.0 as isize, drag.window_x + delta,
                    )
                {
                    drag.position = Some(position);
                }
            }
            if let Some((x, y)) = released_drag.as_ref().and_then(|drag| drag.position)
                && let Err(error) = state.runtime.try_update_settings(|current| {
                    let mut settings = current.clone();
                    if settings.allow_horizontal_drag {
                        settings.panel_x = Some(f64::from(x));
                        settings.panel_y = Some(f64::from(y));
                    }
                    Ok(settings)
                })
            {
                tracing::warn!(%error, "failed to save the island drag position");
            }

            if drag.is_some() {
                pending_hover = None;
                if prehover {
                    prehover = false;
                    changed = true;
                }
            } else if idle {
                pending_hover = None;
                if prehover {
                    prehover = false;
                    changed = true;
                }
                if now_inside {
                    idle_leave = None;
                    if !idle_hovered {
                        idle_hovered = true;
                        changed = true;
                    }
                } else if inside && idle_hovered {
                    idle_leave = Some(now + Duration::from_millis(300));
                } else if idle_leave.is_some_and(|deadline| now >= deadline) {
                    idle_leave = None;
                    idle_hovered = false;
                    changed = true;
                }
            } else {
                if idle_hovered {
                    idle_hovered = false;
                    idle_leave = None;
                    changed = true;
                }
                if inside != now_inside {
                    let prehover_before = prehover;
                    prehover = now_inside
                        && surface.is_collapsed()
                        && !(settings.smart_suppress && state.runtime.current_session_in_front());
                    changed |= prehover != prehover_before;
                    pending_hover = match (&surface, now_inside, prehover) {
                        (IslandSurface::Collapsed, true, true) => Some((
                            now,
                            surface.clone(),
                            IslandSurface::SessionList,
                        )),
                        (IslandSurface::SessionList, false, _) if settings.collapse_on_mouse_leave => {
                            Some((now, surface.clone(), IslandSurface::Collapsed))
                        }
                        _ => None,
                    };
                    if !now_inside && matches!(surface, IslandSurface::CompletionCard { .. }) {
                        target = Some(IslandSurface::Collapsed);
                    }
                }
            }

            if now_inside != inside {
                inside = now_inside;
                state.runtime.set_pointer_over_island(inside);
            }
            if let Some((started, expected, destination)) = &pending_hover {
                let opening = matches!(destination, IslandSurface::SessionList);
                let delay = if opening {
                    Duration::from_millis(u64::from(settings.hover_expand_delay_ms))
                } else {
                    COLLAPSE_DELAY
                };
                let suppressed = opening
                    && settings.smart_suppress
                    && now.duration_since(*started) >= delay
                    && state.runtime.current_session_in_front();
                if *expected != surface || suppressed || (!opening && !settings.collapse_on_mouse_leave) {
                    pending_hover = None;
                    if prehover {
                        prehover = false;
                        changed = true;
                    }
                } else if now.duration_since(*started) >= delay {
                    target = Some(destination.clone());
                    pending_hover = None;
                }
            }
            if changed {
                emit_hover(&app, prehover, idle_hovered);
            }
            let interactive = folded
                && (drag.is_some()
                    || (now_inside && ((idle && idle_hovered)
                        || (settings.allow_horizontal_drag && sample.in_drag_region()))));
            if interactive != compact_interactive {
                compact_interactive = interactive;
                set_compact_interactive(&app, interactive);
            }

            // The compact bar handles a left click only when a question is
            // waiting. Outside clicks leave approval and question cards open.
            let compact_click = (left_click && drag.is_none())
                || released_drag.is_some_and(|drag| drag.folded && drag.position.is_none());
            if compact_click && now_inside && !idle && surface.is_collapsed() {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    if let Some(state) = handle.try_state::<crate::AppState>()
                        && state.runtime.surface().is_collapsed()
                    {
                        crate::commands::open_pending_question_impl(&handle, state.inner(), None);
                    }
                });
            }
            if drag.is_none()
                && (left_click || right_click)
                && !now_inside
                && matches!(surface, IslandSurface::SessionList | IslandSurface::CompletionCard { .. })
            {
                target = Some(IslandSurface::Collapsed);
            }
            if let Some(target) = target {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    if let Some(state) = handle.try_state::<crate::AppState>()
                        && state.runtime.surface() == surface
                    {
                        crate::commands::set_island_surface_impl(&handle, state.inner(), target);
                    }
                });
            }
        }
    });
}

fn emit_hover(app: &AppHandle, prehover: bool, idle_hovered: bool) {
    let _ = app.emit(
        "island://hover",
        json!({ "inside": prehover, "idleHovered": idle_hovered }),
    );
}

fn set_compact_interactive(app: &AppHandle, interactive: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        crate::window::set_compact_interactive(&handle, interactive);
    });
}

struct CursorSample {
    position: POINT,
    window_left: i32,
    window_top: i32,
    shell_left: i32,
    shell_right: i32,
    scale: f64,
    inside: bool,
}

impl CursorSample {
    fn in_drag_region(&self) -> bool {
        let width = self.shell_right - self.shell_left;
        let x = self.position.x - self.shell_left;
        x >= width * 2 / 5 && x < width * 3 / 5
    }
}

/// The webview centers the shell at the top of the fixed native window.
fn sample_cursor(hwnd: HWND, shell_size: Option<(u32, u32)>) -> Option<CursorSample> {
    let mut rect = RECT::default();
    let mut point = POINT::default();
    // SAFETY: both calls only write into stack values owned here.
    unsafe {
        GetWindowRect(hwnd, &mut rect).ok()?;
        GetCursorPos(&mut point).ok()?;
    }

    // SAFETY: hwnd belongs to the main island.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
    let (left, right, bottom) = if let Some((width, height)) = shell_size {
        let window_width = rect.right - rect.left;
        let width = ((f64::from(width) * scale).round() as i32).min(window_width);
        let height = ((f64::from(height) * scale).round() as i32).min(rect.bottom - rect.top);
        let left = rect.left + (window_width - width) / 2;
        (left, left + width, rect.top + height)
    } else {
        (rect.left, rect.right, rect.bottom)
    };

    Some(CursorSample {
        position: point,
        window_left: rect.left,
        window_top: rect.top,
        shell_left: left,
        shell_right: right,
        scale,
        inside: point.x >= left && point.x < right && point.y >= rect.top && point.y < bottom,
    })
}

fn mouse_buttons_down() -> ((bool, bool), bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};

    // SAFETY: GetAsyncKeyState only reads global key state. Bit 15 is "down
    // now"; bit 0 is "pressed since the previous call", which catches a click
    // shorter than the poll interval.
    unsafe {
        let left = GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16;
        let right = GetAsyncKeyState(VK_RBUTTON.0 as i32) as u16;
        ((left & 0x8001 != 0, right & 0x8001 != 0), left & 0x8000 != 0)
    }
}

