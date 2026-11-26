import { create } from "zustand";
import { withSettingsDefaults } from "./defaults";
import * as ipc from "./ipc";
import { applyLanguage } from "./i18n";
import { clampSettings } from "./settings";
import { playAttentionSound } from "./sound";
import type {
  AppSettings,
  AttentionEvent,
  ToastEvent,
  UiState,
  UserAction,
} from "./types";

interface IslandState {
  ui: UiState | null;
  ready: boolean;
  /** Latest attention decision pushed by the backend. */
  attention: AttentionEvent | null;
  attentionSeq: number;
  toast: ToastEvent | null;
  claudePlanQuota: ipc.ClaudePlanQuota | null;
  settingsDraft: Partial<AppSettings>;
  settingsSaveStatus: "idle" | "saving" | "saved" | "error";

  init: (playEventSounds?: boolean) => Promise<() => void>;
  refresh: () => Promise<void>;

  resolveRequest: (
    eventId: string,
    action: UserAction,
    text?: string,
  ) => Promise<void>;
  resolveCurrent: (action: UserAction, text?: string) => Promise<void>;
  hideRequest: (eventId: string) => Promise<void>;
  removeSession: (sessionId: string) => Promise<void>;
  removeIdleSessions: (groups: string[][]) => Promise<void>;
  moveSession: (sessionId: string, beforeId: string) => Promise<void>;
  dismissCompleted: () => Promise<void>;
  openWorkspace: (sessionId: string) => Promise<void>;
  updateSettings: (patch: Partial<AppSettings>) => Promise<void>;
  retrySettings: () => Promise<void>;
  toggleCollapsed: () => Promise<void>;
  openSettings: () => Promise<void>;

  clearToast: () => void;
  clearAttention: () => void;
}

function errorToast(error: unknown): ToastEvent {
  return {
    kind: "error",
    message: error instanceof Error ? error.message : String(error),
  };
}

/** Older backends may omit newly added settings; keep the UI fully defined. */
function normalizeUi(ui: UiState): UiState {
  return { ...ui, settings: withSettingsDefaults(ui.settings) };
}

function editedSettings(settings: AppSettings, patch: Partial<AppSettings>): Partial<AppSettings> {
  const keys = Object.keys(patch) as Array<keyof AppSettings>;
  return Object.fromEntries(keys.map((key) => [key, settings[key]]));
}

let queuedSettings: Partial<AppSettings> = {};
let savingSettings = false;

async function saveSettings() {
  if (savingSettings || Object.keys(queuedSettings).length === 0) return;
  savingSettings = true;
  try {
    while (Object.keys(queuedSettings).length) {
      const submitted = queuedSettings;
      queuedSettings = {};
      useIslandStore.setState({ settingsSaveStatus: "saving" });
      try {
        const saved = withSettingsDefaults(await ipc.patchSettings(submitted));
        useIslandStore.setState((state) => ({
          ui: state.ui ? { ...state.ui, settings: { ...state.ui.settings, ...editedSettings(saved, submitted) } } : null,
          settingsDraft: queuedSettings,
          settingsSaveStatus: Object.keys(queuedSettings).length ? "saving" : "saved",
        }));
        applyLanguage(queuedSettings.language ?? useIslandStore.getState().ui?.settings.language ?? saved.language);
      } catch (error) {
        queuedSettings = { ...submitted, ...queuedSettings };
        useIslandStore.setState({ settingsSaveStatus: "error", toast: errorToast(error) });
        break;
      }
    }
  } finally {
    savingSettings = false;
  }
}

