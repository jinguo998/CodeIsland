import type { SessionSnapshot, SessionStatus } from "./types";

export type StatusKey =
  | "needs_you"
  | "working"
  | "thinking"
  | "done"
  | "idle"
  | "stopped"
  | "error";

/**
 * Maps the backend session state onto the seven chips the island renders.
 * Order matters: waiting and terminal states win over transient activity flags.
 */
export function statusKeyFor(session: SessionSnapshot): StatusKey {
  switch (session.status) {
    case "waiting_permission":
    case "waiting_answer":
      return "needs_you";
    case "cancelled":
      return "stopped";
    case "failed":
      return "error";
    case "completed":
      if (session.last_turn_failed || session.error) return "error";
      return session.interrupted ? "stopped" : "done";
    case "idle":
      if (session.last_turn_failed || session.error) return "error";
      if (session.interrupted) return "stopped";
      return session.last_message || session.last_prompt ? "done" : "idle";
    case "running":
      return session.is_executing_tool || session.active_tool
        ? "working"
        : "thinking";
    case "starting":
      return "thinking";
    default:
      return "thinking";
  }
}

export function statusLabelKey(status: StatusKey): string {
  return `status.${status}`;
}

/** Mirrors `SessionStatus::is_active()` in the Rust domain. */
export function isActiveStatus(status: SessionStatus): boolean {
  return status === "starting" || status === "running";
}

export function isWaitingStatus(status: SessionStatus): boolean {
  return status === "waiting_permission" || status === "waiting_answer";
}

export function isTerminalStatus(status: SessionStatus): boolean {
  return (
    status === "completed" || status === "failed" || status === "cancelled"
  );
}

export function isNeedsYou(session: SessionSnapshot): boolean {
  return isWaitingStatus(session.status);
}
