import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CollapsedStrip } from "../components/CollapsedStrip";
import { compactMarkdownPreview } from "../lib/compactMarkdown";
import { useIslandStore } from "../lib/store";
import { makeRequest, makeSession, makeUiState } from "./factories";

describe("compact session rotation", () => {
  it("rotates active sessions by urgency, snaps when work updates and clears stale tool text", () => {
    vi.useFakeTimers();
    try {
      const approval = makeSession({
        session_id: "approve", project: "Approval", agent: "claude", status: "waiting_permission",
      });
      const question = makeSession({
        session_id: "answer", project: "Question", agent: "codex", status: "waiting_answer",
      });
      const running = makeSession({
        session_id: "working", project: "Running", agent: "gemini", status: "running", active_tool: "Read",
      });
      const thinking = makeSession({
        session_id: "thinking", project: "Thinking", agent: "cursor", status: "running",
      });
      const ui = makeUiState({
        store: {
          sessions: [thinking, running, question, approval],
          pending: [],
          total_sessions: 4,
          current_session_id: thinking.session_id,
        },
      });
      ui.settings.rotation_interval = 3;
      useIslandStore.setState({ ui });

      render(<CollapsedStrip />);
      expect(screen.getByText("Approval")).toBeVisible();

      act(() => vi.advanceTimersByTime(3000));
      expect(screen.getByText("Question")).toBeVisible();

      act(() => vi.advanceTimersByTime(3000));
      expect(screen.getByText("Running")).toBeVisible();
      expect(screen.getByText("Read")).toBeVisible();

      act(() => useIslandStore.setState({
        ui: {
          ...ui,
          store: { ...ui.store, sessions: [...ui.store.sessions] },
        },
      }));
      expect(screen.getByText("Approval")).toBeVisible();
      expect(screen.queryByText("Read")).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("uses the configured idle mascot after the last active session finishes", () => {
    const session = makeSession({ status: "completed", agent: "claude" });
    const ui = makeUiState({
      store: { sessions: [session], pending: [], total_sessions: 1 },
    });
    ui.settings.default_mascot = "codex";
    useIslandStore.setState({ ui });

    render(<CollapsedStrip />);
    const mascot = screen.getByRole("img", { name: "CodeIsland mascot" });
    expect(mascot.closest(".island-compact-mascot")).toHaveAttribute("data-agent", "codex");
    expect(mascot).toHaveClass("mascot--idle");
  });

  it("applies a new rotation interval while multiple sessions are still active", () => {
    vi.useFakeTimers();
    try {
      const sessions = [
        makeSession({ session_id: "one", project: "First project", status: "running" }),
        makeSession({ session_id: "two", project: "Second project", status: "running" }),
      ];
      const ui = makeUiState({ store: { sessions, pending: [], total_sessions: 2 } });
      ui.settings.rotation_interval = 10;
      useIslandStore.setState({ ui });
      render(<CollapsedStrip />);
      expect(screen.getByText("First project")).toBeVisible();
      act(() => vi.advanceTimersByTime(1000));

      act(() => useIslandStore.setState({ ui: { ...ui, settings: { ...ui.settings, rotation_interval: 3 } } }));
      act(() => vi.advanceTimersByTime(2999));
      expect(screen.getByText("First project")).toBeVisible();
      act(() => vi.advanceTimersByTime(1));
      expect(screen.getByText("Second project")).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });

  it("drops completed Codex workers from compact rotation and shows running workers", () => {
    vi.useFakeTimers();
    try {
      const parent = makeSession({ session_id: "root", project: "Parent", agent: "codex", child_session_ids: ["worker"] });
      const child = makeSession({
        session_id: "worker", parent_session_id: "root", project: "Worker", agent: "codex", status: "completed",
      });
      const other = makeSession({ session_id: "main", project: "Current project", status: "running" });
      const ui = makeUiState({ store: {
        sessions: [child, parent, other], pending: [], total_sessions: 3, current_session_id: child.session_id,
      } });
      ui.settings.rotation_interval = 3;
      useIslandStore.setState({ ui });
      render(<CollapsedStrip />);

      expect(screen.getByText("Current project")).toBeVisible();
      expect(screen.getByLabelText("2 sessions")).toHaveTextContent("1/2");
      act(() => vi.advanceTimersByTime(6000));
      expect(screen.getByText("Current project")).toBeVisible();
      expect(screen.queryByText("Worker")).not.toBeInTheDocument();

      act(() => useIslandStore.setState({ ui: {
        ...ui,
        store: { ...ui.store, sessions: [{ ...child, status: "running", active_tool: "Read" }, parent, other] },
      } }));
      expect(screen.getByText("Worker")).toBeVisible();
      expect(screen.getByLabelText("3 sessions")).toHaveTextContent("2/3");
    } finally {
      vi.useRealTimers();
    }
  });

  it("uses the session title in private mode and lets the latest tool detail linger", () => {
    vi.useFakeTimers();
    try {
      const session = makeSession({
        status: "running", agent: "codex", project: "private-project", cwd: "C:\\work\\private-project",
        title: "Short session title", active_tool: "Edit", active_tool_detail: "src\\lib\\store.ts",
      });
      const ui = makeUiState({ store: { sessions: [session], pending: [], total_sessions: 1 } });
      ui.settings.show_project_name = false;
      useIslandStore.setState({ ui });

      render(<CollapsedStrip />);
      expect(screen.getByText("Short session title")).toBeVisible();
      expect(screen.queryByText("private-project")).not.toBeInTheDocument();
      expect(screen.getByText("Edit")).toHaveAttribute("data-tone", "edit");
      expect(screen.getByText("store.ts")).toBeVisible();

      act(() => useIslandStore.setState({ ui: {
        ...ui,
        store: { ...ui.store, sessions: [{ ...session, title: null, active_tool: null, active_tool_detail: null }] },
      } }));
      act(() => vi.advanceTimersByTime(1999));
      expect(screen.getByText("store.ts")).toBeVisible();
      act(() => vi.advanceTimersByTime(1));
      expect(screen.queryByText("store.ts")).not.toBeInTheDocument();
      expect(screen.queryByText("private-project")).not.toBeInTheDocument();
      expect(screen.getByText("thinking")).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });

  it("advertises a dismissed question only while its card is off screen", () => {
    const parent = makeSession({ session_id: "parent", agent: "codex", child_session_ids: ["sess-1"] });
    const session = makeSession({ status: "waiting_answer", agent: "codex", parent_session_id: parent.session_id });
    const question = makeRequest({ kind: "question", hidden: true, agent: "codex" });
    const ui = makeUiState({
      surface: { kind: "collapsed" },
      store: { sessions: [parent, session], pending: [question], total_sessions: 2 },
    });
    useIslandStore.setState({ ui });
    render(<CollapsedStrip />);

    expect(screen.getByRole("button", { name: "Question waiting" })).toBeVisible();
    expect(screen.getByLabelText("2 sessions")).toHaveTextContent("1/2");
    act(() => useIslandStore.setState({ ui: {
      ...ui,
      surface: { kind: "question_card", event_id: question.event_id, session_id: session.session_id },
    } }));
    expect(screen.queryByRole("button", { name: "Question waiting" })).not.toBeInTheDocument();
  });

  it("previews Codex live output as text until a tool takes over", () => {
    const session = makeSession({
      agent: "codex", status: "running", live_codex_output: "## Checking\n\n- **tests**\n- `npm run build`",
    });
    const ui = makeUiState({ store: { sessions: [session], pending: [], total_sessions: 1 } });
    useIslandStore.setState({ ui });
    render(<CollapsedStrip />);

    expect(screen.getByText("Checking · tests · npm run build")).toBeVisible();
    expect(screen.queryByText("thinking")).not.toBeInTheDocument();
    act(() => useIslandStore.setState({ ui: {
      ...ui,
      store: { ...ui.store, sessions: [{ ...session, active_tool: "Bash" }] },
    } }));
    expect(screen.getByText("Bash")).toHaveAttribute("data-tone", "command");
    expect(screen.queryByText("Checking · tests · npm run build")).not.toBeInTheDocument();
  });

  it("keeps whole graphemes and removes table and code block syntax", () => {
    expect(compactMarkdownPreview("| Check | Result |\n| --- | --- |\n| Build | Pass |\n\n```sh\nnpm test\nnpm run build\n```"))
      .toBe("Check, Result · Build, Pass · npm test …");
    const joined = "👩‍💻";
    expect(compactMarkdownPreview(joined.repeat(161))).toBe(joined.repeat(159) + "…");
  });
});
