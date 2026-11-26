import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RequestCard } from "../components/RequestCard";
import { useIslandStore } from "../lib/store";
import { makeRequest, makeSession, makeUiState } from "./factories";

const invokeMock = vi.fn();

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((command: string) => {
    if (command === "get_state") return Promise.resolve(makeUiState());
    return Promise.resolve(null);
  });
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    invoke: invokeMock,
  };
});

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("RequestCard", () => {
  it("invokes resolve_request when Allow once is clicked", async () => {
    const user = userEvent.setup();
    render(<RequestCard request={makeRequest()} />);

    await user.click(screen.getByRole("button", { name: /allow once/i }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "resolve_request",
        { eventId: "req-1", action: "approve" },
        undefined,
      ),
    );
  });

  it("renders a structured edit and the enabled approval shortcut", () => {
    const ui = makeUiState();
    ui.settings.approve_shortcut_enabled = true;
    ui.settings.approve_shortcut = "Ctrl+Alt+P";
    useIslandStore.setState({ ui });
    render(<RequestCard request={makeRequest({
      tool: "Edit",
      tool_input: { file_path: "C:\\dev\\demo\\main.rs", old_string: "old snippet", new_string: "new snippet" },
    })} />);

    expect(screen.getByText("Edit wants to change main.rs")).toBeVisible();
    expect(screen.getByText("old snippet")).toBeVisible();
    expect(screen.getByText("new snippet")).toBeVisible();
    expect(screen.getByText("Ctrl+Alt+P", { selector: "kbd" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Always allow Edit this session" })).toBeVisible();
  });

  it("shows the staged Codex prefixes and resolves the same request on Always", async () => {
    const user = userEvent.setup();
    render(<RequestCard request={makeRequest({
      agent: "codex",
      tool_input: { command: "git status && npm test", description: "Run checks" },
      always_scope: { kind: "codex_rules", patterns: [["git", "status"], ["npm", "test"]] },
    })} />);

    const action = screen.getByRole("button", {
      name: "Always allow commands starting with git status or npm test — saved to ~/.codex/rules",
    });
    expect(screen.getByText("Run checks")).toBeVisible();
    await user.click(action);
    expect(invokeMock).toHaveBeenCalledWith(
      "resolve_request", { eventId: "req-1", action: "always_allow" }, undefined,
    );
  });

  it("lets users inspect an inline request whose turn to decide has not arrived", async () => {
    const user = userEvent.setup();
    render(<RequestCard standalone={false} position={2} total={3} request={makeRequest({
      tool_input: { command: "npm test", description: "Verify changes" },
    })} />);

    expect(screen.getByText("Verify changes")).toBeVisible();
    expect(screen.getByRole("button", { name: "Allow once" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Always allow Bash" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Details" }));
    expect(screen.getByText("npm test")).toBeVisible();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("collapses only after a confirmed terminal jump for the visible approval", async () => {
    const user = userEvent.setup();
    const ui = makeUiState({
      surface: { kind: "approval_card", event_id: "req-1", session_id: "sess-1" },
      surface_revision: 4,
      store: { sessions: [makeSession({ pid: 314 })], pending: [], total_sessions: 1 },
    });
    ui.settings.auto_collapse_after_session_jump = true;
    useIslandStore.setState({ ui });
    invokeMock.mockResolvedValue(true);
    render(<RequestCard request={makeRequest()} />);

    await user.click(screen.getByRole("button", { name: "Click to focus this session's terminal" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("focus_session_terminal", { sessionId: "sess-1" }, undefined);
      expect(invokeMock).toHaveBeenCalledWith("set_island_surface", { surface: { kind: "collapsed" } }, undefined);
    });
    expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain("open_workspace");
  });

  it("keeps a new approval open when a previous jump finishes", async () => {
    const user = userEvent.setup();
    const ui = makeUiState({
      surface: { kind: "approval_card", event_id: "req-1", session_id: "sess-1" },
      surface_revision: 4,
      store: { sessions: [makeSession({ pid: 314 })], pending: [], total_sessions: 1 },
    });
    ui.settings.auto_collapse_after_session_jump = true;
    useIslandStore.setState({ ui });
    let finishFocus: (success: boolean) => void = () => {};
    invokeMock.mockReturnValue(new Promise<boolean>((resolve) => { finishFocus = resolve; }));
    render(<RequestCard request={makeRequest()} />);

    await user.click(screen.getByRole("button", { name: "Click to focus this session's terminal" }));
    await act(async () => {
      useIslandStore.setState({
        ui: { ...ui, surface: { kind: "approval_card", event_id: "req-new", session_id: "sess-1" }, surface_revision: 5 },
      });
      finishFocus(true);
    });
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual(["focus_session_terminal"]);
  });

  it("answers a question through resolve_request", async () => {
    const user = userEvent.setup();
    render(
      <RequestCard
        request={makeRequest({
          event_id: "q-1",
          kind: "question",
          options: ["Postgres", "SQLite"],
        })}
      />,
    );

    await user.click(screen.getByRole("button", { name: "SQLite" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "resolve_request",
        { eventId: "q-1", action: "answer", text: "SQLite" },
        undefined,
      ),
    );
  });

  it("submits checked options in provider order", async () => {
    const user = userEvent.setup();
    render(
      <RequestCard request={makeRequest({
        event_id: "multi",
        kind: "question",
        questions: [{
          question: "Which checks should run?",
          header: "Checks",
          options: ["Lint", "Tests"],
          multi_select: true,
        }],
      })} />,
    );

    await user.click(screen.getByRole("button", { name: "Tests" }));
    await user.click(screen.getByRole("button", { name: "Lint" }));
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "resolve_request",
        { eventId: "multi", action: "answer", text: JSON.stringify({ Checks: "Lint, Tests" }) },
        undefined,
      ),
    );
  });

});
