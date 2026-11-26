import { clampSettings } from "./settings";
import { DEFAULT_SETTINGS } from "./defaults";
import { retainedIdleSessionGroups } from "./sessions";
import type {
  AgentKind,
  AppSettings,
  AttentionEvent,
  CustomCliSetting,
  Decision,
  PendingRequest,
  SessionSnapshot,
  StoreSnapshot,
  ToastEvent,
  ToolStatus,
  UiState,
  UserAction,
} from "./types";

type Listener<T> = (value: T) => void;

function isoAgo(ms: number): string {
  return new Date(Date.now() - ms).toISOString();
}

function baseSession(overrides: Partial<SessionSnapshot>): SessionSnapshot {
  return {
    session_id: "session",
    agent: "unknown",
    status: "idle",
    started_at: isoAgo(60_000),
    last_activity: isoAgo(1_000),
    interrupted: false,
    last_turn_failed: false,
    is_executing_tool: false,
    child_session_ids: [],
    always_allowed_tools: [],
    tasks: [],
    history: [],
    ...overrides,
  };
}

function createSessions(): SessionSnapshot[] {
  return [
    baseSession({
      session_id: "sess-codex",
      agent: "codex",
      status: "waiting_permission",
      project: "atlas-api",
      cwd: "C:\\dev\\atlas-api",
      git_branch: "fix/feed-scroll",
      active_tool: "Bash",
      title: "Deploy pipeline",
      started_at: isoAgo(11 * 60_000),
      last_activity: isoAgo(45_000),
      last_prompt: "Deploy the API to staging and run the smoke tests.",
      last_message: "I need to run a shell command to continue.",
      model: "gpt-5-codex",
      reasoning: "high",
    }),
    baseSession({
      session_id: "sess-gemini",
      agent: "gemini",
      status: "waiting_answer",
      project: "shop-ui",
      cwd: "C:\\dev\\shop-ui",
      git_branch: "feat/checkout",
      started_at: isoAgo(5 * 60_000),
      last_activity: isoAgo(90_000),
      last_prompt: "Set up the checkout flow.",
      last_message: "Which database should the checkout service use?",
      model: "gemini-2.5-pro",
    }),
    baseSession({
      session_id: "sess-claude",
      agent: "claude",
      status: "running",
      project: "codeisland",
      cwd: "C:\\dev\\codeisland",
      git_branch: "feat/ipc-wrapper",
      started_at: isoAgo(6 * 60_000),
      last_activity: isoAgo(3_000),
      last_prompt: "Add a typed IPC wrapper for the island commands.",
      last_message: "Refactoring the IPC layer and wiring the Zustand store.",
      active_tool: "Edit",
      is_executing_tool: true,
      model: "claude-sonnet-4-5",
      reasoning: "high",
      tasks: [
        { id: "t1", title: "Write IPC wrapper", status: "completed" },
        { id: "t2", title: "Wire store", status: "in_progress" },
      ],
    }),
    baseSession({
      session_id: "sess-cursor",
      agent: "cursor",
      status: "completed",
      project: "docs-site",
      cwd: "C:\\dev\\docs-site",
      git_branch: "main",
      started_at: isoAgo(22 * 60_000),
      last_activity: isoAgo(8 * 60_000),
      last_prompt: "Polish the docs landing page.",
      last_message:
        "## Done\n\n- Added **pixel** borders\n- Fixed `markdown` sanitization\n\n```ts\nnpm test\n```",
      model: "cursor-large",
    }),
    baseSession({
      session_id: "sess-kiro",
      agent: "kiro",
      status: "failed",
      project: "infra",
      cwd: "C:\\dev\\infra",
      git_branch: "fix/terraform",
      started_at: isoAgo(16 * 60_000),
      last_activity: isoAgo(15 * 60_000),
      last_prompt: "Bump the terraform provider.",
      last_message: "Build failed: missing dependency `aws-provider`.",
      error: "missing dependency `aws-provider`",
      last_turn_failed: true,
      model: "kiro-auto",
    }),
    baseSession({
      session_id: "sess-opencode",
      agent: "opencode",
      status: "idle",
      project: "infra",
      cwd: "C:\\dev\\infra",
      started_at: isoAgo(60 * 60_000),
      last_activity: isoAgo(30 * 60_000),
      last_message: "Waiting for the next task.",
      model: "opencode-default",
    }),
  ];
}

