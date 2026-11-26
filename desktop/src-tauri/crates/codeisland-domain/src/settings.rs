use std::collections::{BTreeMap, HashSet};

use chrono::Local;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::attention::CompletionStyle;

const NOTCH_ANIMATION_SPEED_MIN: f64 = 0.5;
const NOTCH_ANIMATION_SPEED_MAX: f64 = 2.0;
const NOTCH_ANIMATION_SPEED_NORMAL: f64 = 1.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RemoteHostSetting {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub key_path: String,
    #[serde(default)]
    pub auth_socket: String,
    #[serde(default)]
    pub cwd_filter: String,
    #[serde(default)]
    pub auto_connect: bool,
}

/// A user-defined CLI integration added from Settings > Hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CustomCliSetting {
    pub id: String,
    pub name: String,
    pub source: String,
    pub config_path: String,
    /// One of `claude_json`, `event_map_json`, `cursor_json`, `copilot_json`.
    pub format: String,
    #[serde(default = "default_config_key")]
    pub config_key: String,
}

fn default_config_key() -> String {
    "hooks".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct PushChannelSetting {
    pub kind: String,
    pub enabled: bool,
    pub events: Vec<String>,
    pub endpoint: String,
    pub target: String,
    pub token: String,
    pub secret: String,
    pub group: String,
    pub icon: String,
    pub sound: String,
    pub priority: u8,
    pub include_details: bool,
}

impl Default for PushChannelSetting {
    fn default() -> Self {
        Self {
            kind: String::new(),
            enabled: false,
            events: ["permission", "question", "completion", "error", "reminder"]
                .map(str::to_owned).to_vec(),
            endpoint: String::new(),
            target: String::new(),
            token: String::new(),
            secret: String::new(),
            group: String::new(),
            icon: String::new(),
            sound: String::new(),
            priority: 4,
            include_details: false,
        }
    }
}

/// Names of the bundled effects, also used by macOS to key custom files.
pub const SOUND_FILE_NAMES: [&str; 6] = [
    "8bit_start", "8bit_complete", "8bit_error", "8bit_approval", "8bit_submit", "8bit_boot",
];
pub(crate) const DEFAULT_MAX_HISTORY: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct AppSettings {
    pub language: String,
    pub sound_enabled: bool,
    pub sound_volume: u8,
    pub audible_cues: Vec<String>,
    pub custom_sound_paths: BTreeMap<String, String>,
    pub completion_style: CompletionStyle,
    pub follow_up_reminder_minutes: u32,
    pub auto_expand_on_permission: bool,
    pub auto_expand_on_question: bool,
    pub track_claude_desktop_cowork: bool,
    pub max_visible_sessions: usize,
    pub session_cleanup_minutes: u32,
    pub rotation_interval: u32,
    pub max_history: usize,
    pub codex_subsession_mode: String,
    pub quiet_hours_enabled: bool,
    pub quiet_hours_start: String,
    pub quiet_hours_end: String,
    pub mute_when_away: bool,
    pub smart_suppress: bool,
    pub push_enabled: bool,
    pub push_channel: String,
    pub push_target: String,
    pub push_channels: Vec<PushChannelSetting>,
    pub push_only_when_away: bool,
    pub push_away_idle_minutes: u32,
    pub push_summary_length: usize,
    pub remote_hosts: Vec<RemoteHostSetting>,
    pub extra_config_dirs: Vec<String>,
    pub custom_clis: Vec<CustomCliSetting>,
    pub auto_approve_tools: String,
    pub auto_approve_sources: String,
    pub excluded_hook_cwd_substrings: String,
    pub claude_config_dir: String,
    pub webhook_enabled: bool,
    pub webhook_url: String,
    pub webhook_event_filter: String,
    pub display_mode: String,
    pub display_choice: String,
    pub pinned_display: Option<String>,
    pub allow_horizontal_drag: bool,
    pub hide_in_fullscreen: bool,
    pub hide_when_no_session: bool,
    pub collapse_on_mouse_leave: bool,
    pub auto_collapse_after_session_jump: bool,
    pub hover_expand_delay_ms: u32,
    #[serde(deserialize_with = "deserialize_notch_animation_speed")]
    pub notch_animation_speed: f64,
    pub collapsed_width_scale: i32,
    pub notch_height_mode: String,
    pub custom_notch_height: f64,
    pub startup: bool,
    pub toggle_shortcut: String,
    pub toggle_shortcut_enabled: bool,
    pub approve_shortcut: String,
    pub approve_shortcut_enabled: bool,
    pub approve_always_shortcut: String,
    pub approve_always_shortcut_enabled: bool,
    pub deny_shortcut: String,
    pub deny_shortcut_enabled: bool,
    pub skip_question_shortcut: String,
    pub skip_question_shortcut_enabled: bool,
    pub jump_to_terminal_shortcut: String,
    pub jump_to_terminal_shortcut_enabled: bool,
    pub theme: String,
    pub density: String,
    pub default_mascot: String,
    pub apple_companion_enabled: bool,
    pub apple_companion_heartbeat_seconds: u32,
    pub mascot_speed: u32,
    pub content_font_size: u32,
    pub ai_message_lines: u32,
    pub show_project_name: bool,
    pub show_git_branch: bool,
    pub show_model_label: bool,
    pub show_task_progress: bool,
    pub show_session_recap: bool,
    pub show_usage_stats: bool,
    pub show_claude_quota: bool,
    pub claude_quota_chip: String,
    pub show_agent_details: bool,
    pub show_tool_status: bool,
    pub group_by: String,
    pub auto_remove_idle: bool,
    pub panel_edge: String,
    pub panel_x: Option<f64>,
    pub panel_y: Option<f64>,
    pub collapsed: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            sound_enabled: false,
            sound_volume: 50,
            audible_cues: vec![
                "SessionStart".into(),
                "PermissionRequest".into(),
                "Stop".into(),
                "StopFailure".into(),
                "AppLaunch".into(),
            ],
            custom_sound_paths: BTreeMap::new(),
            completion_style: CompletionStyle::Expand,
            follow_up_reminder_minutes: 0,
            auto_expand_on_permission: true,
            auto_expand_on_question: true,
            track_claude_desktop_cowork: true,
            max_visible_sessions: 5,
            session_cleanup_minutes: 30,
            rotation_interval: 5,
            max_history: DEFAULT_MAX_HISTORY,
            codex_subsession_mode: "separate".into(),
            quiet_hours_enabled: false,
            quiet_hours_start: "22:00".into(),
            quiet_hours_end: "08:00".into(),
            mute_when_away: true,
            smart_suppress: true,
            push_enabled: false,
            push_channel: "ntfy".into(),
            push_target: String::new(),
            push_channels: Vec::new(),
            push_only_when_away: true,
            push_away_idle_minutes: 5,
            push_summary_length: 200,
            remote_hosts: Vec::new(),
            extra_config_dirs: Vec::new(),
            custom_clis: Vec::new(),
            auto_approve_tools: String::new(),
            auto_approve_sources: String::new(),
            excluded_hook_cwd_substrings: String::new(),
            claude_config_dir: String::new(),
            webhook_enabled: false,
            webhook_url: String::new(),
            webhook_event_filter: String::new(),
            display_mode: "primary".into(),
            display_choice: "auto".into(),
            pinned_display: None,
            allow_horizontal_drag: false,
            hide_in_fullscreen: true,
            hide_when_no_session: false,
            collapse_on_mouse_leave: true,
            auto_collapse_after_session_jump: false,
            hover_expand_delay_ms: 500,
            notch_animation_speed: NOTCH_ANIMATION_SPEED_NORMAL,
            collapsed_width_scale: 100,
            notch_height_mode: "matchNotch".into(),
            custom_notch_height: 37.0,
            startup: false,
            toggle_shortcut: "Ctrl+Alt+I".into(),
            toggle_shortcut_enabled: true,
            approve_shortcut: "Ctrl+Alt+A".into(),
            approve_shortcut_enabled: false,
            approve_always_shortcut: "Ctrl+Alt+A".into(),
            approve_always_shortcut_enabled: false,
            deny_shortcut: "Ctrl+Alt+D".into(),
            deny_shortcut_enabled: false,
            skip_question_shortcut: "Ctrl+Alt+A".into(),
            skip_question_shortcut_enabled: false,
            jump_to_terminal_shortcut: "Ctrl+Alt+A".into(),
            jump_to_terminal_shortcut_enabled: false,
            theme: "dark".into(),
            density: "comfortable".into(),
            default_mascot: "claude".into(),
            apple_companion_enabled: false,
            apple_companion_heartbeat_seconds: 5,
            mascot_speed: 100,
            content_font_size: 11,
            ai_message_lines: 1,
            show_project_name: true,
            show_git_branch: true,
            show_model_label: false,
            show_task_progress: true,
            show_session_recap: true,
            show_usage_stats: true,
            show_claude_quota: false,
            claude_quota_chip: "auto".to_owned(),
            show_agent_details: false,
            show_tool_status: true,
            group_by: "all".into(),
            auto_remove_idle: false,
            panel_edge: "top".into(),
            panel_x: None,
            panel_y: None,
            collapsed: false,
        }
    }
}