export const useIslandStore = create<IslandState>()((set, get) => ({
  ui: null,
  ready: false,
  attention: null,
  attentionSeq: 0,
  toast: null,
  claudePlanQuota: null,
  settingsDraft: {},
  settingsSaveStatus: "idle",

  init: async (playEventSounds = true) => {
    let cancelled = false;
    try {
      const ui = normalizeUi(await ipc.getState());
      if (cancelled) return () => {};
      set({ ui, ready: true });
      applyLanguage(ui.settings.language);
    } catch (error) {
      set({ ready: true, toast: errorToast(error) });
    }

    const unlisten = await Promise.all([
      ipc.onState((next) => {
        const ui = normalizeUi(next);
        set({ ui });
        applyLanguage(get().settingsDraft.language ?? ui.settings.language);
      }),
      ipc.onAttention((event) => {
        const settings = get().ui?.settings;
        if (playEventSounds && settings) {
          // Sound-only events still play, and only the main island plays them.
          playAttentionSound(event.decision.sound, settings, undefined, event.sessionId);
        }
        const completion = event.decision.completion;
        const hasCard = Boolean(
          completion && completion.presentation !== "status_only",
        );
        const hasPush = Boolean(event.decision.push);
        // Sound-only events (session start, prompt, heartbeat) are not UI news.
        if (!hasCard && !hasPush) return;
        set((state) => ({
          attention: event,
          attentionSeq: state.attentionSeq + 1,
        }));
      }),
      ipc.onToast((event) => set({ toast: event })),
      playEventSounds && typeof ipc.onClaudePlanQuota === "function"
        ? ipc.onClaudePlanQuota((claudePlanQuota) => set({ claudePlanQuota })).catch(() => () => {})
        : Promise.resolve(() => {}),
    ]);

    if (cancelled) {
      unlisten.forEach((off) => off());
      return () => {};
    }
    if (playEventSounds && get().ui?.settings) {
      void ipc.takeLaunchSound().then((shouldPlay) => {
        const settings = get().ui?.settings;
        if (shouldPlay && settings) playAttentionSound("AppLaunch", settings);
      }).catch(() => {});
    }
    return () => {
      cancelled = true;
      unlisten.forEach((off) => off());
    };
  },

  refresh: async () => {
    try {
      const ui = normalizeUi(await ipc.getState());
      set({ ui });
      applyLanguage(get().settingsDraft.language ?? ui.settings.language);
    } catch (error) {
      set({ toast: errorToast(error) });
    }
  },

  resolveRequest: async (eventId, action, text) => {
    try {
      await ipc.resolveRequest(eventId, action, text);
    } catch (error) {
      set({ toast: errorToast(error) });
    }
    await get().refresh();
  },

  resolveCurrent: async (action, text) => {
    try {
      await ipc.resolveCurrent(action, text);
    } catch (error) {
      set({ toast: errorToast(error) });
    }
    await get().refresh();
  },

  hideRequest: async (eventId) => {
    try {
      await ipc.hideRequest(eventId);
    } catch (error) {
      set({ toast: errorToast(error) });
    }
    await get().refresh();
  },

  removeSession: async (sessionId) => {
    try {
      await ipc.removeSession(sessionId);
    } catch (error) {
      set({ toast: errorToast(error) });
    }
    await get().refresh();
  },

  removeIdleSessions: async (groups) => {
    try {
      await ipc.removeIdleSessions(groups);
      await get().refresh();
    } catch (error) {
      set({ toast: errorToast(error) });
    }
  },

  moveSession: async (sessionId, beforeId) => {
    try {
      await ipc.moveSession(sessionId, beforeId);
    } catch (error) {
      set({ toast: errorToast(error) });
    }
    await get().refresh();
  },

  dismissCompleted: async () => {
    try {
      await ipc.dismissCompleted();
    } catch (error) {
      set({ toast: errorToast(error) });
    }
    await get().refresh();
  },

  openWorkspace: async (sessionId) => {
    try {
      await ipc.openWorkspace(sessionId);
    } catch (error) {
      set({ toast: errorToast(error) });
    }
  },

  updateSettings: (patch) => {
    const { ui, settingsDraft } = get();
    const edited = ui ? editedSettings(clampSettings({ ...ui.settings, ...settingsDraft, ...patch }), patch) : patch;
    queuedSettings = { ...queuedSettings, ...edited };
    set({ settingsDraft: { ...settingsDraft, ...edited } });
    if (edited.language) applyLanguage(edited.language);
    return saveSettings();
  },
  retrySettings: saveSettings,

  toggleCollapsed: async () => {
    try {
      await ipc.toggleCollapsed();
    } catch (error) {
      set({ toast: errorToast(error) });
    }
    await get().refresh();
  },

  openSettings: async () => {
    try {
      await ipc.openSettings();
    } catch (error) {
      set({ toast: errorToast(error) });
    }
  },

  clearToast: () => set({ toast: null }),
  clearAttention: () => set({ attention: null }),
}));
