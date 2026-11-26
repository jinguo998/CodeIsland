import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../App";
import { IslandApp } from "../components/IslandApp";
import { useIslandStore } from "../lib/store";
import { makeSession, makeUiState } from "./factories";

describe("browser fallback", () => {
  it("hydrates the island from the mock backend", async () => {
    render(<App />);

    // Mock sessions from lib/mock.ts.
    expect(await screen.findByRole("article", { name: /atlas-api/ })).toBeInTheDocument();
    expect(screen.getByText("ALL")).toBeInTheDocument();
    expect(screen.getByText("STATUS")).toBeInTheDocument();
    expect(screen.getAllByText("NEEDS YOU").length).toBeGreaterThan(0);
  });

  it("pauses the folded mascot when the last session disappears from the expanded list", () => {
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    try {
      const ui = makeUiState({ surface: { kind: "session_list" } });
      useIslandStore.setState({ ui });
      const { container } = render(<IslandApp />);
      const surface = container.querySelector(".island-surface");
      expect(surface).toHaveAttribute("data-collapsed", "false");
      expect(surface).not.toHaveAttribute("data-mascot-paused");

      act(() => useIslandStore.setState({ ui: {
        ...ui,
        store: { ...ui.store, sessions: [], total_sessions: 0, current_session_id: null },
      } }));
      expect(surface).toHaveAttribute("data-collapsed", "true");
      expect(surface).toHaveAttribute("data-mascot-paused", "true");

      act(() => useIslandStore.setState({ ui: {
        ...ui,
        surface: { kind: "collapsed" },
        store: { ...ui.store, sessions: [makeSession({ status: "running" })] },
      } }));
      expect(surface).toHaveAttribute("data-collapsed", "true");
      expect(surface).not.toHaveAttribute("data-mascot-paused");
    } finally {
      hidden.mockRestore();
    }
  });

  it("does not animate or widen the folded strip for a completed Codex worker", () => {
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    try {
      const parent = makeSession({ session_id: "root", agent: "codex", child_session_ids: ["worker"] });
      const child = makeSession({ session_id: "worker", agent: "codex", parent_session_id: "root", status: "completed" });
      const ui = makeUiState({
        surface: { kind: "collapsed" },
        store: { sessions: [parent, child], pending: [], total_sessions: 2 },
      });
      useIslandStore.setState({ ui });
      const { container } = render(<IslandApp />);
      const surface = container.querySelector<HTMLElement>(".island-surface");
      const idleWidth = surface?.style.getPropertyValue("--island-compact-width");
      expect(idleWidth).toBeTruthy();
      expect(surface).toHaveAttribute("data-mascot-paused", "true");

      act(() => useIslandStore.setState({ ui: {
        ...ui,
        store: { ...ui.store, sessions: [{ ...parent, status: "running" }, child] },
      } }));
      expect(surface).not.toHaveAttribute("data-mascot-paused");
      expect(surface?.style.getPropertyValue("--island-compact-width")).not.toBe(idleWidth);
    } finally {
      hidden.mockRestore();
    }
  });
});
