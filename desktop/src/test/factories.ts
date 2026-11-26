import { DEFAULT_SETTINGS } from "../lib/defaults";
import type {
  PendingRequest,
  SessionSnapshot,
  UiState,
} from "../lib/types";

export function makeSession(
  overrides: Partial<SessionSnapshot> = {},
): SessionSnapshot {
  const now = Date.now();
  return {
    session_id: "sess-1",
    agent: "claude",
    status: "idle",
    project: "demo",
    started_at: new Date(now - 60_000).toISOString(),
    last_activity: new Date(now - 1_000).toISOString(),
    last_message: "Hello from the island.",
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

export function makeRequest(
  overrides: Partial<PendingRequest> = {},
): PendingRequest {
  return {
    event_id: "req-1",
    session_id: "sess-1",
    agent: "claude",
    kind: "permission",
    tool: "Bash",
    title: "Run command",
    body: "npm test",
    options: [],
    created_at: new Date().toISOString(),
    hidden: false,
    ...overrides,
  };
}

export function makeUiState(overrides: Partial<UiState> = {}): UiState {
  return {
    store: {
      sessions: [makeSession()],
      pending: [],
      current_session_id: "sess-1",
      total_sessions: 1,
    },
    settings: { ...DEFAULT_SETTINGS, audible_cues: [...DEFAULT_SETTINGS.audible_cues] },
    version: "test",
    ...overrides,
  };
}
