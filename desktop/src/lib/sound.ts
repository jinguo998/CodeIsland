import type { AudibleCue } from "./defaults";
import { readCustomSound } from "./ipc";
import type { AppSettings, SoundFileName } from "./types";

type CueName = "approval" | "completion" | "error" | "start" | "submit" | "boot";

const SOUND_FILES: Record<CueName, SoundFileName> = {
  approval: "8bit_approval",
  completion: "8bit_complete",
  error: "8bit_error",
  start: "8bit_start",
  submit: "8bit_submit",
  boot: "8bit_boot",
};

const CUE_FOR_SOUND: Readonly<Record<AudibleCue | "TaskRoundComplete", CueName>> = {
  PermissionRequest: "approval",
  Stop: "completion",
  TaskRoundComplete: "completion",
  StopFailure: "error",
  SessionStart: "start",
  UserPromptSubmit: "submit",
  AppLaunch: "boot",
};

const audioCache = new Map<CueName, HTMLAudioElement>();
const customAudio = new Map<CueName, { audio: HTMLAudioElement; url: string }>();
const pendingCustomAudio = new Map<CueName, symbol>();

const AUDIO_MIME_TYPES: Readonly<Partial<Record<string, string>>> = {
  wav: "audio/wav",
  mp3: "audio/mpeg",
  m4a: "audio/mp4",
  aac: "audio/aac",
  flac: "audio/flac",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  opus: "audio/ogg",
  webm: "audio/webm",
  wma: "audio/x-ms-wma",
  aif: "audio/aiff",
  aiff: "audio/aiff",
};

export function soundFileName(cue: AudibleCue): SoundFileName {
  return SOUND_FILES[CUE_FOR_SOUND[cue]];
}
const lastFailureBySession = new Map<string, number>();
const FAILURE_QUIET_MS = 60_000;

/** `"22:00"` → 1320 minutes since midnight. */
export function parseClock(value: string): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return hours * 60 + minutes;
}

/** Handles quiet windows that span midnight (start > end). */
export function inQuietHours(
  start: string,
  end: string,
  now: Date = new Date(),
): boolean {
  const startMinutes = parseClock(start);
  const endMinutes = parseClock(end);
  if (startMinutes === null || endMinutes === null) return false;
  if (startMinutes === endMinutes) return false;
  const current = now.getHours() * 60 + now.getMinutes();
  return startMinutes < endMinutes
    ? current >= startMinutes && current < endMinutes
    : current >= startMinutes || current < endMinutes;
}

/** True when a cue should be audible right now. */
export function shouldPlaySound(
  settings: Pick<
    AppSettings,
    "sound_enabled" | "quiet_hours_enabled" | "quiet_hours_start" | "quiet_hours_end" | "mute_when_away"
  >,
  now: Date = new Date(),
): boolean {
  if (!settings.sound_enabled) return false;
  // Native cues already passed the runtime's Windows away check. The
  // browser preview uses document visibility as its local fallback.
  if (
    settings.mute_when_away &&
    typeof window !== "undefined" &&
    !("__TAURI_INTERNALS__" in window) &&
    typeof document !== "undefined" &&
    document.hidden
  ) {
    return false;
  }
  if (
    settings.quiet_hours_enabled &&
    inQuietHours(settings.quiet_hours_start, settings.quiet_hours_end, now)
  ) {
    return false;
  }
  return true;
}

/** The macOS slider is quadratic through 25% and linear above it. */
export function soundAmplitude(percent: number): number {
  const clamped = Math.min(100, Math.max(0, percent));
  return clamped < 25 ? 0.25 * (clamped / 25) ** 2 : clamped / 100;
}

function startAudio(audio: HTMLAudioElement, volume: number): Promise<void> {
  if (!audio.paused) audio.pause();
  audio.currentTime = 0;
  audio.volume = volume;
  return audio.play();
}

function releaseCustomAudio(cue: CueName) {
  const custom = customAudio.get(cue);
  if (custom) {
    customAudio.delete(cue);
    custom.audio.pause();
    URL.revokeObjectURL(custom.url);
  }
}

