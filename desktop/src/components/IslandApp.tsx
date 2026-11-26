import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { onHover, onIslandVisibility, onLaunchPop } from "../lib/ipc";
import { presentedSessions } from "../lib/sessions";
import { isActiveStatus, isWaitingStatus } from "../lib/status";
import { useIslandStore } from "../lib/store";
import type { SessionSnapshot } from "../lib/types";
import { IslandSurface } from "./IslandSurface";
import { LiveRegion } from "./LiveRegion";
import { ToastHost } from "./ToastHost";

function isActive(session: SessionSnapshot) {
  return isActiveStatus(session.status) || isWaitingStatus(session.status);
}

export function IslandApp() {
  const { t } = useTranslation();
  const ui = useIslandStore((state) => state.ui);
  const store = ui?.store;
  const codexSubsessionMode = ui?.settings.codex_subsession_mode;
  const { sessions } = useMemo(() => presentedSessions(store, codexSubsessionMode), [store, codexSubsessionMode]);
  const [prehover, setPrehover] = useState(false);
  const [idleHovered, setIdleHovered] = useState(false);
  const [launchRevision, setLaunchRevision] = useState<number | null>(null);
  const [visible, setVisible] = useState(true);
  const [documentVisible, setDocumentVisible] = useState(() => !document.hidden);

  useEffect(() => {
    let disposed = false;
    let off: Array<() => void> = [];
    void Promise.all([
      onHover((inside, idle) => {
        setPrehover(inside);
        setIdleHovered(idle);
      }),
      onLaunchPop(setLaunchRevision),
      onIslandVisibility(setVisible),
    ]).then((dispose) => {
      if (disposed) dispose.forEach((unsubscribe) => unsubscribe());
      else off = dispose;
    });
    return () => {
      disposed = true;
      off.forEach((unsubscribe) => unsubscribe());
    };
  }, []);

  useEffect(() => {
    const onVisibility = () => setDocumentVisible(!document.hidden);
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, []);

  if (!ui) {
    return (
      <div className="relative flex min-h-full items-start justify-center">
        <div className="island-shell px-4 py-3 text-[11px] text-muted">
          {t("common.loading")}
        </div>
        <ToastHost />
      </div>
    );
  }

  const { settings } = ui;
  const active = sessions.some(isActive);
  if (settings.hide_when_no_session && !ui.store.sessions.some(isActive)) return <LiveRegion />;

  const surface = ui.surface ?? { kind: settings.collapsed ? "collapsed" : "session_list" };
  const idle = sessions.length === 0;
  const mascotSpeed = settings.mascot_speed;
  const shellStyle = {
    "--content-font-size": `${settings.content_font_size}px`,
    "--ai-message-lines": settings.ai_message_lines > 0 ? String(settings.ai_message_lines) : "none",
  } as CSSProperties;

  return (
    <div
      className="relative min-h-full w-full"
      role="region"
      aria-label={t("a11y.islandLabel")}
      style={shellStyle}
    >
      <IslandSurface
        surface={surface}
        prehover={prehover}
        idle={idle}
        idleHovered={idleHovered}
        speed={settings.notch_animation_speed}
        pop={launchRevision === ui.surface_revision}
        widthScale={settings.collapsed_width_scale}
        notchHeight={settings.notch_height_mode === "custom" ? settings.custom_notch_height : 25}
        statusExtra={active ? 20 : 0}
        showsTool={settings.show_tool_status}
        animateMascots={mascotSpeed > 0 && visible && documentVisible && !ui.display_asleep &&
          !idle && (active || surface.kind !== "collapsed")}
      />
      <ToastHost />
      <LiveRegion />
    </div>
  );
}
