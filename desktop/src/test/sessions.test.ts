import { describe, expect, it } from "vitest";
import { presentedSessions } from "../lib/sessions";
import { makeRequest, makeSession } from "./factories";

describe("Codex sub-sessions", () => {
  const parent = makeSession({
    session_id: "parent", agent: "codex", child_session_ids: ["child", "settled"],
  });
  const child = makeSession({
    session_id: "child", agent: "codex", parent_session_id: "parent",
    status: "running", title: "Worker", child_session_ids: ["grandchild"],
  });
  const grandchild = makeSession({
    session_id: "grandchild", agent: "codex", parent_session_id: "child", status: "running",
  });
  const settled = makeSession({
    session_id: "settled", agent: "codex", parent_session_id: "parent", status: "completed",
  });
  const unrelated = makeSession({
    session_id: "unrelated", agent: "codex", project: parent.project, status: "running",
  });
  const store = {
    sessions: [parent, child, grandchild, settled, unrelated], pending: [], total_sessions: 5,
  };

  it("keeps active children separate without counting them twice", () => {
    const { sessions } = presentedSessions(store);
    expect(sessions.map((session) => session.session_id)).toEqual(["parent", "child", "grandchild", "unrelated"]);
    expect(sessions[0].child_session_ids).toEqual([]);
    expect(sessions[1].child_session_ids).toEqual([]);
  });

  it("merges a multi-generation family without including a same-project root or settled worker", () => {
    const { sessions } = presentedSessions(store, "merge");
    expect(sessions.map((session) => session.session_id)).toEqual(["parent", "unrelated"]);
    expect(sessions[0]).toMatchObject({
      status: "running", active_tool: "Agent", active_tool_detail: "Worker",
      child_session_ids: ["child", "grandchild"],
    });
    expect(parent.child_session_ids).toEqual(["child", "settled"]);
    const throughSettled = {
      ...store, sessions: [parent, { ...child, status: "idle" as const }, grandchild],
    };
    expect(presentedSessions(throughSettled, "merge").sessions[0].child_session_ids).toEqual(["grandchild"]);
  });

  it("hides children while keeping any card that has its own request", () => {
    expect(presentedSessions(store, "hide").sessions.map((session) => session.session_id))
      .toEqual(["parent", "unrelated"]);
    const request = makeRequest({ session_id: "grandchild", agent: "codex" });
    for (const mode of ["merge", "hide"]) {
      const { sessions, requests } = presentedSessions({ ...store, pending: [request] }, mode);
      expect(sessions.map((session) => session.session_id)).toEqual(["parent", "grandchild", "unrelated"]);
      expect(requests).toEqual([request]);
    }
  });
});
