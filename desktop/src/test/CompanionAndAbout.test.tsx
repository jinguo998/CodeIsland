import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "../lib/defaults";
import * as ipc from "../lib/ipc";
import { clampSettings } from "../lib/settings";
import { MaxVisibleSessionsField } from "../settings/MaxVisibleSessionsField";
import { AboutTab } from "../settings/tabs/AboutTab";
import { BuddyTab } from "../settings/tabs/BuddyTab";

afterEach(() => vi.restoreAllMocks());

describe("iPhone Buddy settings", () => {
  it("applies enable, sync interval and restart and reports subscribed devices", async () => {
    const user = userEvent.setup();
    const changes = vi.fn();
    const status = vi.spyOn(ipc, "appleCompanionStatus").mockResolvedValue({
      powered_on: true, advertising: true, subscribed_clients: 2, error: null,
    });
    const restart = vi.spyOn(ipc, "appleCompanionRestart").mockResolvedValue();
    const { rerender } = render(<BuddyTab settings={DEFAULT_SETTINGS} onChange={changes} />);
    const enabled = screen.getByRole("checkbox", { name: "Allow iPhone Buddy to discover this PC" });
    const interval = screen.getByRole("slider", { name: "Sync interval" });
    const restartButton = screen.getByRole("button", { name: "Restart iPhone Buddy" });
    expect(interval).toBeDisabled();
    expect(restartButton).toBeDisabled();
    expect(status).not.toHaveBeenCalled();

    await user.click(enabled);
    expect(changes).toHaveBeenCalledWith({ apple_companion_enabled: true });
    rerender(<BuddyTab settings={{ ...DEFAULT_SETTINGS, apple_companion_enabled: true }} onChange={changes} />);
    expect(await screen.findByText("(2)")).toBeVisible();
    expect(interval).toHaveAttribute("aria-valuetext", "5 s");
    fireEvent.change(interval, { target: { value: "30" } });
    expect(changes).toHaveBeenCalledWith({ apple_companion_heartbeat_seconds: 30 });

    await user.click(restartButton);
    expect(restart).toHaveBeenCalledOnce();
    await waitFor(() => expect(restartButton).not.toBeDisabled());
  });
});

describe("About actions", () => {
  it("links the current version to its release notes and falls back for an invalid tag", () => {
    const props = { settings: DEFAULT_SETTINGS, onChange: vi.fn() };
    const { rerender } = render(<AboutTab {...props} version="v1.0.41" />);
    expect(screen.getByRole("link", { name: "What's New in v1.0.41" })).toHaveAttribute(
      "href", "https://github.com/wxtsky/CodeIsland/releases/tag/v1.0.41",
    );
    rerender(<AboutTab {...props} version="https://invalid.example/tag" />);
    expect(screen.getByRole("link", { name: /What's New in/ })).toHaveAttribute(
      "href", "https://github.com/wxtsky/CodeIsland/releases",
    );
  });

  it("reports the saved diagnostics path and treats a canceled picker as no action", async () => {
    const user = userEvent.setup();
    const destination = "C:\\reports\\diagnostics.json";
    const exportReport = vi.spyOn(ipc, "exportDiagnosticsAs")
      .mockResolvedValueOnce(null).mockResolvedValueOnce(destination);
    render(<AboutTab version="1.0.41" settings={DEFAULT_SETTINGS} onChange={vi.fn()} />);
    const button = screen.getByRole("button", { name: "Export diagnostics" });

    await user.click(button);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    await user.click(button);
    expect(await screen.findByRole("status")).toHaveTextContent(destination);
    expect(exportReport).toHaveBeenCalledTimes(2);
  });
});

describe("visible session limits", () => {
  it("keeps a legacy choice and saves the original Unlimited sentinel", () => {
    const changes = vi.fn();
    render(<MaxVisibleSessionsField settings={{ ...DEFAULT_SETTINGS, max_visible_sessions: 7 }} onChange={changes} />);
    const limit = screen.getByRole("combobox", { name: "Max visible sessions" });
    expect(limit).toHaveValue("7");
    expect(within(limit).getAllByRole("option").map((item) => item.textContent))
      .toEqual(["3", "5", "7", "8", "10", "Unlimited"]);
    fireEvent.change(limit, { target: { value: "99" } });
    expect(changes).toHaveBeenCalledWith({ max_visible_sessions: 99 });
    expect(clampSettings({ ...DEFAULT_SETTINGS, max_visible_sessions: 99 }).max_visible_sessions).toBe(99);
    expect(clampSettings({ ...DEFAULT_SETTINGS, max_visible_sessions: 0 }).max_visible_sessions).toBe(1);
  });
});
