import { useEffect, useRef, useState, type AnimationEvent, type KeyboardEvent, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { focusSessionTerminal, setIslandSurface } from "../lib/ipc";
import { playCue } from "../lib/sound";
import { useIslandStore } from "../lib/store";

export function useSessionJump(
  sessionId: string,
  origin: "session_list" | "completion_card",
  dragging = false,
) {
  const { t } = useTranslation();
  const [jumpFailed, setJumpFailed] = useState(false);
  const jumpAttempt = useRef(0);
  useEffect(() => () => { jumpAttempt.current += 1; }, [sessionId, origin]);

  const jumpToSession = async () => {
    const attempt = ++jumpAttempt.current;
    setJumpFailed(false);
    const before = useIslandStore.getState().ui;
    const focused = await focusSessionTerminal(sessionId).catch(() => false);
    if (attempt !== jumpAttempt.current || !before?.settings.auto_collapse_after_session_jump) return;

    const current = useIslandStore.getState().ui;
    if (!current || current.surface_revision !== before.surface_revision) return;
    const surface = current.surface ?? { kind: current.settings.collapsed ? "collapsed" : "session_list" };
    if (surface.kind !== origin || (surface.kind === "completion_card" && surface.session_id !== sessionId)) return;

    if (focused) {
      await setIslandSurface({ kind: "collapsed" });
    } else {
      playCue("error", current.settings.sound_volume, current.settings.custom_sound_paths["8bit_error"]);
      setJumpFailed(true);
    }
  };

  const handleClick = (event: MouseEvent<HTMLElement>) => {
    if (dragging || event.defaultPrevented || !(event.target instanceof Element) ||
        event.target.closest("button, a, input, select, textarea, summary, [role='button'], [role='link'], [contenteditable='true']")) return;
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed && event.currentTarget.contains(selection.anchorNode)) return;
    void jumpToSession();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget || event.repeat || dragging ||
        (event.key !== "Enter" && event.key !== " ")) return;
    event.preventDefault();
    void jumpToSession();
  };

  const handleAnimationEnd = (event: AnimationEvent<HTMLElement>) => {
    if (event.target === event.currentTarget) setJumpFailed(false);
  };

  if (sessionId.startsWith("ssh:")) return {};
  return {
    "data-can-jump": true,
    "data-jump-failed": jumpFailed || undefined,
    tabIndex: 0,
    "aria-description": t("approval.jumpHint"),
    "aria-keyshortcuts": "Enter Space",
    onClick: handleClick,
    onKeyDown: handleKeyDown,
    onAnimationEnd: jumpFailed ? handleAnimationEnd : undefined,
  };
}
