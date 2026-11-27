/**
 * Wire types for the CodeIsland desktop frontend.
 *
 * IMPORTANT: these mirror the Rust domain structs in
 * `src-tauri/crates/codeisland-domain/src/types.rs`.
 *
 * Serde serializes struct *fields* as snake_case (serde's default; `rename_all`
 * is only applied where the Rust code opts in), so every interface below keeps
 * snake_case field names on purpose. Enum *values* that Rust marks with
 * `rename_all = "snake_case"` are represented as snake_case string unions.
 *
 * Do not camelCase these fields. Adapt to view models in `lib/` instead.
 */

export type AgentKind =
  | "claude"
  | "codex"
  | "gemini"
  | "cursor"
  | "opencode"
  | "qoder"
  | "droid"
  | "codebuddy"
  | "copilot"
  | "grok"
  | "kimi"
  | "cline"
  | "pi"
  | "qwen"
  | "hermes"
  | "antigravity"
  | "kiro"
  | "stepfun"
  | "workbuddy"
  | "openclaw"
  | "zcode"
  | "minimax"
  | "deepseek"
  | "aiwork"
  | "mimo"
  | "trae"
  | "traecn"
  | "traecli"
  | "codybuddycn"
  | "qodercn"
  | "qoderwork"
  | "unknown";

export type AgentEventType =
  | "session_start"
  | "session_end"
  | "user_prompt"
  | "tool_start"
  | "tool_end"
  | "permission_request"
  | "question"
  | "message"
  | "error"
  | "heartbeat"
  | "turn_end"
  | "turn_failed"
  | "interrupt"
  | "notification"
  | "subagent_start"
  | "subagent_end";

export type SessionStatus =
  | "idle"
  | "starting"
  | "running"
  | "waiting_permission"
  | "waiting_answer"
  | "completed"
  | "failed"
  | "cancelled";

export type TaskStatus = "pending" | "in_progress" | "completed";

export type RequestKind = "permission" | "question";

export type UserAction =
  | "approve"
  | "deny"
  | "always_allow"
  | "answer"
  | "skip"
  | "hide";

export type CompletionStyle = "expand" | "glance" | "off";

export type CompletionPresentation = "status_only" | "glance" | "card";

export type PushKind =
  | "error"
  | "completion"
  | "approval"
  | "question"
  | "display_only_wait";

export interface AgentTask {
  id: string;
  title: string;
  status: TaskStatus;
}

export interface HistoryEntry {
  at: string;
  kind: string;
  text?: string | null;
}

export interface SessionSnapshot {
  session_id: string;
  parent_session_id?: string | null;
  agent: AgentKind;
  status: SessionStatus;
  title?: string | null;
  cwd?: string | null;
  project?: string | null;
  started_at: string;
  last_activity: string;
  last_message?: string | null;
  live_codex_output?: string | null;
  last_prompt?: string | null;
  active_tool?: string | null;
  active_tool_detail?: string | null;
  provider_session_id?: string | null;
  claude_desktop_session_id?: string | null;
  tool_use_id?: string | null;
  model?: string | null;
  model_label?: string;
  reasoning?: string | null;
  recap?: string | null;
  error?: string | null;
  pid?: number | null;
  terminal_kind?: string | null;
  wezterm_pane?: number | null;
  transcript_path?: string | null;
  git_branch?: string | null;
  interrupted: boolean;
  last_turn_failed: boolean;
  is_executing_tool: boolean;
  child_session_ids: string[];
  always_allowed_tools: string[];
  tasks: AgentTask[];
  history: HistoryEntry[];
}

export interface QuestionItem {
  question: string;
  header?: string | null;
  options: string[];
  multi_select: boolean;
  answer_key?: string | null;
  option_descriptions?: string[];
  is_secret?: boolean;
}

export type ApprovalAlwaysScope =
  | { kind: "session" }
  | { kind: "codex_rules"; patterns: string[][] }
  | { kind: "codex_mcp_config" };

export interface PendingRequest {
  event_id: string;
  session_id: string;
  agent: AgentKind;
  kind: RequestKind;
  tool?: string | null;
  title?: string | null;
  body?: string | null;
  options: string[];
  tool_input?: Record<string, unknown> | null;
  always_scope?: ApprovalAlwaysScope;
  questions?: QuestionItem[];
  created_at: string;
  hidden: boolean;
}

export interface Decision {
  event_id: string;
  session_id: string;
  agent: AgentKind;
  action: UserAction;
  text?: string | null;
}

export interface StoreSnapshot {
  sessions: SessionSnapshot[];
  pending: PendingRequest[];
  current_session_id?: string | null;
  total_sessions: number;
}

export interface RemoteHostSetting {
  id: string;
  name: string;
  host: string;
  port: number | null;
  user: string;
  key_path: string;
  auth_socket: string;
  cwd_filter: string;
  auto_connect: boolean;
}

export interface CustomCliSetting {
  id: string;
  name: string;
  source: string;
  config_path: string;
  format: string;
  config_key: string;
}

export type SoundFileName =
  | "8bit_start"
  | "8bit_complete"
  | "8bit_error"
  | "8bit_approval"
  | "8bit_submit"
  | "8bit_boot";