impl AppSettings {
    pub fn validate(mut self) -> Self {
        // The original picker stores 99 for Unlimited. Preserve older
        // Windows settings that used any number from 1 through 20.
        if self.max_visible_sessions != 99 {
            self.max_visible_sessions = self.max_visible_sessions.clamp(1, 20);
        }
        // 0 means "never expire"; the runtime skips the sweep entirely.
        self.session_cleanup_minutes = self.session_cleanup_minutes.clamp(0, 1440);
        if !matches!(self.rotation_interval, 3 | 5 | 8 | 10) {
            self.rotation_interval = 5;
        }
        self.max_history = self.max_history.clamp(10, 2000);
        if !matches!(self.codex_subsession_mode.as_str(), "separate" | "merge" | "hide") {
            self.codex_subsession_mode = "separate".into();
        }
        if !matches!(self.push_away_idle_minutes, 1 | 2 | 5 | 10 | 15 | 30) {
            self.push_away_idle_minutes = 5;
        }
        if !matches!(self.push_summary_length, 100 | 200 | 500 | 1000) {
            self.push_summary_length = 200;
        }
        self.hover_expand_delay_ms = self.hover_expand_delay_ms.clamp(100, 1000);
        self.notch_animation_speed = clamp_notch_animation_speed(self.notch_animation_speed);
        self.collapsed_width_scale = self.collapsed_width_scale.clamp(50, 150);
        if !matches!(self.notch_height_mode.as_str(), "matchNotch" | "matchMenuBar" | "custom") {
            self.notch_height_mode = "matchNotch".into();
        }
        if !matches!(self.claude_quota_chip.as_str(), "off" | "auto" | "session" | "weeklyAll" | "weeklyScoped") {
            self.claude_quota_chip = "auto".into();
        }
        self.custom_notch_height = if self.custom_notch_height > 0.0 {
            self.custom_notch_height.clamp(15.0, 60.0)
        } else {
            37.0
        };
        self.sound_volume = self.sound_volume.clamp(0, 100);
        self.custom_sound_paths.retain(|name, path| {
            SOUND_FILE_NAMES.contains(&name.as_str()) && !path.trim().is_empty()
        });
        self.mascot_speed = self.mascot_speed.clamp(0, 300);
        self.apple_companion_heartbeat_seconds = self.apple_companion_heartbeat_seconds.clamp(1, 30);
        self.content_font_size = self.content_font_size.clamp(11, 18);
        self.ai_message_lines = self.ai_message_lines.clamp(0, 10);
        if !matches!(
            self.language.as_str(),
            "system"
                | "en-US"
                | "zh-CN"
                | "zh-TW"
                | "zh-Hant"
                | "de"
                | "de-DE"
                | "ja"
                | "ja-JP"
                | "ko"
                | "ko-KR"
                | "tr"
                | "tr-TR"
        ) {
            self.language = "system".into();
        }
        if !matches!(self.display_mode.as_str(), "primary" | "cursor" | "pinned") {
            self.display_mode = "primary".into();
        }
        // "auto", "cursor", or a monitor id the frontend resolves.
        self.display_choice = self.display_choice.trim().to_owned();
        if self.display_choice.is_empty() {
            self.display_choice = "auto".into();
        }
        if !matches!(self.density.as_str(), "comfortable" | "compact") {
            self.density = "comfortable".into();
        }
        if !matches!(self.group_by.as_str(), "all" | "status" | "cli" | "none") {
            self.group_by = "all".into();
        }
        if !matches!(self.panel_edge.as_str(), "top" | "bottom" | "left" | "right") {
            self.panel_edge = "top".into();
        }
        if !matches!(self.theme.as_str(), "dark" | "light") {
            self.theme = "dark".into();
        }
        self.toggle_shortcut = normalize_shortcut(&self.toggle_shortcut, "Ctrl+Alt+I");
        self.approve_shortcut = normalize_shortcut(&self.approve_shortcut, "Ctrl+Alt+A");
        self.approve_always_shortcut =
            normalize_shortcut(&self.approve_always_shortcut, "Ctrl+Alt+A");
        self.deny_shortcut = normalize_shortcut(&self.deny_shortcut, "Ctrl+Alt+D");
        self.skip_question_shortcut =
            normalize_shortcut(&self.skip_question_shortcut, "Ctrl+Alt+A");
        self.jump_to_terminal_shortcut =
            normalize_shortcut(&self.jump_to_terminal_shortcut, "Ctrl+Alt+A");
        self
    }