function createPending(): PendingRequest[] {
  return [
    {
      event_id: "req-perm-1",
      session_id: "sess-codex",
      agent: "codex",
      kind: "permission",
      tool: "Bash",
      title: "Run command",
      body: "npm install && npm run test:e2e -- --grep staging",
      options: [],
      created_at: isoAgo(45_000),
      hidden: false,
    },
    {
      event_id: "req-q-1",
      session_id: "sess-gemini",
      agent: "gemini",
      kind: "question",
      title: "Choose a database",
      body: "Which database should the checkout service use?",
      tool: "AskUserQuestion",
      options: ["Postgres", "SQLite", "DynamoDB"],
      created_at: isoAgo(90_000),
      hidden: false,
    },
  ];
}

function createHooks(): ToolStatus[] {
  return [
    {
      agent: "claude",
      display_name: "Claude Code",
      installed: true,
      hook_installed: true,
      healthy: true,
      config_path: "C:\\Users\\me\\.claude\\settings.json",
      command: "codeisland-hook",
      problem: null,
    },
    {
      agent: "codex",
      display_name: "Codex",
      installed: true,
      hook_installed: false,
      healthy: false,
      config_path: "C:\\Users\\me\\.codex\\config.toml",
      command: "codeisland-hook",
      problem: "Hook not installed",
    },
    {
      agent: "gemini",
      display_name: "Gemini CLI",
      installed: false,
      hook_installed: false,
      healthy: false,
      config_path: null,
      command: null,
      problem: "CLI not found on PATH",
    },
  ];
}

function createUi(): UiState {
  const sessions = createSessions();
  return {
    store: {
      sessions,
      pending: createPending(),
      current_session_id: "sess-claude",
      total_sessions: sessions.length,
    },
    settings: { ...DEFAULT_SETTINGS, audible_cues: [...DEFAULT_SETTINGS.audible_cues] },
    version: "0.1.0-dev",
  };
}

let mockUi: UiState = createUi();
let mockHooks: ToolStatus[] = createHooks();
const mockCustomClis = new Set<string>();

const stateListeners = new Set<Listener<UiState>>();
const attentionListeners = new Set<Listener<AttentionEvent>>();
const toastListeners = new Set<Listener<ToastEvent>>();

function idleSessionRemovalGroups(store: StoreSnapshot): string[][] {
  const sessions = new Map(store.sessions.map((session) => [session.session_id, session]));
  const blocked = new Set(store.pending.map((request) => request.session_id));
  const links = new Map(store.sessions.map((session) => [session.session_id, new Set<string>()]));
  for (const session of store.sessions) {
    const parent = session.parent_session_id && links.get(session.parent_session_id);
    if (!parent || !session.parent_session_id) continue;
    parent.add(session.session_id);
    links.get(session.session_id)?.add(session.parent_session_id);
  }

  const visited = new Set<string>();
  const groups: string[][] = [];
  for (const id of [...links.keys()].sort()) {
    if (visited.has(id)) continue;
    const group = [id];
    visited.add(id);
    for (const member of group) {
      for (const neighbor of links.get(member) ?? []) {
        if (visited.has(neighbor)) continue;
        visited.add(neighbor);
        group.push(neighbor);
      }
    }
    if (group.every((member) => {
      const status = sessions.get(member)?.status;
      return !blocked.has(member) &&
        (status === "idle" || status === "completed" || status === "failed" || status === "cancelled");
    })) groups.push(group.sort());
  }
  return groups;
}

function snapshot(): UiState {
  return {
    idle_session_removal_groups: idleSessionRemovalGroups(mockUi.store),
    store: {
      sessions: mockUi.store.sessions.map((session) => ({ ...session })),
      pending: mockUi.store.pending.map((request) => ({ ...request })),
      current_session_id: mockUi.store.current_session_id,
      total_sessions: mockUi.store.total_sessions,
    },
    settings: {
      ...mockUi.settings,
      audible_cues: [...mockUi.settings.audible_cues],
    },
    version: mockUi.version,
  };
}