export interface AppSettings {
  language: string;
  sound_enabled: boolean;
  sound_volume: number;
  audible_cues: string[];
  custom_sound_paths: Partial<Record<SoundFileName, string>>;
  completion_style: CompletionStyle;
  follow_up_reminder_minutes: number;
  max_visible_sessions: number;
  session_cleanup_minutes: number;
  rotation_interval: number;
  max_history: number;
  codex_subsession_mode: string;
  quiet_hours_enabled: boolean;
  quiet_hours_start: string;
  quiet_hours_end: string;
  mute_when_away: boolean;
  smart_suppress: boolean;
  auto_expand_on_permission: boolean;
  auto_expand_on_question: boolean;
  track_claude_desktop_cowork: boolean;
  hide_when_no_session: boolean;
  hover_expand_delay_ms: number;
  notch_animation_speed: number;
  collapsed_width_scale: number;
  notch_height_mode: string;
  custom_notch_height: number;
  allow_horizontal_drag: boolean;
  push_enabled: boolean;
  push_channel: string;
  push_target: string;
  push_channels: PushChannelSetting[];
  push_only_when_away: boolean;
  push_away_idle_minutes: number;
  push_summary_length: number;
  remote_hosts: RemoteHostSetting[];
  extra_config_dirs: string[];
  custom_clis: CustomCliSetting[];
  auto_approve_tools: string;
  auto_approve_sources: string;
  excluded_hook_cwd_substrings: string;
  claude_config_dir: string;
  webhook_enabled: boolean;
  webhook_url: string;
  webhook_event_filter: string;
  display_mode: string;
  display_choice: string;
  pinned_display: string | null;
  hide_in_fullscreen: boolean;
  collapse_on_mouse_leave: boolean;
  auto_collapse_after_session_jump: boolean;
  startup: boolean;
  toggle_shortcut: string;
  toggle_shortcut_enabled: boolean;
  approve_shortcut: string;
  approve_shortcut_enabled: boolean;
  approve_always_shortcut: string;
  approve_always_shortcut_enabled: boolean;
  deny_shortcut: string;
  deny_shortcut_enabled: boolean;
  skip_question_shortcut: string;
  skip_question_shortcut_enabled: boolean;
  jump_to_terminal_shortcut: string;
  jump_to_terminal_shortcut_enabled: boolean;
  theme: string;
  density: string;
  group_by: string;
  auto_remove_idle: boolean;
  panel_edge: string;
  panel_x: number | null;
  panel_y: number | null;
  collapsed: boolean;

  /* Content rows shown on the island cards. */
  content_font_size: number;
  ai_message_lines: number;
  show_agent_details: boolean;
  show_tool_status: boolean;
  show_project_name: boolean;
  show_git_branch: boolean;
  show_task_progress: boolean;
  show_session_recap: boolean;
  show_model_label: boolean;
  show_usage_stats: boolean;
  show_claude_quota: boolean;
  claude_quota_chip: string;

  /* Mascots. */
  mascot_speed: number;
  default_mascot: string;
  apple_companion_enabled: boolean;
  apple_companion_heartbeat_seconds: number;
}

export interface PushChannelSetting {
  kind: string;
  enabled: boolean;
  events: string[];
  endpoint: string;
  target: string;
  token: string;
  secret: string;
  group: string;
  icon: string;
  sound: string;
  priority: number;
  include_details: boolean;
}

export type ExpandedIslandSurface =
  | { kind: "session_list" }
  | { kind: "approval_card"; event_id: string; session_id: string }
  | { kind: "question_card"; event_id: string; session_id: string }
  | { kind: "completion_card"; session_id: string };

export type IslandSurfaceState = { kind: "collapsed" } | ExpandedIslandSurface;

export interface UiState {
  surface?: IslandSurfaceState;
  surface_revision?: number;
  glance_completion_active?: boolean;
  follow_up_hint_active?: boolean;
  follow_up_hint_pulse?: number;
  display_asleep?: boolean;
  idle_session_removal_groups?: string[][];
  store: StoreSnapshot;
  settings: AppSettings;
  version: string;
}

export interface CompletionDecision {
  turn_failed: boolean;
  presentation: CompletionPresentation;
}

export interface PushDecision {
  kind: PushKind;
  smart_suppressed: boolean;
}

/**
 * Rust serializes `FollowUpIntent` as an externally tagged enum:
 * `{ "track_completion": { "turn_failed": false } }` or `"restart_wait"`.
 */
export type FollowUpIntent =
  | { track_completion: { turn_failed: boolean } }
  | "restart_wait";

export interface AttentionDecision {
  sound?: string | null;
  completion?: CompletionDecision | null;
  push?: PushDecision | null;
  follow_up?: FollowUpIntent | null;
}

/** Payload of the `island://attention` Tauri event (camelCase per CONTRACTS.md). */
export interface AttentionEvent {
  sessionId: string;
  decision: AttentionDecision;
}

/** Payload of the `island://toast` Tauri event. */
export interface ToastEvent {
  kind: string;
  message: string;
}

export interface ToolStatus {
  agent: AgentKind;
  display_name: string;
  installed: boolean;
  hook_installed: boolean;
  healthy: boolean;
  config_path?: string | null;
  command?: string | null;
  problem?: string | null;
}
