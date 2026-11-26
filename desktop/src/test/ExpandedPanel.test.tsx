import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ExpandedPanel } from "../components/ExpandedPanel";
import { IslandHeaderControls } from "../components/IslandHeaderControls";
import * as ipc from "../lib/ipc";
import { useIslandStore } from "../lib/store";
import { makeRequest, makeSession, makeUiState } from "./factories";

describe("notch panel surfaces", () => {
  it("uses the visible-session setting as a scroll budget and keeps every card", () => {
    const one = makeUiState();
    useIslandStore.setState({
      ui: {
        ...one,
        settings: { ...one.settings, max_visible_sessions: 1 },
        store: {
          ...one.store,
          sessions: [
            makeSession({ session_id: "a", project: "first-project" }),
            makeSession({ session_id: "b", project: "second-project" }),
          ],
        },
      },
    });
    const { container } = render(<ExpandedPanel />);
    expect(screen.getByText("first-project")).toBeInTheDocument();
    expect(screen.getByText("second-project")).toBeInTheDocument();
    expect(container.querySelector(".island-shell")?.getAttribute("style"))
      .toContain("--island-session-budget: 90px");
  });

  it("lets the fixed panel viewport determine the height for Unlimited", () => {
    const ui = makeUiState();
    const sessions = Array.from({ length: 24 }, (_, index) => makeSession({
      session_id: `s${index}`, project: `project-${index}`,
    }));
    useIslandStore.setState({
      ui: {
        ...ui,
        settings: { ...ui.settings, max_visible_sessions: 99, ai_message_lines: 0 },
        store: { ...ui.store, sessions },
      },
    });
    const { container } = render(<ExpandedPanel />);
    expect(screen.getAllByRole("article")).toHaveLength(24);
    expect(container.querySelector(".island-shell")?.getAttribute("style"))
      .toContain("--island-session-budget: 100vh");
    expect(container.querySelector<HTMLElement>(".island-shell")?.style.getPropertyValue("--ai-message-lines")).toBe("none");
  });

  it.each(["comfortable", "compact"] as const)("keeps the %s mascot scenes tied to session status across updates", (density) => {
    const sessions = [
      makeSession({ session_id: "rest", project: "Resting", agent: "claude", status: "idle", last_message: null }),
      makeSession({ session_id: "done", project: "Finished", agent: "codex", status: "completed" }),
      makeSession({ session_id: "work", project: "Working", agent: "gemini", status: "running", active_tool: "Edit" }),
      makeSession({ session_id: "wait", project: "Waiting", agent: "cursor", status: "waiting_answer" }),
    ];
    const ui = makeUiState({ store: { sessions, total_sessions: sessions.length, pending: [] } });
    ui.settings.density = density;
    useIslandStore.setState({ ui });
    render(<ExpandedPanel />);

    const mascotFor = (project: string) => {
      const row = screen.getByRole("article", { name: new RegExp(`^${project} —`) });
      return within(row).getByRole("img", { name: "CodeIsland mascot" }).querySelector("canvas");
    };
    const initialScenes = sessions.map((session) => mascotFor(session.project ?? session.session_id));
    expect(initialScenes.map((canvas) => canvas?.dataset.pose)).toEqual(["idle", "idle", "working", "waiting"]);

    const publish = (workingSettled: boolean) => act(() => useIslandStore.setState({ ui: {
      ...ui,
      store: { ...ui.store, sessions: sessions.map((session) => workingSettled && session.session_id === "work"
        ? { ...session, status: "completed", active_tool: null }
        : { ...session }) },
    } }));
    publish(false);
    publish(false);
    sessions.forEach((session, index) => expect(mascotFor(session.project ?? session.session_id)).toBe(initialScenes[index]));

    publish(true);
    expect(mascotFor("Working")).toBe(initialScenes[2]);
    expect(mascotFor("Working")).toHaveAttribute("data-pose", "idle");
  });

  it("renders one addressed approval and hides tabs for one session", () => {
    const ui = makeUiState();
    const onScreen = makeRequest({ event_id: "shown", title: "Addressed approval" });
    const queued = makeRequest({ event_id: "queued", title: "Queued approval" });
    useIslandStore.setState({ ui: { ...ui, store: { ...ui.store, pending: [queued, onScreen] } } });
    render(<ExpandedPanel surface={{ kind: "approval_card", event_id: "shown", session_id: "sess-1" }} />);
    expect(screen.getByText("Addressed approval", { selector: "span" })).toBeInTheDocument();
    expect(screen.queryByText("Queued approval")).not.toBeInTheDocument();
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("keeps a linked Codex child's approval actionable until it is answered and removes the finished card", () => {
    const parent = makeSession({ session_id: "root", project: "Parent", agent: "codex", child_session_ids: ["worker"] });
    const child = makeSession({
      session_id: "worker", project: "Worker", agent: "codex", parent_session_id: parent.session_id, status: "waiting_permission",
    });
    const approval = makeRequest({ session_id: child.session_id, title: "Worker approval", agent: "codex" });
    const ui = makeUiState({ store: { sessions: [parent, child], pending: [approval], total_sessions: 2 } });
    useIslandStore.setState({ ui });
    const { rerender } = render(<ExpandedPanel />);
    expect(within(screen.getByRole("article", { name: /^Worker —/ }))
      .getByRole("button", { name: "Allow once" })).toBeVisible();

    rerender(<ExpandedPanel surface={{ kind: "approval_card", event_id: approval.event_id, session_id: child.session_id }} />);
    expect(screen.getByText("Worker approval", { selector: "span" })).toBeVisible();

    act(() => useIslandStore.setState({ ui: {
      ...ui,
      store: { ...ui.store, sessions: [parent, { ...child, status: "completed" }], pending: [] },
    } }));
    rerender(<ExpandedPanel />);
    expect(screen.queryByRole("article", { name: /^Worker —/ })).not.toBeInTheDocument();
    expect(screen.getByRole("article", { name: /^Parent —/ })).toBeVisible();
  });

  it("keeps completion links and buttons independent of click-to-jump", async () => {
    const focus = vi.spyOn(ipc, "focusSessionTerminal").mockResolvedValue(true);
    const workspace = vi.spyOn(ipc, "openWorkspace").mockResolvedValue(true);
    const collapse = vi.spyOn(ipc, "setIslandSurface").mockResolvedValue(true);
    const remote = makeSession({ session_id: "ssh:3:dev:done", project: "remote-project" });
    const local = makeSession({
      session_id: "local-done", project: "local-project", last_message: "Completed report\n\n[Read details](#details)",
    });
    const remoteSurface = { kind: "completion_card", session_id: remote.session_id } as const;
    const localSurface = { kind: "completion_card", session_id: local.session_id } as const;
    const ui = makeUiState({ surface: remoteSurface, surface_revision: 3 });
    ui.settings.auto_collapse_after_session_jump = true;
    ui.store.sessions = [remote, local];
    useIslandStore.setState({ ui });
    try {
      const { rerender } = render(<ExpandedPanel surface={remoteSurface} />);
      fireEvent.click(screen.getByText("remote-project"));
      expect(screen.queryByRole("button", { name: /Open workspace/ })).not.toBeInTheDocument();

      act(() => useIslandStore.setState({ ui: { ...ui, surface: localSurface, surface_revision: 4 } }));
      rerender(<ExpandedPanel surface={localSurface} />);
      fireEvent.click(screen.getByRole("button", { name: "Open workspace local-project" }));
      fireEvent.click(screen.getByRole("link", { name: "Read details" }));
      expect(workspace).toHaveBeenCalledExactlyOnceWith(local.session_id);
      expect(focus).not.toHaveBeenCalled();

      fireEvent.click(screen.getByText("Completed report"));
      expect(focus).toHaveBeenCalledExactlyOnceWith(local.session_id);
      await waitFor(() => expect(collapse).toHaveBeenCalledExactlyOnceWith({ kind: "collapsed" }));
    } finally {
      vi.restoreAllMocks();
    }
  });

  it("marks only the captured idle families that still match at confirmation time", async () => {
    const family = makeSession({ session_id: "parent", project: "family" });
    const child = makeSession({ session_id: "child", parent_session_id: "parent" });
    const standalone = makeSession({ session_id: "single", project: "standalone" });
    const newIdle = makeSession({ session_id: "new", project: "unrelated" });
    const captured = [["child", "parent"], ["single"]];
    const ui = makeUiState({ idle_session_removal_groups: captured });
    useIslandStore.setState({
      ui: { ...ui, store: { ...ui.store, sessions: [family, child, standalone] } },
    });
    const remove = vi.spyOn(ipc, "removeIdleSessions").mockResolvedValue(1);
    const refresh = vi.spyOn(ipc, "getState").mockResolvedValue(ui);
    try {
      render(<ExpandedPanel />);
      const trash = screen.getByRole("button", { name: "Remove idle sessions" });
      fireEvent.click(trash);
      expect(trash).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByText("Remove 3 idle cards?", { selector: ".idle-session-removal-copy > div" })).toBeVisible();
      expect(screen.getByRole("article", { name: /family/ })).toHaveAttribute("aria-description", "Will be removed");

      act(() => useIslandStore.setState({
        ui: {
          ...ui,
          idle_session_removal_groups: [["new"], ["single"]],
          store: { ...ui.store, sessions: [family, { ...child, status: "waiting_answer" }, standalone, newIdle] },
        },
      }));
      expect(screen.getByText("Remove 1 idle card?", { selector: ".idle-session-removal-copy > div" })).toBeVisible();
      expect(screen.getByRole("article", { name: /family/ })).not.toHaveAttribute("data-marked-for-removal");
      expect(screen.getByRole("article", { name: /unrelated/ })).not.toHaveAttribute("data-marked-for-removal");
      expect(screen.getByRole("article", { name: /standalone/ })).toHaveAttribute("data-marked-for-removal", "true");
      expect(remove).not.toHaveBeenCalled();
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));
      await waitFor(() => expect(trash).not.toBeDisabled());
      expect(remove).toHaveBeenCalledExactlyOnceWith(captured);
    } finally {
      refresh.mockRestore();
      remove.mockRestore();
    }
  });

  it("cancels idle removal on Escape, invalidation, and leaving the session list", () => {
    const ui = makeUiState({ idle_session_removal_groups: [["sess-1"]] });
    useIslandStore.setState({ ui });
    const { rerender } = render(<ExpandedPanel />);
    const open = () => fireEvent.click(screen.getByRole("button", { name: "Remove idle sessions" }));

    open();
    const keep = screen.getByRole("button", { name: "Keep" });
    fireEvent.keyDown(keep, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "Keep" })).not.toBeInTheDocument();

    open();
    act(() => useIslandStore.setState({ ui: { ...ui, idle_session_removal_groups: [] } }));
    act(() => useIslandStore.setState({ ui }));
    expect(screen.queryByRole("button", { name: "Keep" })).not.toBeInTheDocument();

    open();
    rerender(<ExpandedPanel surface={{ kind: "completion_card", session_id: "sess-1" }} />);
    expect(screen.queryByRole("button", { name: "Remove idle sessions" })).not.toBeInTheDocument();
    rerender(<ExpandedPanel />);
    expect(screen.queryByRole("button", { name: "Keep" })).not.toBeInTheDocument();
  });

  it("shows a quit confirmation, disarms on leave and checks its real deadline", () => {
    const clock = vi.spyOn(performance, "now").mockReturnValue(100);
    const quit = vi.spyOn(ipc, "quitApp").mockResolvedValue();
    try {
      render(<IslandHeaderControls />);
      fireEvent.click(screen.getByRole("button", { name: "Quit" }));
      const pill = screen.getByRole("button", { name: "Click again to quit" });
      expect(within(pill).getByText("QUIT?")).toBeVisible();
      expect(screen.getByRole("status")).toHaveTextContent("Click again to quit");

      clock.mockReturnValue(3200);
      fireEvent.click(pill);
      expect(quit).not.toHaveBeenCalled();

      fireEvent.mouseLeave(pill);
      fireEvent.click(screen.getByRole("button", { name: "Quit" }));
      fireEvent.click(screen.getByRole("button", { name: "Click again to quit" }));
      expect(quit).toHaveBeenCalledTimes(1);
    } finally {
      clock.mockRestore();
      quit.mockRestore();
    }
  });
});
