import { isActiveStatus, isWaitingStatus } from "./status";
import type { SessionSnapshot, StoreSnapshot } from "./types";

/** Cards and requests represented in the notch. */
export function presentedSessions(store: StoreSnapshot | undefined, codexMode = "separate") {
  if (!store) return { sessions: [], requests: [] };

  const byId = new Map(store.sessions.map((session) => [session.session_id, session]));
  const parentIds = new Map<string, string>();
  const foldedChildren = new Set<string>();
  for (const session of store.sessions) {
    const parent = session.parent_session_id;
    if (parent && parent !== session.session_id) parentIds.set(session.session_id, parent);
  }
  for (const session of store.sessions) {
    for (const childId of session.child_session_ids) {
      if (childId !== session.session_id && !parentIds.has(childId)) {
        parentIds.set(childId, session.session_id);
      }
    }
  }

  const pendingIds = new Set(store.pending.filter((request) => request.agent === "codex")
    .map((request) => request.session_id));
  for (const session of store.sessions) {
    const id = session.session_id;
    const parent = parentIds.get(id);
    if (!parent) continue;
    if (session.agent !== "codex") {
      if (byId.has(parent)) foldedChildren.add(id);
      continue;
    }
    // A worker with an actionable request always keeps its own card.
    if (pendingIds.has(id) || isWaitingStatus(session.status)) continue;
    if (!isActiveStatus(session.status) || codexMode === "hide" ||
        (codexMode === "merge" && byId.get(parent)?.agent === "codex")) {
      foldedChildren.add(id);
    }
  }

  const merged = new Map<string, SessionSnapshot[]>();
  if (codexMode === "merge") {
    for (const child of store.sessions) {
      const id = child.session_id;
      if (child.agent !== "codex" || !foldedChildren.has(id) || !isActiveStatus(child.status)) continue;
      // A worker can spawn its own worker. Follow explicit links to the
      // visible card, even if an intermediate worker has already settled.
      const seen = new Set([id]);
      let parent = parentIds.get(id);
      while (parent && foldedChildren.has(parent) && !seen.has(parent)) {
        seen.add(parent);
        parent = parentIds.get(parent);
      }
      if (!parent || seen.has(parent) || byId.get(parent)?.agent !== "codex") continue;
      const siblings = merged.get(parent) ?? [];
      siblings.push(child);
      merged.set(parent, siblings);
    }
  }

  return {
    sessions: store.sessions.filter((session) => !foldedChildren.has(session.session_id)).map((session) => {
      if (session.agent !== "codex") return session;
      const children = merged.get(session.session_id) ?? [];
      const childIds = session.child_session_ids.filter((id) => {
        const child = byId.get(id);
        return child && child.agent !== "codex";
      }).concat(children.map((child) => child.session_id));
      if (!children.length && childIds.length === session.child_session_ids.length) return session;
      const activeChild = children.reduce<SessionSnapshot | undefined>((latest, child) =>
        !latest || Date.parse(child.last_activity) > Date.parse(latest.last_activity) ? child : latest, undefined);
      if (!activeChild || isWaitingStatus(session.status)) return { ...session, child_session_ids: childIds };
      return {
        ...session,
        status: "running" as const,
        active_tool: session.active_tool || "Agent",
        active_tool_detail: session.active_tool ? session.active_tool_detail : activeChild.title,
        last_activity: Date.parse(activeChild.last_activity) > Date.parse(session.last_activity)
          ? activeChild.last_activity : session.last_activity,
        child_session_ids: childIds,
      };
    }),
    requests: store.pending.filter((request) => !foldedChildren.has(request.session_id)),
  };
}

/** Matches whole families so a new or missing member invalidates a confirmation. */
export function retainedIdleSessionGroups(
  requested: string[][],
  current: string[][],
): string[][] {
  const groupKey = (group: string[]) => JSON.stringify([...group].sort());
  const requestedKeys = new Set(requested.map(groupKey));
  return current.filter((group) => requestedKeys.has(groupKey(group)));
}
