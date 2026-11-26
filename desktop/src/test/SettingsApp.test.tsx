import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import * as ipc from "../lib/ipc";
import { useIslandStore } from "../lib/store";
import type { AppSettings } from "../lib/types";
import { SettingsApp } from "../settings/SettingsApp";
import { makeUiState } from "./factories";

describe("SettingsApp", () => {
  it("renders every tab and opens the behavior panel", async () => {
    useIslandStore.setState({ ui: makeUiState(), ready: true });
    const user = userEvent.setup();
    render(<SettingsApp />);

    expect(
      await screen.findByRole("tab", { name: "General" }),
    ).toBeInTheDocument();

    for (const name of [
      "General",
      "Behavior",
      "Appearance",
      "Sound",
      "Hooks",
      "Shortcuts",
      "About",
    ]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }

    await user.click(screen.getByRole("tab", { name: "Behavior" }));
    expect(screen.getByLabelText("Max visible sessions")).toBeInTheDocument();
    expect(screen.getByLabelText("Tool History Limit")).toHaveValue("20");
  });

  it("renders the appearance, mascots, sound and general pages", async () => {
    const ui = makeUiState();
    ui.settings.sound_enabled = true;
    useIslandStore.setState({ ui, ready: true });
    const user = userEvent.setup();
    render(<SettingsApp />);

    await user.click(screen.getByRole("tab", { name: "Appearance" }));
    expect(screen.getByLabelText("Content font size")).toBeInTheDocument();
    expect(screen.getByLabelText("AI message lines")).toBeInTheDocument();
    expect(screen.getByLabelText("Show Claude quota")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Mascots" }));
    expect(screen.getByLabelText("Default mascot")).toBeInTheDocument();
    expect(screen.getByText("Clawd")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Sound" }));
    expect(screen.getByLabelText("Volume")).toBeInTheDocument();
    expect(screen.getByLabelText("Mute when away")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "General" }));
    expect(screen.getByLabelText("Display")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Allow horizontal dragging"),
    ).toBeInTheDocument();
  });

  it("saves immediately, coalesces later edits, and keeps unrelated incoming settings", async () => {
    const ui = makeUiState();
    useIslandStore.setState({ ui, ready: true });
    let finishFirst!: (settings: AppSettings) => void;
    const save = vi.spyOn(ipc, "patchSettings")
      .mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve; }))
      .mockResolvedValueOnce({ ...ui.settings, density: "compact" });
    try {
      const { unmount } = render(<SettingsApp />);
      fireEvent.click(screen.getByLabelText("Allow horizontal dragging"));
      expect(save).toHaveBeenCalledExactlyOnceWith({ allow_horizontal_drag: true });
      expect(screen.getByLabelText("Allow horizontal dragging")).toBeChecked();

      fireEvent.click(screen.getByRole("tab", { name: "Appearance" }));
      const density = screen.getByLabelText("Density");
      for (const value of ["compact", "comfortable", "compact"]) fireEvent.change(density, { target: { value } });
      expect(density).toHaveValue("compact");
      expect(save).toHaveBeenCalledTimes(1);

      act(() => useIslandStore.setState({ ui: { ...ui, settings: { ...ui.settings, startup: true } } }));
      fireEvent.click(screen.getByRole("tab", { name: "General" }));
      expect(screen.getByLabelText("Launch at startup")).toBeChecked();
      expect(screen.getByLabelText("Allow horizontal dragging")).toBeChecked();
      unmount();

      await act(async () => finishFirst({ ...ui.settings, allow_horizontal_drag: true }));
      expect(save).toHaveBeenCalledTimes(2);
      expect(save).toHaveBeenLastCalledWith({ density: "compact" });
      expect(useIslandStore.getState().ui?.settings).toMatchObject({
        allow_horizontal_drag: true, startup: true, density: "compact",
      });
      expect(useIslandStore.getState().settingsSaveStatus).toBe("saved");
    } finally {
      vi.restoreAllMocks();
    }
  });

  it("reports a failed save and retries the same draft", async () => {
    const ui = makeUiState();
    useIslandStore.setState({ ui, ready: true });
    const save = vi.spyOn(ipc, "patchSettings")
      .mockRejectedValueOnce(new Error("write failed"))
      .mockResolvedValueOnce({ ...ui.settings, allow_horizontal_drag: true });
    try {
      render(<SettingsApp />);
      const horizontalDrag = screen.getByLabelText("Allow horizontal dragging");
      fireEvent.click(horizontalDrag);
      expect(await screen.findByText("Could not save settings")).toBeVisible();
      expect(horizontalDrag).toBeChecked();
      expect(screen.queryByText("Saved")).not.toBeInTheDocument();

      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
      expect(await screen.findByText("Saved")).toBeVisible();
      expect(save.mock.calls).toEqual([[{ allow_horizontal_drag: true }], [{ allow_horizontal_drag: true }]]);
    } finally {
      vi.restoreAllMocks();
    }
  });

  it("uses native display identities and keeps the legacy pin selected", async () => {
    const first = { id: "panel-a", name: "Display A", legacyId: "legacy-a", width: 2560, height: 1440, isPrimary: true };
    const second = { id: "panel-b", name: "Display B", legacyId: "legacy-b", width: 1920, height: 1080, isPrimary: false };
    const ui = makeUiState();
    Object.assign(ui.settings, { display_choice: "display:legacy-a", display_mode: "pinned", pinned_display: "legacy-a" });
    useIslandStore.setState({ ui, ready: true });
    vi.spyOn(ipc, "listDisplays").mockResolvedValue([first, second]);
    const save = vi.spyOn(ipc, "patchSettings").mockImplementation(async (patch) => ({ ...ui.settings, ...patch }));
    try {
      render(<SettingsApp />);
      const display = screen.getByLabelText("Display");
      await waitFor(() => expect(display).toHaveValue("display:panel-a"));
      fireEvent.change(display, { target: { value: "display:panel-b" } });
      expect(save).toHaveBeenCalledExactlyOnceWith({
        display_choice: "display:panel-b", display_mode: "pinned", pinned_display: "panel-b",
      });
      expect(await screen.findByText("Saved")).toBeVisible();
    } finally {
      vi.restoreAllMocks();
    }
  });
});