function playBundledCue(cue: CueName, volume: number): Promise<void> {
  releaseCustomAudio(cue);
  let audio = audioCache.get(cue);
  if (!audio) {
    audio = new Audio(`/sounds/${SOUND_FILES[cue]}.wav`);
    audioCache.set(cue, audio);
  }
  return startAudio(audio, volume);
}

async function playSelectedCue(cue: CueName, path: string, volume: number, fallback: boolean): Promise<boolean> {
  const ticket = Symbol();
  pendingCustomAudio.set(cue, ticket);
  try {
    const bytes = await readCustomSound(SOUND_FILES[cue], path);
    if (pendingCustomAudio.get(cue) !== ticket) return true;
    const extension = /\.([^./\\]+)$/.exec(path)?.[1].toLowerCase() ?? "";
    const url = URL.createObjectURL(new Blob([bytes], { type: AUDIO_MIME_TYPES[extension] }));
    releaseCustomAudio(cue);
    audioCache.get(cue)?.pause();
    let audio: HTMLAudioElement;
    try {
      audio = new Audio(url);
    } catch (error) {
      URL.revokeObjectURL(url);
      throw error;
    }
    customAudio.set(cue, { audio, url });
    const release = () => {
      if (customAudio.get(cue)?.audio === audio) releaseCustomAudio(cue);
    };
    audio.addEventListener("ended", release, { once: true });
    audio.addEventListener("error", release, { once: true });
    await startAudio(audio, volume);
    return true;
  } catch {
    if (pendingCustomAudio.get(cue) !== ticket) return true;
    releaseCustomAudio(cue);
    if (!fallback) return false;
    try {
      await playBundledCue(cue, volume);
      return true;
    } catch {
      return false;
    }
  } finally {
    if (pendingCustomAudio.get(cue) === ticket) pendingCustomAudio.delete(cue);
  }
}

/** Queues the selected custom cue, or the bundled effect when it cannot play. */
export function playCue(cue: CueName, volumePercent: number, customPath?: string): boolean {
  const volume = soundAmplitude(volumePercent);
  if (!volume || typeof Audio === "undefined") return false;

  if (customPath) {
    void playSelectedCue(cue, customPath, volume, true);
    return true;
  }
  pendingCustomAudio.delete(cue);
  try {
    void playBundledCue(cue, volume).catch(() => {});
    return true;
  } catch {
    return false;
  }
}

/** A deliberate preview bypasses the event toggles, quiet hours and away hold. */
export async function previewSound(cue: AudibleCue, volumePercent: number, customPath?: string): Promise<boolean> {
  const name = CUE_FOR_SOUND[cue];
  const volume = soundAmplitude(volumePercent);
  if (!volume) return true;
  if (typeof Audio === "undefined") return false;
  if (customPath) return playSelectedCue(name, customPath, volume, false);
  pendingCustomAudio.delete(name);
  try {
    await playBundledCue(name, volume);
    return true;
  } catch {
    return false;
  }
}

function shouldPlayFailure(sessionId: string, now: number): boolean {
  const previous = lastFailureBySession.get(sessionId);
  for (const [id, time] of lastFailureBySession) {
    if (now - time >= FAILURE_QUIET_MS) lastFailureBySession.delete(id);
  }
  // Every failure extends the quiet window, even if that failure is silent.
  lastFailureBySession.set(sessionId, now);
  return previous === undefined || now - previous >= FAILURE_QUIET_MS;
}

/** Applies the current sound settings and queues the decided event cue. */
export function playAttentionSound(
  sound: string | null | undefined,
  settings: AppSettings,
  now: Date = new Date(),
  sessionId = "",
): boolean {
  if (!sound || !shouldPlaySound(settings, now)) return false;
  const preference = sound === "TaskRoundComplete" ? "Stop" : sound;
  if (!settings.audible_cues.includes(sound) && !settings.audible_cues.includes(preference)) return false;
  if (!Object.prototype.hasOwnProperty.call(CUE_FOR_SOUND, sound)) return false;
  const cue = CUE_FOR_SOUND[sound as keyof typeof CUE_FOR_SOUND];
  if (sound === "StopFailure" && !shouldPlayFailure(sessionId, now.getTime())) return false;
  return playCue(cue, settings.sound_volume, settings.custom_sound_paths?.[SOUND_FILES[cue]]);
}
