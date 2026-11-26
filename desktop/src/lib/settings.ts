import {
  clampCustomNotchHeight,
  clampNotchAnimationSpeed,
  COLLAPSED_WIDTH_SCALE_MAX,
  COLLAPSED_WIDTH_SCALE_MIN,
  UNLIMITED_VISIBLE_SESSIONS,
  normalizeRotationInterval,
} from "./defaults";
import type { AppSettings } from "./types";

export function clamp(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

/**
 * Client-side mirror of `AppSettings::validate()` so the form never submits
 * values the backend would silently replace.
 */
export function clampSettings(settings: AppSettings): AppSettings {
  const oneOf = (value: string, allowed: readonly string[], fallback: string) =>
    allowed.includes(value) ? value : fallback;
  const heartbeat = settings.apple_companion_heartbeat_seconds;

  return {
    ...settings,
    remote_hosts: (settings.remote_hosts ?? []).map((host) => ({
      ...host,
      port: host.port ?? null,
      cwd_filter: host.cwd_filter ?? "",
      auto_connect: host.auto_connect ?? false,
      auth_socket: host.auth_socket ?? "",
    })),
    extra_config_dirs: settings.extra_config_dirs ?? [],
    apple_companion_enabled: settings.apple_companion_enabled ?? false,
    apple_companion_heartbeat_seconds: Number.isFinite(heartbeat)
      ? clamp(Math.round(heartbeat), 1, 30)
      : 5,
    custom_clis: settings.custom_clis ?? [],
    max_visible_sessions: settings.max_visible_sessions === UNLIMITED_VISIBLE_SESSIONS
      ? UNLIMITED_VISIBLE_SESSIONS : clamp(settings.max_visible_sessions, 1, 20),
    session_cleanup_minutes: clamp(settings.session_cleanup_minutes, 0, 1440),
    rotation_interval: normalizeRotationInterval(settings.rotation_interval),
    max_history: clamp(settings.max_history, 10, 2000),
    codex_subsession_mode: oneOf(settings.codex_subsession_mode, ["separate", "merge", "hide"], "separate"),
    follow_up_reminder_minutes: clamp(settings.follow_up_reminder_minutes, 0, 4294967295),
    hover_expand_delay_ms: clamp(settings.hover_expand_delay_ms, 100, 1000),
    notch_animation_speed: clampNotchAnimationSpeed(settings.notch_animation_speed),
    collapsed_width_scale: clamp(
      settings.collapsed_width_scale,
      COLLAPSED_WIDTH_SCALE_MIN,
      COLLAPSED_WIDTH_SCALE_MAX,
    ),
    notch_height_mode: oneOf(
      settings.notch_height_mode,
      ["matchNotch", "matchMenuBar", "custom"],
      "matchNotch",
    ),
    custom_notch_height: clampCustomNotchHeight(settings.custom_notch_height),
    sound_volume: clamp(settings.sound_volume, 0, 100),
    mascot_speed: clamp(settings.mascot_speed, 0, 300),
    content_font_size: clamp(settings.content_font_size, 11, 18),
    ai_message_lines: clamp(settings.ai_message_lines, 0, 10),
    default_mascot: settings.default_mascot || "claude",
    language: oneOf(settings.language, LANGUAGE_OPTIONS, "system"),
    display_mode: oneOf(
      settings.display_mode,
      ["primary", "cursor", "pinned"],
      "primary",
    ),
    display_choice: normalizeDisplayChoice(settings.display_choice),
    density: oneOf(settings.density, ["comfortable", "compact"], "comfortable"),
    group_by: oneOf(settings.group_by, ["all", "status", "cli", "none"], "all"),
    panel_edge: oneOf(
      settings.panel_edge,
      ["top", "bottom", "left", "right"],
      "top",
    ),
    theme: oneOf(settings.theme, ["dark", "light"], "dark"),
    completion_style: oneOf(
      settings.completion_style,
      ["expand", "glance", "off"],
      "expand",
    ) as AppSettings["completion_style"],
    toggle_shortcut: normalizeShortcut(
      settings.toggle_shortcut,
      "Ctrl+Alt+I",
    ),
    approve_shortcut: normalizeShortcut(
      settings.approve_shortcut,
      "Ctrl+Alt+A",
    ),
    approve_always_shortcut: normalizeShortcut(
      settings.approve_always_shortcut,
      "Ctrl+Alt+A",
    ),
    deny_shortcut: normalizeShortcut(settings.deny_shortcut, "Ctrl+Alt+D"),
    skip_question_shortcut: normalizeShortcut(
      settings.skip_question_shortcut,
      "Ctrl+Alt+A",
    ),
    jump_to_terminal_shortcut: normalizeShortcut(
      settings.jump_to_terminal_shortcut,
      "Ctrl+Alt+A",
    ),
  };
}

function normalizeDisplayChoice(value: string): string {
  if (!value) return "auto";
  if (value === "auto" || value === "cursor" || value === "pinned") {
    return value;
  }
  // Pinned displays are stored as `display:<id>`; the Windows host may also
  // use `monitor:` / `screen:` spellings, so accept all of them verbatim.
  if (/^(display|monitor|screen)[:_].+/.test(value)) return value;
  return "auto";
}

function pushUnique(values: string[], value: string) {
  if (!values.includes(value)) values.push(value);
}

function normalizeKey(value: string): string {
  const lower = value.toLowerCase();
  switch (lower) {
    case "esc":
    case "escape":
      return "Escape";
    case "enter":
    case "return":
      return "Enter";
    case "space":
      return "Space";
    case "tab":
      return "Tab";
    default:
      break;
  }
  if (lower.length === 1) return lower.toUpperCase();
  if (lower.startsWith("f") && /^\d+$/.test(lower.slice(1))) {
    return lower.toUpperCase();
  }
  if (!value) return "";
  return value.charAt(0).toUpperCase() + value.slice(1);
}

/**
 * Mirrors `parse_shortcut()` in codeisland-domain/settings.rs.
 * Returns normalized parts, or null when the chord is invalid.
 */
export function parseShortcut(value: string): string[] | null {
  const modifiers: string[] = [];
  let key: string | null = null;

  for (const raw of value.split("+")) {
    const part = raw.trim();
    if (!part) continue;
    switch (part.toLowerCase()) {
      case "ctrl":
      case "control":
        pushUnique(modifiers, "Ctrl");
        break;
      case "shift":
        pushUnique(modifiers, "Shift");
        break;
      case "alt":
        pushUnique(modifiers, "Alt");
        break;
      case "meta":
      case "win":
      case "super":
        pushUnique(modifiers, "Meta");
        break;
      default:
        if (key !== null) return null;
        key = normalizeKey(part);
        break;
    }
  }

  if (key === null) return null;
  return [...modifiers, key];
}

export function normalizeShortcut(value: string, fallback: string): string {
  const parts = parseShortcut(value);
  return parts ? parts.join("+") : fallback;
}

export function isValidShortcut(value: string): boolean {
  return parseShortcut(value) !== null;
}

export const LANGUAGE_OPTIONS = [
  "system",
  "en-US",
  "zh-CN",
  "zh-TW",
  "de-DE",
  "ja-JP",
  "ko-KR",
  "tr-TR",
] as const;
export const DISPLAY_MODE_OPTIONS = ["primary", "cursor", "pinned"] as const;
export const DISPLAY_CHOICE_OPTIONS = ["auto", "cursor", "pinned"] as const;
export const COMPLETION_STYLE_OPTIONS = ["expand", "glance", "off"] as const;
export const DENSITY_OPTIONS = ["comfortable", "compact"] as const;
export const GROUP_BY_OPTIONS = ["all", "status", "cli", "none"] as const;
export const THEME_OPTIONS = ["dark", "light"] as const;
