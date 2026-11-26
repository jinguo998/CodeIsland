import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "../lib/defaults";
import * as ipc from "../lib/ipc";
import * as sound from "../lib/sound";
import { SoundTab } from "../settings/tabs/SoundTab";

afterEach(() => vi.restoreAllMocks());

describe("sound settings", () => {
  it("shows only the master switch while sound effects are disabled", () => {
    render(<SoundTab settings={DEFAULT_SETTINGS} onChange={vi.fn()} />);
    expect(screen.getByLabelText("Enable Sound Effects")).not.toBeChecked();
    expect(screen.queryByLabelText("Volume")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Preview —/ })).not.toBeInTheDocument();
  });

  it("selects, previews and resets one custom cue with quiet hours enabled", async () => {
    const user = userEvent.setup();
    const path = "C:\\sounds\\calm bell.wav";
    const choose = vi.spyOn(ipc, "pickCustomSound").mockResolvedValue(path);
    const reset = vi.spyOn(ipc, "resetCustomSound").mockResolvedValue();
    const preview = vi.spyOn(sound, "previewSound").mockResolvedValue(true);
    render(<SoundTab settings={{
      ...DEFAULT_SETTINGS,
      sound_enabled: true,
      quiet_hours_enabled: true,
      audible_cues: [],
    }} onChange={vi.fn()} />);

    const trigger = screen.getByRole("button", { name: "Choose Sound File — Session Start" });
    const menu = document.getElementById(trigger.getAttribute("popovertarget")!)!;
    await user.click(trigger);
    // jsdom leaves native popovers closed, so dispatch the menu action directly.
    fireEvent.click(within(menu).getByText("Choose Sound File"));
    expect(choose).toHaveBeenCalledExactlyOnceWith("8bit_start", "Choose Sound File");
    expect(await screen.findByText("Custom: calm bell.wav")).toBeVisible();
    expect(screen.getByLabelText("From")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Session Start" })).not.toBeChecked();

    await user.click(screen.getByRole("button", { name: "Preview — Session Start" }));
    expect(preview).toHaveBeenCalledExactlyOnceWith("SessionStart", 50, path);

    await user.click(trigger);
    fireEvent.click(within(menu).getByText("Reset to Default"));
    expect(reset).toHaveBeenCalledExactlyOnceWith("8bit_start");
    await waitFor(() => expect(screen.queryByText("Custom: calm bell.wav")).not.toBeInTheDocument());
    expect(screen.getByText("New Claude Code session")).toBeVisible();
  });

  it("shows the six grouped events and only shows time fields when quiet hours are enabled", () => {
    render(<SoundTab settings={{ ...DEFAULT_SETTINGS, sound_enabled: true }} onChange={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "Sessions" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "Interaction" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "System" })).toBeVisible();
    expect(screen.getAllByRole("button", { name: /^Preview —/ })).toHaveLength(6);
    expect(screen.queryByLabelText("From")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Until")).not.toBeInTheDocument();
  });
});
