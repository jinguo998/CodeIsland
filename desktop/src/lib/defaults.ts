import type { AppSettings } from "./types";

export const NOTCH_ANIMATION_SPEED_MIN = 0.5;
export const NOTCH_ANIMATION_SPEED_MAX = 2.0;
export const NOTCH_ANIMATION_SPEED_STEP = 0.1;
const NOTCH_ANIMATION_SPEED_NORMAL = 1.0;
export const COLLAPSED_WIDTH_SCALE_MIN = 50;
export const COLLAPSED_WIDTH_SCALE_MAX = 150;
export const CUSTOM_NOTCH_HEIGHT_MIN = 15;
export const CUSTOM_NOTCH_HEIGHT_MAX = 60;
const CUSTOM_NOTCH_HEIGHT_DEFAULT = 37;

/** Mirrors `AppSettings::default()` in codeisland-domain. */
export const DEFAULT_SETTINGS: AppSettings = {
  language: "system",
  sound_enabled: false,
  sound_volume: 50,
  audible_cues: [
    "SessionStart",
    "PermissionRequest",
    "Stop",
    "StopFailure",
    "AppLaunch",
  ],
  custom_sound_paths: {},
  completion_style: "expand",
  follow_up_reminder_minutes: 0,
  max_visible_sessions: 5,
  session_cleanup_minutes: 30,
  rotation_interval: 5,
  max_history: 20,
  codex_subsession_mode: "separate",
  quiet_hours_enabled: false,
  quiet_hours_start: "22:00",
  quiet_hours_end: "08:00",
  mute_when_away: true,
  smart_suppress: true,
  auto_expand_on_permission: true,
  auto_expand_on_question: true,
  track_claude_desktop_cowork: true,
  hide_when_no_session: false,
  hover_expand_delay_ms: 500,
  notch_animation_speed: NOTCH_ANIMATION_SPEED_NORMAL,
  collapsed_width_scale: 100,
  notch_height_mode: "matchNotch",
  custom_notch_height: CUSTOM_NOTCH_HEIGHT_DEFAULT,
  allow_horizontal_drag: false,
  push_enabled: false,
  push_channel: "ntfy",
  push_target: "",
  push_channels: [],
  push_only_when_away: true,
  push_away_idle_minutes: 5,
  push_summary_length: 200,
  remote_hosts: [],
  extra_config_dirs: [],
  custom_clis: [],
  auto_approve_tools: "",
  auto_approve_sources: "",
  excluded_hook_cwd_substrings: "",
  claude_config_dir: "",
  webhook_enabled: false,
  webhook_url: "",
  webhook_event_filter: "",
  display_mode: "primary",
  display_choice: "auto",
  pinned_display: null,
  hide_in_fullscreen: true,
  collapse_on_mouse_leave: true,
  auto_collapse_after_session_jump: false,
  startup: false,
  toggle_shortcut: "Ctrl+Alt+I",
  toggle_shortcut_enabled: true,
  approve_shortcut: "Ctrl+Alt+A",
  approve_shortcut_enabled: false,
  approve_always_shortcut: "Ctrl+Alt+A",
  approve_always_shortcut_enabled: false,
  deny_shortcut: "Ctrl+Alt+D",
  deny_shortcut_enabled: false,
  skip_question_shortcut: "Ctrl+Alt+A",
  skip_question_shortcut_enabled: false,
  jump_to_terminal_shortcut: "Ctrl+Alt+A",
  jump_to_terminal_shortcut_enabled: false,
  theme: "dark",
  density: "comfortable",
  group_by: "all",
  auto_remove_idle: false,
  panel_edge: "top",
  panel_x: null,
  panel_y: null,
  collapsed: false,
  content_font_size: 11,
  ai_message_lines: 1,
  show_agent_details: false,
  show_tool_status: true,
  show_project_name: true,
  show_git_branch: true,
  show_task_progress: true,
  show_session_recap: true,
  show_model_label: false,
  show_usage_stats: true,
  show_claude_quota: false,
  claude_quota_chip: "auto",
  mascot_speed: 100,
  default_mascot: "claude",
  apple_companion_enabled: false,
  apple_companion_heartbeat_seconds: 5,
};

export const UNLIMITED_VISIBLE_SESSIONS = 99;

export const ROTATION_INTERVAL_OPTIONS = [3, 5, 8, 10] as const;

export const HOVER_EXPAND_DELAY_MIN = 100;
export const HOVER_EXPAND_DELAY_MAX = 1000;
export const HOVER_EXPAND_DELAY_STEP = 50;

export const CONTENT_FONT_SIZE_MIN = 11;
export const CONTENT_FONT_SIZE_MAX = 18;
export const AI_MESSAGE_LINES_MIN = 0;
export const AI_MESSAGE_LINES_MAX = 10;

export const SOUND_VOLUME_MIN = 0;
export const SOUND_VOLUME_MAX = 100;

export const MASCOT_SPEED_MIN = 0;
export const MASCOT_SPEED_MAX = 300;
export const MASCOT_SPEED_STEP = 25;

export function clampNotchAnimationSpeed(value: number): number {
  if (!Number.isFinite(value)) return NOTCH_ANIMATION_SPEED_NORMAL;
  return Math.min(
    NOTCH_ANIMATION_SPEED_MAX,
    Math.max(NOTCH_ANIMATION_SPEED_MIN, value),
  );
}

export function normalizeRotationInterval(value: number): number {
  return ROTATION_INTERVAL_OPTIONS.some((option) => option === value)
    ? value
    : DEFAULT_SETTINGS.rotation_interval;
}

export function clampCustomNotchHeight(value: number): number {
  return value > 0
    ? Math.min(CUSTOM_NOTCH_HEIGHT_MAX, Math.max(CUSTOM_NOTCH_HEIGHT_MIN, value))
    : CUSTOM_NOTCH_HEIGHT_DEFAULT;
}

/**
 * Fills in fields a newer frontend knows about but an older backend may not
 * have persisted yet, so controls and cards never see `undefined`.
 */
export function withSettingsDefaults(
  settings?: Partial<AppSettings> | null,
): AppSettings {
  const merged: AppSettings = { ...DEFAULT_SETTINGS };
  for (const [key, value] of Object.entries(settings ?? {})) {
    if (value !== undefined) {
      (merged as unknown as Record<string, unknown>)[key] = value;
    }
  }
  merged.notch_animation_speed = clampNotchAnimationSpeed(merged.notch_animation_speed);
  merged.custom_notch_height = clampCustomNotchHeight(merged.custom_notch_height);
  merged.rotation_interval = normalizeRotationInterval(merged.rotation_interval);
  merged.custom_sound_paths = { ...settings?.custom_sound_paths };
  merged.audible_cues = Array.isArray(settings?.audible_cues)
    ? [...settings.audible_cues]
    : [...DEFAULT_SETTINGS.audible_cues];
  return merged;
}

export const AUDIBLE_CUES = [
  "SessionStart",
  "UserPromptSubmit",
  "PermissionRequest",
  "Stop",
  "StopFailure",
  "AppLaunch",
] as const;

export type AudibleCue = (typeof AUDIBLE_CUES)[number];

/** i18n keys for each audible cue. */
export const CUE_LABEL_KEYS: Record<AudibleCue, string> = {
  SessionStart: "settings.sound.cueSessionStart",
  UserPromptSubmit: "settings.sound.cuePrompt",
  PermissionRequest: "settings.sound.cuePermission",
  Stop: "settings.sound.cueStop",
  StopFailure: "settings.sound.cueStopFailure",
  AppLaunch: "settings.sound.cueLaunch",
};
