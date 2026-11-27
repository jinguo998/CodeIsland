import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SessionCard } from "../components/SessionCard";
import { DEFAULT_SETTINGS } from "../lib/defaults";
import * as ipc from "../lib/ipc";
import * as sound from "../lib/sound";
import { useIslandStore } from "../lib/store";
import type { SessionSnapshot } from "../lib/types";
import { makeSession, makeUiState } from "./factories";

const CASES: Array<[string, SessionSnapshot, string]> = [
  ["waiting for permission", makeSession({ status: "waiting_permission" }), "NEEDS YOU"],
  ["waiting for an answer", makeSession({ status: "waiting_answer" }), "NEEDS YOU"],
  [
    "executing a tool",
    makeSession({ status: "running", is_executing_tool: true, active_tool: "Edit" }),
    "WORKING",
  ],
  ["thinking", makeSession({ status: "running" }), "THINKING"],
  ["completed", makeSession({ status: "completed" }), "DONE"],
  ["idle", makeSession({ status: "idle", last_message: null }), "IDLE"],
  ["restored idle with a prior reply", makeSession({ status: "idle" }), "DONE"],
  ["cancelled", makeSession({ status: "cancelled" }), "STOPPED"],
  ["failed", makeSession({ status: "failed" }), "ERROR"],
  [
    "completed with a failed turn",
    makeSession({ status: "completed", last_turn_failed: true }),
    "ERROR",
  ],
];

