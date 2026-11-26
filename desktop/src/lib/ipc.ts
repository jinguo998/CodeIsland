/**
 * Typed wrappers around every Tauri command and event in CONTRACTS.md.
 *
 * When the app runs in a plain browser (`npm run dev` without Tauri), the
 * `mockBackend` keeps the UI interactive so the island can be developed and
 * screenshotted without the Rust host.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { mockBackend } from "./mock";
import type {
  AgentKind,
  AppSettings,
  AttentionEvent,
  CustomCliSetting,
  Decision,
  IslandSurfaceState,
  SoundFileName,
  PushChannelSetting,
  ToastEvent,
  ToolStatus,
  UiState,
  UserAction,
} from "./types";

export type { UnlistenFn };

export function isTauri(): boolean {
  return (
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
  );
}

function args(values: Record<string, unknown>): Record<string, unknown> {
  const clean: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(values)) {
    if (value !== undefined) clean[key] = value;
  }
  return clean;
}

export async function getState(): Promise<UiState> {
  return isTauri() ? invoke<UiState>("get_state") : mockBackend.getState();
}

export async function takeLaunchSound(): Promise<boolean> {
  return isTauri() ? invoke<boolean>("take_launch_sound") : false;
}

export async function resolveRequest(
  eventId: string,
  action: UserAction,
  text?: string,
): Promise<Decision | null> {
  return isTauri()
    ? invoke<Decision | null>("resolve_request", args({ eventId, action, text }))
    : mockBackend.resolveRequest(eventId, action, text);
}

export async function resolveCurrent(
  action: UserAction,
  text?: string,
): Promise<Decision | null> {
  return isTauri()
    ? invoke<Decision | null>("resolve_current", args({ action, text }))
    : mockBackend.resolveCurrent(action, text);
}

export async function hideRequest(eventId: string): Promise<boolean> {
  return isTauri()
    ? invoke<boolean>("hide_request", args({ eventId }))
    : mockBackend.hideRequest(eventId);
}

export async function unhideRequest(eventId: string): Promise<boolean> {
  return isTauri()
    ? invoke<boolean>("unhide_request", args({ eventId }))
    : mockBackend.unhideRequest(eventId);
}

export async function removeSession(sessionId: string): Promise<boolean> {
  return isTauri()
    ? invoke<boolean>("remove_session", args({ sessionId }))
    : mockBackend.removeSession(sessionId);
}

export async function removeIdleSessions(groups: string[][]): Promise<number> {
  return isTauri()
    ? invoke<number>("remove_idle_sessions", { groups })
    : mockBackend.removeIdleSessions(groups);
}

export async function moveSession(
  sessionId: string,
  beforeId: string,
): Promise<boolean> {
  return isTauri()
    ? invoke<boolean>("move_session", args({ sessionId, beforeId }))
    : mockBackend.moveSession(sessionId, beforeId);
}

export async function dismissCompleted(): Promise<number> {
  return isTauri()
    ? invoke<number>("dismiss_completed")
    : mockBackend.dismissCompleted();
}

export async function pickCustomSound(cue: SoundFileName, title: string): Promise<string | null> {
  return isTauri() ? invoke<string | null>("pick_custom_sound", { cue, title }) : null;
}

export async function resetCustomSound(cue: SoundFileName): Promise<void> {
  if (isTauri()) await invoke<void>("reset_custom_sound", { cue });
}

export async function readCustomSound(cue: SoundFileName, path: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("read_custom_sound", { cue, path });
}

export async function getSettings(): Promise<AppSettings> {
  return isTauri()
    ? invoke<AppSettings>("get_settings")
    : mockBackend.getSettings();
}

export async function patchSettings(
  patch: Partial<AppSettings>,
): Promise<AppSettings> {
  return isTauri()
    ? invoke<AppSettings>("patch_settings", { patch })
    : mockBackend.patchSettings(patch);
}

export interface DisplayInfo {
  id: string;
  legacyId?: string | null;
  name: string;
  isPrimary: boolean;
  width: number;
  height: number;
}

export async function listDisplays(): Promise<DisplayInfo[]> {
  return isTauri() ? invoke<DisplayInfo[]>("list_displays") : [];
}

export async function claudeConfigPath(preference: string): Promise<string> {
  return isTauri()
    ? invoke<string>("claude_config_path", { preference })
    : preference.trim() || "~/.claude";
}

export async function hookStatus(): Promise<ToolStatus[]> {
  return isTauri() ? invoke<ToolStatus[]>("hook_status") : mockBackend.hookStatus();
}

export async function installHooks(agent?: AgentKind): Promise<ToolStatus[]> {
  return isTauri()
    ? invoke<ToolStatus[]>("install_hooks", args({ agent }))
    : mockBackend.installHooks(agent);
}

export async function repairHooks(agent?: AgentKind): Promise<ToolStatus[]> {
  return isTauri()
    ? invoke<ToolStatus[]>("repair_hooks", args({ agent }))
    : mockBackend.repairHooks(agent);
}

export async function uninstallHooks(agent?: AgentKind): Promise<ToolStatus[]> {
  return isTauri()
    ? invoke<ToolStatus[]>("uninstall_hooks", args({ agent }))
    : mockBackend.uninstallHooks(agent);
}

export async function customCliStatus(
  setting: CustomCliSetting,
): Promise<ToolStatus> {
  return isTauri()
    ? invoke<ToolStatus>("custom_cli_status", { setting })
    : mockBackend.customCliStatus(setting);
}

export async function installCustomCli(
  setting: CustomCliSetting,
): Promise<ToolStatus> {
  return isTauri()
    ? invoke<ToolStatus>("install_custom_cli", { setting })
    : mockBackend.installCustomCli(setting);
}

export async function uninstallCustomCli(
  setting: CustomCliSetting,
): Promise<ToolStatus> {
  return isTauri()
    ? invoke<ToolStatus>("uninstall_custom_cli", { setting })
    : mockBackend.uninstallCustomCli(setting);
}

export async function exportDiagnosticsAs(title: string): Promise<string | null> {
  return isTauri() ? invoke<string | null>("export_diagnostics_as", { title }) : null;
}

export async function exportDiagnostics(path: string): Promise<string> {
  return isTauri()
    ? invoke<string>("export_diagnostics", { path })
    : mockBackend.exportDiagnostics(path);
}

export async function openWorkspace(sessionId: string): Promise<boolean> {
  return isTauri()
    ? invoke<boolean>("open_workspace", args({ sessionId }))
    : mockBackend.openWorkspace(sessionId);
}

export async function focusSessionTerminal(sessionId: string): Promise<boolean> {
  return isTauri() && invoke<boolean>("focus_session_terminal", { sessionId });
}

export async function toggleCollapsed(): Promise<boolean> {
  return isTauri()
    ? invoke<boolean>("toggle_collapsed")
    : mockBackend.toggleCollapsed();
}

export async function setHoverPanelSize(
  width: number,
  height: number,
  collapsed = false,
): Promise<void> {
  if (isTauri()) await invoke<void>("set_hover_panel_size", { width, height, collapsed });
}

export async function setIslandSurface(surface: IslandSurfaceState): Promise<boolean> {
  if (isTauri()) return invoke<boolean>("set_island_surface", { surface });
  const { settings: { collapsed } } = await mockBackend.getState();
  if (collapsed !== (surface.kind === "collapsed")) await mockBackend.toggleCollapsed();
  return true;
}

export async function ackShortcutCard(eventId: string, surfaceRevision: number): Promise<void> {
  if (isTauri()) await invoke<void>("ack_shortcut_card", { eventId, surfaceRevision });
}

export async function openPendingQuestion(eventId?: string): Promise<boolean> {
  if (isTauri()) return invoke<boolean>("open_pending_question", args({ eventId }));
  return setIslandSurface({ kind: "session_list" });
}

export async function openSettings(): Promise<void> {
  return isTauri() ? invoke<void>("open_settings") : mockBackend.openSettings();
}

export async function quitApp(): Promise<void> {
  return isTauri() ? invoke<void>("quit_app") : mockBackend.quitApp();
}

export interface QuotaSnapshot {
  plan?: string | null;
  used_today: number;
  limit?: number | null;
  last_5h: ClaudeUsageTotals;
  today: ClaudeUsageTotals;
  hourly_output_tokens: number[];
}

export interface ClaudeUsageTotals {
  input_tokens: number;
  output_tokens: number;
  cache_creation_tokens: number;
  cache_read_tokens: number;
  message_count: number;
}

export async function quotaSnapshot(): Promise<QuotaSnapshot | null> {
  return isTauri() ? invoke<QuotaSnapshot | null>("quota_snapshot") : null;
}

export async function testPushChannel(channel: PushChannelSetting, title: string, body: string): Promise<void> {
  if (isTauri()) await invoke("test_push_channel", { channel, title, body });
}

export type ClaudeQuotaKind = "session" | "weekly_all" | "weekly_scoped";

export interface ClaudeQuotaLimit {
  kind: ClaudeQuotaKind;
  percent: number;
  severity: string;
  resets_at: string | null;
  scope_label: string | null;
}

export interface ClaudePlanQuota {
  snapshot: { limits: ClaudeQuotaLimit[]; fetched_at: string } | null;
  error: "no_credential" | "unauthorized" | "rate_limited" | "transport" | "parse" | { http: number } | null;
}

export async function onClaudePlanQuota(callback: (state: ClaudePlanQuota) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  let received = false;
  const off = await listen<ClaudePlanQuota>("island://claude-quota", ({ payload }) => {
    received = true;
    callback(payload);
  });
  try {
    const current = await invoke<ClaudePlanQuota>("claude_plan_quota");
    if (!received) callback(current);
  } catch (error) {
    off();
    throw error;
  }
  return off;
}

export async function sshConnect(hostId: string): Promise<string> {
  return isTauri() ? invoke<string>("ssh_connect", { hostId }) : "unsupported";
}

export async function sshDisconnect(hostId: string): Promise<void> {
  if (isTauri()) await invoke<void>("ssh_disconnect", { hostId });
}

export async function sshStatus(hostId: string): Promise<string> {
  return isTauri() ? invoke<string>("ssh_status", { hostId }) : "not_connected";
}

export async function sshForward(
  hostId: string,
  localPort: number,
  remoteHost: string,
  remotePort: number,
): Promise<number> {
  return isTauri()
    ? invoke<number>("ssh_forward", {
        hostId,
        localPort,
        remoteHost,
        remotePort,
      })
    : localPort;
}

interface RemoteDeployResult {
  remote_binary: string;
  remote_wrapper: string;
  summary?: string;
}

export async function sshDeployHook(
  hostId: string,
  remoteDir?: string,
): Promise<RemoteDeployResult> {
  return isTauri()
    ? invoke<RemoteDeployResult>(
        "ssh_deploy_hook",
        args({ hostId, remoteDir }),
      )
    : { remote_binary: remoteDir ?? "~/.codeisland", remote_wrapper: "" };
}

export async function sshUninstallHooks(hostId: string): Promise<string> {
  return isTauri() ? invoke<string>("ssh_uninstall_hooks", { hostId }) : "";
}

export async function buddyConnect(): Promise<string> {
  return isTauri() ? invoke<string>("buddy_connect") : "unsupported";
}

export async function buddyDisconnect(): Promise<void> {
  if (isTauri()) await invoke<void>("buddy_disconnect");
}

export async function buddyStatus(): Promise<string> {
  return isTauri() ? invoke<string>("buddy_status") : "unsupported";
}

export interface AppleCompanionStatus {
  powered_on: boolean;
  advertising: boolean;
  subscribed_clients: number;
  error: string | null;
}

export async function appleCompanionStatus(): Promise<AppleCompanionStatus> {
  return isTauri()
    ? invoke<AppleCompanionStatus>("apple_companion_status")
    : { powered_on: false, advertising: false, subscribed_clients: 0, error: null };
}

export async function appleCompanionRestart(): Promise<void> {
  if (isTauri()) await invoke<void>("apple_companion_restart");
}

export function onState(handler: (state: UiState) => void): Promise<UnlistenFn> {
  if (isTauri()) {
    return listen<UiState>("island://state", (event) => handler(event.payload));
  }
  return Promise.resolve(mockBackend.onState(handler));
}

export function onAttention(
  handler: (event: AttentionEvent) => void,
): Promise<UnlistenFn> {
  if (isTauri()) {
    return listen<AttentionEvent>("island://attention", (event) =>
      handler(event.payload),
    );
  }
  return Promise.resolve(mockBackend.onAttention(handler));
}

export function onHover(
  handler: (inside: boolean, idleHovered: boolean) => void,
): Promise<UnlistenFn> {
  if (isTauri()) {
    return listen<{ inside: boolean; idleHovered?: boolean }>(
      "island://hover",
      ({ payload }) => handler(payload.inside, payload.idleHovered === true),
    );
  }
  return Promise.resolve(() => {});
}

export function onLaunchPop(handler: (revision: number) => void): Promise<UnlistenFn> {
  return isTauri()
    ? listen<number>("island://launch-pop", (event) => handler(event.payload))
    : Promise.resolve(() => {});
}

export function onIslandVisibility(handler: (visible: boolean) => void): Promise<UnlistenFn> {
  return isTauri()
    ? listen<boolean>("island://visibility", ({ payload }) => handler(payload))
    : Promise.resolve(() => {});
}

export function onIslandDisplayWidth(handler: (width: number) => void): Promise<UnlistenFn> {
  return isTauri()
    ? listen<number>("island://display-width", ({ payload }) => handler(payload))
    : Promise.resolve(() => {});
}

export function onToast(handler: (event: ToastEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) {
    return listen<ToastEvent>("island://toast", (event) =>
      handler(event.payload),
    );
  }
  return Promise.resolve(mockBackend.onToast(handler));
}
