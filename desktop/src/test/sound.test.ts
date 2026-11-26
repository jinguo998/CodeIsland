import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "../lib/defaults";
import * as ipc from "../lib/ipc";
import * as sound from "../lib/sound";
import { useIslandStore } from "../lib/store";
import type { AttentionEvent } from "../lib/types";
import { makeUiState } from "./factories";

const settings = {
  ...DEFAULT_SETTINGS,
  sound_enabled: true,
  sound_volume: 50,
  mute_when_away: false,
  quiet_hours_enabled: false,
};

class MockAudio {
  static instances = new Map<string, MockAudio>();
  paused = true;
  currentTime = 0;
  volume = 1;
  listeners = new Map<string, () => void>();
  play = vi.fn(async () => { this.paused = false; });
  pause = vi.fn(() => { this.paused = true; });

  addEventListener(event: string, listener: () => void) {
    this.listeners.set(event, listener);
  }

  constructor(src: string) {
    MockAudio.instances.set(src, this);
  }
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("island sound", () => {
  it("replays the bundled error cue after 60 quiet seconds per session", () => {
    vi.stubGlobal("Audio", MockAudio);
    const playFailure = (sessionId: string, seconds: number) =>
      sound.playAttentionSound("StopFailure", settings, new Date(seconds * 1_000), sessionId);

    expect(playFailure("session-a", 0)).toBe(true);
    expect(playFailure("session-a", 59)).toBe(false);
    expect(playFailure("session-a", 60)).toBe(false);
    expect(playFailure("session-b", 60)).toBe(true);
    expect(playFailure("session-a", 120)).toBe(true);

    const audio = MockAudio.instances.get("/sounds/8bit_error.wav");
    expect(audio?.play).toHaveBeenCalledTimes(3);
    expect(audio?.pause).toHaveBeenCalledTimes(2);
    expect(audio?.volume).toBe(0.5);
  });

  it("applies the original volume curve through the 25 percent mark", () => {
    expect(sound.soundAmplitude(0)).toBe(0);
    expect(sound.soundAmplitude(5)).toBeCloseTo(0.01);
    expect(sound.soundAmplitude(25)).toBe(0.25);
    expect(sound.soundAmplitude(50)).toBe(0.5);
    expect(sound.soundAmplitude(100)).toBe(1);
  });

  it("keeps the master switch, per-cue switch and quiet hours in force", () => {
    const midnight = new Date(2026, 0, 1, 0, 0);
    const playApproval = (overrides: Partial<typeof settings>) =>
      sound.playAttentionSound("PermissionRequest", { ...settings, ...overrides }, midnight);

    expect(playApproval({ sound_enabled: false })).toBe(false);
    expect(playApproval({ audible_cues: ["Stop"] })).toBe(false);
    expect(playApproval({
      quiet_hours_enabled: true,
      quiet_hours_start: "23:00",
      quiet_hours_end: "07:00",
    })).toBe(false);
    vi.stubGlobal("Audio", MockAudio);
    expect(playApproval({})).toBe(true);
    expect(MockAudio.instances.get("/sounds/8bit_approval.wav")?.play).toHaveBeenCalledOnce();
  });

  it("plays the chosen file with the original volume curve and releases its URL", async () => {
    vi.stubGlobal("Audio", MockAudio);
    const path = "C:\\sounds\\done.MP3";
    const read = vi.spyOn(ipc, "readCustomSound").mockResolvedValue(new ArrayBuffer(3));
    const create = vi.fn((_blob: Blob) => "blob:chosen-completion");
    const revoke = vi.fn();
    vi.stubGlobal("URL", class extends URL {
      static createObjectURL = create;
      static revokeObjectURL = revoke;
    });
    const customSettings = { ...settings, sound_volume: 5, custom_sound_paths: { "8bit_complete": path } };

    expect(sound.playAttentionSound("Stop", customSettings)).toBe(true);
    await vi.waitFor(() => expect(MockAudio.instances.has("blob:chosen-completion")).toBe(true));
    expect(read).toHaveBeenCalledExactlyOnceWith("8bit_complete", path);
    expect(create.mock.calls[0]).toEqual([expect.objectContaining({ type: "audio/mpeg" })]);
    const audio = MockAudio.instances.get("blob:chosen-completion");
    expect(audio?.play).toHaveBeenCalledOnce();
    expect(audio?.volume).toBeCloseTo(0.01);
    audio?.listeners.get("ended")?.();
    expect(revoke).toHaveBeenCalledExactlyOnceWith("blob:chosen-completion");
  });

  it("falls back to the bundled event cue if its custom file is unavailable", async () => {
    vi.stubGlobal("Audio", MockAudio);
    const path = "C:\\sounds\\missing.wav";
    vi.spyOn(ipc, "readCustomSound").mockRejectedValue(new Error("missing"));
    expect(sound.playAttentionSound("SessionStart", {
      ...settings, custom_sound_paths: { "8bit_start": path },
    })).toBe(true);
    await vi.waitFor(() => expect(MockAudio.instances.get("/sounds/8bit_start.wav")?.play).toHaveBeenCalledOnce());
    expect(await sound.previewSound("SessionStart", 50, path)).toBe(false);
    expect(MockAudio.instances.get("/sounds/8bit_start.wav")?.play).toHaveBeenCalledOnce();
  });

  it("plays one event cue when both the main island and settings subscribe", async () => {
    const listeners = new Set<(event: AttentionEvent) => void>();
    vi.spyOn(ipc, "getState").mockResolvedValue(makeUiState({ settings }));
    vi.spyOn(ipc, "onAttention").mockImplementation(async (listener) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    });
    const play = vi.spyOn(sound, "playAttentionSound").mockReturnValue(true);

    const disposeIsland = await useIslandStore.getState().init(true);
    const disposeSettings = await useIslandStore.getState().init(false);
    for (const listener of listeners) {
      listener({ sessionId: "session-1", decision: { sound: "PermissionRequest" } });
    }

    expect(listeners.size).toBe(2);
    expect(play).toHaveBeenCalledOnce();
    expect(play).toHaveBeenCalledWith(
      "PermissionRequest", expect.objectContaining({ sound_enabled: true }), undefined, "session-1",
    );
    disposeIsland();
    disposeSettings();
  });

  it("requests the launch cue from the main island after its listener is ready", async () => {
    vi.spyOn(ipc, "getState").mockResolvedValue(makeUiState({ settings }));
    let attachListener!: () => void;
    const listenerReady = new Promise<() => void>((resolve) => {
      attachListener = () => resolve(() => {});
    });
    const onAttention = vi.spyOn(ipc, "onAttention")
      .mockResolvedValue(() => {})
      .mockReturnValueOnce(Promise.resolve(() => {}))
      .mockReturnValueOnce(listenerReady);
    const takeLaunchSound = vi.spyOn(ipc, "takeLaunchSound").mockResolvedValueOnce(true).mockResolvedValue(false);
    const play = vi.spyOn(sound, "playAttentionSound").mockReturnValue(true);

    const disposeSettings = await useIslandStore.getState().init(false);
    expect(takeLaunchSound).not.toHaveBeenCalled();
    const island = useIslandStore.getState().init(true);
    await vi.waitFor(() => expect(onAttention).toHaveBeenCalledTimes(2));
    expect(takeLaunchSound).not.toHaveBeenCalled();
    attachListener();
    const disposeIsland = await island;
    const disposeRemount = await useIslandStore.getState().init(true);

    expect(play).toHaveBeenCalledExactlyOnceWith("AppLaunch", expect.objectContaining({ sound_enabled: true }));
    disposeSettings();
    disposeIsland();
    disposeRemount();
  });
});