function emitState() {
  const next = snapshot();
  stateListeners.forEach((listener) => listener(next));
}

function updateStore(store: StoreSnapshot) {
  mockUi = {
    ...mockUi,
    store: { ...store, total_sessions: store.sessions.length },
  };
  emitState();
}

function updateSettings(settings: AppSettings): AppSettings {
  const next = clampSettings(settings);
  mockUi = { ...mockUi, settings: next };
  emitState();
  return next;
}

function resolve(
  eventId: string,
  action: UserAction,
  text?: string,
): Decision | null {
  const request = mockUi.store.pending.find((item) => item.event_id === eventId);
  if (!request) return null;
  const resumed = action !== "deny" && action !== "skip";
  updateStore({
    ...mockUi.store,
    sessions: mockUi.store.sessions.map((session) =>
      session.session_id === request.session_id &&
      (session.status === "waiting_permission" ||
        session.status === "waiting_answer")
        ? {
            ...session,
            status: resumed ? "running" : "idle",
            is_executing_tool: false,
            last_activity: new Date().toISOString(),
          }
        : session,
    ),
    pending: mockUi.store.pending.filter((item) => item.event_id !== eventId),
  });
  return {
    event_id: request.event_id,
    session_id: request.session_id,
    agent: request.agent,
    action,
    text: text ?? null,
  };
}

