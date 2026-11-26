import { isTerminalStatus } from "./status";
import type { SessionSnapshot } from "./types";

export function parseIso(value: string | null | undefined): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

/** Compact elapsed label: `12s`, `3m 04s`, `1h 05m`. */
export function formatElapsed(startIso: string, endMs: number = Date.now()): string {
  const start = parseIso(startIso);
  if (start === null) return "—";
  const total = Math.max(0, Math.floor((endMs - start) / 1000));
  if (total < 60) return `${total}s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) {
    const seconds = total % 60;
    return seconds ? `${minutes}m ${String(seconds).padStart(2, "0")}s` : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours}h ${String(rest).padStart(2, "0")}m` : `${hours}h`;
}

/**
 * Active sessions count up to now; terminal sessions show how long the turn
 * itself took (started_at → last_activity).
 */
export function sessionElapsed(session: SessionSnapshot, now: number = Date.now()): string {
  const end = isTerminalStatus(session.status)
    ? (parseIso(session.last_activity) ?? now)
    : now;
  return formatElapsed(session.started_at, end);
}

/** Two-line-ish preview: collapse whitespace and cap the character count. */
export function previewText(value: string | null | undefined, max = 180): string {
  if (!value) return "";
  const normalized = value.replace(/\s+/g, " ").trim();
  if (normalized.length <= max) return normalized;
  return `${normalized.slice(0, max - 1)}…`;
}

