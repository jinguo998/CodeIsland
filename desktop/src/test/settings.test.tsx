import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS, withSettingsDefaults } from "../lib/defaults";
import { clampSettings } from "../lib/settings";
import { AppearanceTab } from "../settings/tabs/AppearanceTab";
import { BehaviorTab } from "../settings/tabs/BehaviorTab";

describe("settings form", () => {
  it("exposes the original notch animation speed range in Appearance", () => {
    const onChange = vi.fn();
    render(<AppearanceTab settings={DEFAULT_SETTINGS} onChange={onChange} />);

    const slider = screen.getByRole("slider", { name: "Open / Close Speed" });
    expect(slider).toHaveAttribute("min", "0.5");
    expect(slider).toHaveAttribute("max", "2");
    expect(slider).toHaveAttribute("step", "0.1");
    expect(slider).toHaveAttribute("aria-valuetext", "1.0×");

    fireEvent.change(slider, { target: { value: "1.4" } });
    expect(onChange).toHaveBeenCalledWith({ notch_animation_speed: 1.4 });
  });

  it("defaults missing speeds, clamps outliers and preserves fractional values", () => {
    expect(withSettingsDefaults({}).notch_animation_speed).toBe(1);
    const speed = (notch_animation_speed: number) =>
      clampSettings({ ...DEFAULT_SETTINGS, notch_animation_speed }).notch_animation_speed;
    expect(speed(0)).toBe(0.5);
    expect(speed(3)).toBe(2);
    expect(speed(Infinity)).toBe(1);
    expect(speed(NaN)).toBe(1);
    expect(speed(1.37)).toBe(1.37);
  });

  it("exposes collapsed width and custom notch height with the original limits", () => {
    const onChange = vi.fn();
    render(<AppearanceTab settings={{ ...DEFAULT_SETTINGS, notch_height_mode: "custom" }} onChange={onChange} />);

    const width = screen.getByRole("slider", { name: "Island Width" });
    expect(width).toHaveAttribute("min", "50");
    expect(width).toHaveAttribute("max", "150");
    expect(width).toHaveAttribute("aria-valuetext", "100%");
    fireEvent.change(width, { target: { value: "80" } });
    expect(onChange).toHaveBeenCalledWith({ collapsed_width_scale: 80 });

    const height = screen.getByRole("slider", { name: "Custom Height" });
    expect(height).toHaveAttribute("min", "15");
    expect(height).toHaveAttribute("max", "60");
    expect(height).toHaveAttribute("aria-valuetext", "37 pt");
    fireEvent.change(height, { target: { value: "28" } });
    expect(onChange).toHaveBeenCalledWith({ custom_notch_height: 28 });

    const invalid = clampSettings({
      ...DEFAULT_SETTINGS,
      collapsed_width_scale: 200,
      notch_height_mode: "invalid",
      custom_notch_height: 0,
    });
    expect(invalid.collapsed_width_scale).toBe(150);
    expect(invalid.notch_height_mode).toBe("matchNotch");
    expect(invalid.custom_notch_height).toBe(37);
  });

  it("offers Unlimited from the Behavior session settings", () => {
    const onChange = vi.fn();
    render(<BehaviorTab settings={{ ...DEFAULT_SETTINGS }} onChange={onChange} />);

    fireEvent.change(screen.getByLabelText("Max visible sessions"), {
      target: { value: "99" },
    });

    expect(onChange).toHaveBeenCalledWith({ max_visible_sessions: 99 });
  });

  it("allows cleanup minutes to be 0 (never)", () => {
    const onChange = vi.fn();
    render(<BehaviorTab settings={{ ...DEFAULT_SETTINGS }} onChange={onChange} />);

    fireEvent.change(screen.getByLabelText("Session cleanup (minutes)"), {
      target: { value: "0" },
    });

    expect(onChange).toHaveBeenCalledWith({ session_cleanup_minutes: 0 });
  });

  it("clampSettings normalizes out-of-range and invalid values", () => {
    const next = clampSettings({
      ...DEFAULT_SETTINGS,
      max_visible_sessions: 999,
      session_cleanup_minutes: 0,
      max_history: 1,
      language: "fr",
      density: "huge",
      display_mode: "wall",
      toggle_shortcut: "Ctrl+Shift+A+B",
    });

    expect(next.max_visible_sessions).toBe(20);
    expect(next.session_cleanup_minutes).toBe(0);
    expect(next.max_history).toBe(10);
    expect(next.language).toBe("system");
    expect(next.density).toBe("comfortable");
    expect(next.display_mode).toBe("primary");
    expect(next.toggle_shortcut).toBe("Ctrl+Alt+I");
  });

  it("clamps the newer content, sound and hover settings", () => {
    const next = clampSettings({
      ...DEFAULT_SETTINGS,
      hover_expand_delay_ms: 5000,
      sound_volume: -10,
      mascot_speed: 900,
      content_font_size: 3,
      ai_message_lines: 42,
      group_by: "mood",
      display_choice: "window-3",
    });

    expect(next.hover_expand_delay_ms).toBe(1000);
    expect(next.sound_volume).toBe(0);
    expect(next.mascot_speed).toBe(300);
    expect(next.content_font_size).toBe(11);
    expect(next.ai_message_lines).toBe(10);
    expect(next.group_by).toBe("all");
    expect(next.display_choice).toBe("auto");
  });
});