    pub fn audible_cues(&self) -> HashSet<String> {
        self.audible_cues.iter().cloned().collect()
    }

    pub fn is_quiet_now(&self) -> bool {
        self.quiet_hours_enabled
            && crate::attention::in_quiet_hours(
                Local::now(),
                &self.quiet_hours_start,
                &self.quiet_hours_end,
            )
    }
}

fn clamp_notch_animation_speed(speed: f64) -> f64 {
    if speed.is_finite() {
        speed.clamp(NOTCH_ANIMATION_SPEED_MIN, NOTCH_ANIMATION_SPEED_MAX)
    } else {
        NOTCH_ANIMATION_SPEED_NORMAL
    }
}

fn deserialize_notch_animation_speed<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<f64, D::Error> {
    // A malformed speed should not discard the rest of the user's settings.
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_f64().unwrap_or(NOTCH_ANIMATION_SPEED_NORMAL))
}

pub fn normalize_shortcut(value: &str, fallback: &str) -> String {
    match parse_shortcut(value) {
        Some(parts) => parts.join("+"),
        None => fallback.to_owned(),
    }
}

pub fn parse_shortcut(value: &str) -> Option<Vec<String>> {
    let mut modifiers = Vec::new();
    let mut key = None;
    for part in value.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => push_unique(&mut modifiers, "Ctrl"),
            "shift" => push_unique(&mut modifiers, "Shift"),
            "alt" => push_unique(&mut modifiers, "Alt"),
            "meta" | "win" | "super" => push_unique(&mut modifiers, "Meta"),
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(normalize_key(part));
            }
        }
    }
    let key = key?;
    let mut parts = modifiers;
    parts.push(key);
    Some(parts)
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

fn normalize_key(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "esc" | "escape" => "Escape".into(),
        "enter" | "return" => "Enter".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        _ if lower.len() == 1 => lower.to_ascii_uppercase(),
        _ if lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => lower.to_ascii_uppercase(),
        _ => {
            let mut chars = value.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}