export const mockBackend = {
  getState: async (): Promise<UiState> => snapshot(),

  getSettings: async (): Promise<AppSettings> => ({ ...mockUi.settings }),

  patchSettings: async (patch: Partial<AppSettings>): Promise<AppSettings> =>
    updateSettings({ ...mockUi.settings, ...patch }),

  resolveRequest: async (
    eventId: string,
    action: UserAction,
    text?: string,
  ): Promise<Decision | null> => resolve(eventId, action, text),

  resolveCurrent: async (
    action: UserAction,
    text?: string,
  ): Promise<Decision | null> => {
    const request = mockUi.store.pending.find((item) => !item.hidden);
    return request ? resolve(request.event_id, action, text) : null;
  },

  hideRequest: async (eventId: string): Promise<boolean> => {
    let found = false;
    updateStore({
      ...mockUi.store,
      pending: mockUi.store.pending.map((request) => {
        if (request.event_id !== eventId) return request;
        found = true;
        return { ...request, hidden: true };
      }),
    });
    return found;
  },

  unhideRequest: async (eventId: string): Promise<boolean> => {
    let found = false;
    updateStore({
      ...mockUi.store,
      pending: mockUi.store.pending.map((request) => {
        if (request.event_id !== eventId) return request;
        found = true;
        return { ...request, hidden: false };
      }),
    });
    return found;
  },

  removeSession: async (sessionId: string): Promise<boolean> => {
    const exists = mockUi.store.sessions.some(
      (session) => session.session_id === sessionId,
    );
    if (!exists) return false;
    updateStore({
      ...mockUi.store,
      sessions: mockUi.store.sessions.filter(
        (session) => session.session_id !== sessionId,
      ),
      pending: mockUi.store.pending.filter(
        (request) => request.session_id !== sessionId,
      ),
    });
    return true;
  },

  removeIdleSessions: async (groups: string[][]): Promise<number> => {
    const store = mockUi.store;
    const matched = retainedIdleSessionGroups(groups, idleSessionRemovalGroups(store));
    const ids = new Set(matched.flat());
    if (ids.size === 0) return 0;
    const sessions = store.sessions.filter((session) => !ids.has(session.session_id));
    updateStore({
      ...store,
      sessions,
      current_session_id: store.current_session_id && ids.has(store.current_session_id)
        ? sessions[0]?.session_id : store.current_session_id,
    });
    return ids.size;
  },

  moveSession: async (sessionId: string, beforeId: string): Promise<boolean> => {
    const sessions = [...mockUi.store.sessions];
    const from = sessions.findIndex((session) => session.session_id === sessionId);
    if (from < 0) return false;
    const [moved] = sessions.splice(from, 1);
    const target = beforeId
      ? sessions.findIndex((session) => session.session_id === beforeId)
      : -1;
    if (target < 0) sessions.push(moved);
    else sessions.splice(target, 0, moved);
    updateStore({ ...mockUi.store, sessions });
    return true;
  },

  dismissCompleted: async (): Promise<number> => {
    const before = mockUi.store.sessions.length;
    updateStore({
      ...mockUi.store,
      sessions: mockUi.store.sessions.filter(
        (session) =>
          session.status !== "completed" && session.status !== "failed",
      ),
    });
    return before - mockUi.store.sessions.length;
  },

  hookStatus: async (): Promise<ToolStatus[]> =>
    mockHooks.map((status) => ({ ...status })),

  installHooks: async (agent?: AgentKind): Promise<ToolStatus[]> => {
    mockHooks = mockHooks.map((status) =>
      !agent || status.agent === agent
        ? {
            ...status,
            installed: true,
            hook_installed: true,
            healthy: true,
            problem: null,
          }
        : status,
    );
    return mockHooks.map((status) => ({ ...status }));
  },

  repairHooks: async (agent?: AgentKind): Promise<ToolStatus[]> => {
    mockHooks = mockHooks.map((status) =>
      !agent || status.agent === agent
        ? { ...status, hook_installed: true, healthy: true, problem: null }
        : status,
    );
    return mockHooks.map((status) => ({ ...status }));
  },

  uninstallHooks: async (agent?: AgentKind): Promise<ToolStatus[]> => {
    mockHooks = mockHooks.map((status) =>
      !agent || status.agent === agent
        ? {
            ...status,
            hook_installed: false,
            healthy: false,
            problem: "Hook not installed",
          }
        : status,
    );
    return mockHooks.map((status) => ({ ...status }));
  },

  customCliStatus: async (setting: CustomCliSetting): Promise<ToolStatus> => ({
    agent: "unknown",
    display_name: setting.name,
    installed: mockCustomClis.has(setting.source),
    hook_installed: mockCustomClis.has(setting.source),
    healthy: mockCustomClis.has(setting.source),
    config_path: setting.config_path,
    command: `codeisland-hook send --source codeisland-${setting.source} --stdin`,
    problem: mockCustomClis.has(setting.source)
      ? null
      : "hooks are not installed",
  }),

  installCustomCli: async (setting: CustomCliSetting): Promise<ToolStatus> => {
    mockCustomClis.add(setting.source);
    return mockBackend.customCliStatus(setting);
  },

  uninstallCustomCli: async (setting: CustomCliSetting): Promise<ToolStatus> => {
    mockCustomClis.delete(setting.source);
    return mockBackend.customCliStatus(setting);
  },

  exportDiagnostics: async (path: string): Promise<string> => path,

  openWorkspace: async (_sessionId: string): Promise<boolean> => true,

  toggleCollapsed: async (): Promise<boolean> => {
    const collapsed = !mockUi.settings.collapsed;
    updateSettings({ ...mockUi.settings, collapsed });
    return collapsed;
  },

  openSettings: async (): Promise<void> => {
    if (typeof window === "undefined" || typeof window.open !== "function") return;
    const base =
      typeof window.location !== "undefined" ? window.location.pathname : "/";
    window.open(`${base}?window=settings`, "_blank", "width=760,height=600");
  },

  quitApp: async (): Promise<void> => {
    // Browser fallback: nothing to quit.
  },

  onState: (listener: Listener<UiState>): (() => void) => {
    stateListeners.add(listener);
    return () => stateListeners.delete(listener);
  },

  onAttention: (listener: Listener<AttentionEvent>): (() => void) => {
    attentionListeners.add(listener);
    return () => attentionListeners.delete(listener);
  },

  onToast: (listener: Listener<ToastEvent>): (() => void) => {
    toastListeners.add(listener);
    return () => toastListeners.delete(listener);
  },

  /** Dev-only helper: emit a fake attention event from the console. */
  emitAttention: (event: AttentionEvent) => {
    attentionListeners.forEach((listener) => listener(event));
  },

  /** Dev-only helper: emit a fake toast from the console. */
  emitToast: (event: ToastEvent) => {
    toastListeners.forEach((listener) => listener(event));
  },
};

/** Test helper: restore the canned browser state. */
export function resetMockBackend(): void {
  mockUi = createUi();
  mockHooks = createHooks();
  stateListeners.clear();
  attentionListeners.clear();
  toastListeners.clear();
}