describe("SessionCard", () => {
  for (const [name, session, label] of CASES) {
    it(`renders the ${label} chip for ${name}`, () => {
      render(<SessionCard session={session} now={Date.now()} />);
      expect(screen.getByText(label)).toBeInTheDocument();
    });
  }

  it("shows project, terminal, model, reply and tool", () => {
    useIslandStore.setState({ ui: makeUiState({ settings: { ...DEFAULT_SETTINGS, show_model_label: true } }) });
    const session = makeSession({
      status: "running",
      is_executing_tool: true,
      active_tool: "Edit",
      model: "claude-sonnet-4-5",
      model_label: "Sonnet 4.5 · high",
      reasoning: "high",
      terminal_kind: "Windows Terminal",
    });
    render(<SessionCard session={session} now={Date.now()} />);

    expect(screen.getByRole("article", { name: "demo — Claude Code" })).toBeInTheDocument();
    expect(screen.getByText("demo")).toBeInTheDocument();
    expect(screen.getByText("Windows Terminal")).toBeInTheDocument();
    expect(screen.getByText("Hello from the island.")).toBeInTheDocument();
    expect(screen.getByText("▸ Edit")).toBeInTheDocument();
    expect(screen.getByText("Sonnet 4.5 · high")).toBeInTheDocument();
  });

  it("uses the session title or agent when project names are hidden", () => {
    useIslandStore.setState({ ui: makeUiState({ settings: { ...DEFAULT_SETTINGS, show_project_name: false } }) });
    const session = makeSession({ project: "private-project", title: "Refactor UI" });
    const { rerender } = render(<SessionCard session={session} now={Date.now()} />);
    expect(screen.getByText("Refactor UI")).toBeInTheDocument();
    expect(screen.queryByText("private-project")).not.toBeInTheDocument();

    rerender(<SessionCard session={{ ...session, title: null }} now={Date.now()} />);
    expect(screen.getByText("Claude Code")).toBeInTheDocument();
    expect(screen.queryByText("private-project")).not.toBeInTheDocument();
  });

  it("marks a parent card with its child-session count", () => {
    const session = makeSession({ child_session_ids: ["child-a", "child-b"] });
    render(<SessionCard session={session} now={Date.now()} />);

    expect(screen.getByText("+2 Sub")).toBeInTheDocument();
  });

  it("applies tool visibility to an already visible compact row", () => {
    const session = makeSession({ status: "running", active_tool: "Edit", last_prompt: "Original prompt" });
    const ui = makeUiState({ settings: { ...DEFAULT_SETTINGS, density: "compact", show_tool_status: false } });
    useIslandStore.setState({ ui });
    render(<SessionCard session={session} now={Date.now()} />);
    expect(screen.getByText("Original prompt")).toBeVisible();
    expect(screen.queryByText("▸ Edit")).not.toBeInTheDocument();

    act(() => useIslandStore.setState({ ui: { ...ui, settings: { ...ui.settings, show_tool_status: true } } }));
    expect(screen.getByText("▸ Edit")).toBeVisible();
  });

  it.each(["comfortable", "compact"] as const)("jumps to the exact session from the %s card", async (density) => {
    const focus = vi.spyOn(ipc, "focusSessionTerminal").mockResolvedValue(true);
    const collapse = vi.spyOn(ipc, "setIslandSurface").mockResolvedValue(true);
    try {
      const session = makeSession({ session_id: "codexapp:selected", provider_session_id: "provider-id", agent: "codex" });
      const ui = makeUiState({ surface: { kind: "session_list" }, surface_revision: 4 });
      ui.settings.density = density;
      useIslandStore.setState({ ui });
      render(<SessionCard session={session} now={Date.now()} />);

      fireEvent.click(screen.getByText("demo"));
      expect(focus).toHaveBeenLastCalledWith(session.session_id);
      expect(collapse).not.toHaveBeenCalled();

      act(() => useIslandStore.setState({ ui: {
        ...ui, settings: { ...ui.settings, auto_collapse_after_session_jump: true },
      } }));
      fireEvent.keyDown(screen.getByRole("article"), { key: "Enter" });
      expect(focus).toHaveBeenCalledTimes(2);
      await waitFor(() => expect(collapse).toHaveBeenCalledExactlyOnceWith({ kind: "collapsed" }));
    } finally {
      vi.restoreAllMocks();
    }
  });

  it("leaves interactive descendants, dragging and remote cards to their own controls", () => {
    const focus = vi.spyOn(ipc, "focusSessionTerminal").mockResolvedValue(true);
    const onRemove = vi.fn();
    const onApprove = vi.fn();
    const onFollowLink = vi.fn();
    const session = makeSession();
    const now = Date.now();
    try {
      const { rerender } = render(
        <SessionCard session={session} now={now} onRemove={onRemove}>
          <button type="button" onClick={onApprove}>Approve test request</button>
          <a href="#approval-details" onClick={onFollowLink}>Approval details link</a>
        </SessionCard>,
      );
      const row = within(screen.getByRole("article"));
      const approve = row.getByRole("button", { name: "Approve test request" });
      fireEvent.keyDown(approve, { key: "Enter" });
      fireEvent.click(approve);
      fireEvent.click(row.getByRole("button", { name: "Remove demo" }));
      fireEvent.click(row.getByRole("link", { name: "Approval details link" }));
      fireEvent.click(row.getByRole("button", { name: "Drag to reorder demo" }));
      expect(onApprove).toHaveBeenCalledTimes(1);
      expect(onRemove).toHaveBeenCalledTimes(1);
      expect(onFollowLink).toHaveBeenCalledTimes(1);

      rerender(<SessionCard session={session} now={now} dragging />);
      fireEvent.click(screen.getByText("demo"));
      rerender(<SessionCard session={{ ...session, session_id: "ssh:1:host:remote" }} now={now} />);
      fireEvent.click(screen.getByText("demo"));
      expect(screen.getByRole("article")).not.toHaveAttribute("tabindex");
      expect(focus).not.toHaveBeenCalled();
    } finally {
      vi.restoreAllMocks();
    }
  });

  it("keeps the surface on a failed jump and ignores completion from an older surface", async () => {
    const focus = vi.spyOn(ipc, "focusSessionTerminal").mockResolvedValueOnce(false);
    const collapse = vi.spyOn(ipc, "setIslandSurface").mockResolvedValue(true);
    const preview = vi.spyOn(sound, "playCue").mockReturnValue(true);
    try {
      const ui = makeUiState({ surface: { kind: "session_list" }, surface_revision: 4 });
      ui.settings.auto_collapse_after_session_jump = true;
      useIslandStore.setState({ ui });
      render(<SessionCard session={makeSession()} now={Date.now()} />);
      const card = screen.getByRole("article");
      fireEvent.click(card);
      await waitFor(() => expect(card).toHaveAttribute("data-jump-failed", "true"));
      expect(preview).toHaveBeenCalledTimes(1);
      expect(collapse).not.toHaveBeenCalled();

      let confirmFocus!: (focused: boolean) => void;
      focus.mockImplementationOnce(() => new Promise((resolve) => { confirmFocus = resolve; }));
      fireEvent.click(card);
      act(() => useIslandStore.setState({ ui: {
        ...ui, surface: { kind: "approval_card", event_id: "new", session_id: "sess-1" }, surface_revision: 5,
      } }));
      await act(async () => confirmFocus(true));
      expect(collapse).not.toHaveBeenCalled();
    } finally {
      vi.restoreAllMocks();
    }
  });
});
