import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { quitApp } from "../lib/ipc";
import { useIslandStore } from "../lib/store";
import { GearIcon, PowerIcon, SpeakerIcon, TrashIcon } from "./icons";
import { IconButton } from "./PixelButton";

const QUIT_CONFIRM_MS = 3000;

export function IslandHeaderControls({
  active = true,
  showIdleCount = false,
  idleRemoval,
}: {
  active?: boolean;
  showIdleCount?: boolean;
  idleRemoval?: { confirming: boolean; busy: boolean; toggle: () => void };
}) {
  const { t } = useTranslation();
  const soundEnabled = useIslandStore((state) => state.settingsSaveStatus === "saving"
    ? state.settingsDraft.sound_enabled ?? state.ui?.settings.sound_enabled
    : state.ui?.settings.sound_enabled);
  const openSettings = useIslandStore((state) => state.openSettings);
  const updateSettings = useIslandStore((state) => state.updateSettings);
  const [quitDeadline, setQuitDeadline] = useState<number | null>(null);
  const controlsRef = useRef<HTMLDivElement>(null);
  const previousPositions = useRef<Map<HTMLButtonElement, number> | null>(null);
  const quitArmed = active && quitDeadline !== null;
  const quitHint = t("header.quitConfirmHint");

  const setQuitConfirmation = useCallback((deadline: number | null) => {
    const controls = controlsRef.current;
    if (controls && !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) {
      const buttons = controls.querySelectorAll<HTMLButtonElement>(":scope > button:not([data-confirming])");
      previousPositions.current = new Map(Array.from(buttons, (button) => [
        button, button.getBoundingClientRect().left,
      ]));
    }
    setQuitDeadline(deadline);
  }, []);

  useLayoutEffect(() => {
    const before = previousPositions.current;
    previousPositions.current = null;
    if (!active || !before) return;
    for (const button of before.keys()) {
      button.getAnimations?.().find((animation) => animation.id === "quit-neighbor-slide")?.cancel();
    }
    const offsets = Array.from(before, ([button, left]) => [button, left - button.getBoundingClientRect().left] as const);
    for (const [button, offset] of offsets) {
      if (!offset || !button.animate) continue;
      const animation = button.animate(
        [{ translate: `${offset}px` }, { translate: "0px" }],
        { duration: 120, easing: "ease-out" },
      );
      animation.id = "quit-neighbor-slide";
    }
  }, [active, quitArmed]);

  useEffect(() => {
    if (quitDeadline === null) return;
    if (!active) {
      setQuitDeadline(null);
      return;
    }
    const timeout = window.setTimeout(
      () => setQuitConfirmation(null),
      Math.max(0, quitDeadline - performance.now()),
    );
    return () => window.clearTimeout(timeout);
  }, [active, quitDeadline, setQuitConfirmation]);

  const pressQuit = () => {
    if (!active) return;
    const now = performance.now();
    // The webview can defer timers in the background. Check the deadline
    // again on the confirming press, as the macOS button does.
    if (quitDeadline !== null && now <= quitDeadline) {
      setQuitConfirmation(null);
      void quitApp();
    } else {
      setQuitConfirmation(now + QUIT_CONFIRM_MS);
    }
  };

  const controls = (
    <div ref={controlsRef} className="island-header-controls">
      {idleRemoval && (
        <IconButton
          label={t("idleRemoval.label")}
          title={t("idleRemoval.help")}
          aria-expanded={idleRemoval.confirming}
          data-tone={idleRemoval.confirming ? "remove-idle" : undefined}
          disabled={idleRemoval.busy}
          onClick={idleRemoval.toggle}
        >
          <TrashIcon />
        </IconButton>
      )}
      <IconButton
        label={soundEnabled ? t("header.soundOff") : t("header.soundOn")}
        onClick={() => {
          if (soundEnabled !== undefined) {
            void updateSettings({ sound_enabled: !soundEnabled });
          }
        }}
      >
        <SpeakerIcon muted={!soundEnabled} />
      </IconButton>
      <IconButton label={t("header.settings")} onClick={() => void openSettings()}>
        <GearIcon />
      </IconButton>
      <IconButton
        label={quitArmed ? quitHint : t("header.quit")}
        data-tone="danger"
        data-confirming={quitArmed}
        onClick={pressQuit}
        onMouseLeave={() => quitArmed && setQuitConfirmation(null)}
        onBlur={() => quitArmed && setQuitConfirmation(null)}
      >
        {quitArmed ? t("header.quitConfirm") : <PowerIcon />}
      </IconButton>
      <span className="sr-only" role="status">
        {quitArmed ? quitHint : ""}
      </span>
    </div>
  );
  return showIdleCount ? (
    <div className="island-idle-controls">
      {!quitArmed && <span className="island-compact-count">0</span>}
      {controls}
    </div>
  ) : controls;
}
