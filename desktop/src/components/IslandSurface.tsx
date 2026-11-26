import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { onIslandDisplayWidth, setHoverPanelSize } from "../lib/ipc";
import type { ExpandedIslandSurface, IslandSurfaceState } from "../lib/types";
import { CollapsedStrip } from "./CollapsedStrip";
import { ExpandedPanel } from "./ExpandedPanel";
import { MascotAnimationContext } from "./Mascot";

const OPEN_RESPONSE_MS = 420;
const CLOSE_RESPONSE_MS = 380;

type SpringState = { position: number; velocity: number };

function sampleSpring(
  start: SpringState,
  target: number,
  seconds: number,
  response: number,
  damping: number,
): SpringState {
  const omega = 2 * Math.PI / response;
  const decay = damping * omega;
  const initial = start.position - target;
  const envelope = Math.exp(-decay * seconds);

  if (damping === 1) {
    const slope = start.velocity + decay * initial;
    const displacement = initial + slope * seconds;
    return {
      position: target + envelope * displacement,
      velocity: envelope * (slope - decay * displacement),
    };
  }

  const frequency = omega * Math.sqrt(1 - damping * damping);
  const sine = Math.sin(frequency * seconds);
  const cosine = Math.cos(frequency * seconds);
  const slope = (start.velocity + decay * initial) / frequency;
  const displacement = initial * cosine + slope * sine;
  return {
    position: target + envelope * displacement,
    velocity: envelope * (
      frequency * (slope * cosine - initial * sine) - decay * displacement
    ),
  };
}

export function IslandSurface({
  surface,
  prehover,
  idle,
  idleHovered,
  speed,
  pop = false,
  widthScale = 100,
  notchHeight = 25,
  statusExtra,
  showsTool,
  animateMascots = true,
}: {
  surface: IslandSurfaceState;
  prehover: boolean;
  idle: boolean;
  idleHovered: boolean;
  speed: number;
  pop?: boolean;
  widthScale?: number;
  notchHeight?: number;
  statusExtra: number;
  showsTool: boolean;
  animateMascots?: boolean;
}) {
  const collapsed = surface.kind === "collapsed" || idle;
  const panel = surface.kind !== "collapsed" && !idle ? surface : null;
  const surfaceRef = useRef<HTMLDivElement>(null);
  const stripRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [retainedPanel, setRetainedPanel] = useState<ExpandedIslandSurface | null>(panel);
  const spring = useRef<SpringState>({ position: collapsed ? 0 : 1, velocity: 0 });
  const [reducedMotion, setReducedMotion] = useState(
    () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false,
  );
  const [screenWidth, setScreenWidth] = useState(() => window.screen.width || window.innerWidth);
  const [quotaWidth, setQuotaWidth] = useState(0);
  const [toolStatusForWidth, setToolStatusForWidth] = useState(showsTool);
  const requestedToolStatus = useRef(showsTool);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let nativeWidth: number | undefined;
    const update = () => setScreenWidth(nativeWidth ?? (window.screen.width || window.innerWidth));
    void onIslandDisplayWidth((width) => {
      if (!disposed && Number.isFinite(width) && width > 0) {
        nativeWidth = width;
        update();
      }
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(console.warn);
    window.addEventListener("resize", update);
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("resize", update);
    };
  }, []);

  useEffect(() => {
    const query = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    if (!query) return;
    const update = () => setReducedMotion(query.matches);
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  useLayoutEffect(() => {
    const host = surfaceRef.current;
    const changed = showsTool !== requestedToolStatus.current;
    requestedToolStatus.current = showsTool;
    if (!changed || !collapsed || reducedMotion || !host?.animate) {
      setToolStatusForWidth(showsTool);
      return;
    }
    // NotchPanelView's curtain changes the width while the strip is hidden.
    const raised = `translateY(-${notchHeight}px)`;
    const curtain = host.animate([
      { offset: 0, transform: "translateY(0)", opacity: 1, easing: "ease-in" },
      { offset: 200 / 600, transform: raised, opacity: 0 },
      { offset: 350 / 600, transform: raised, opacity: 0, easing: "ease-out" },
      { offset: 1, transform: "translateY(0)", opacity: 1 },
    ], 600);
    const swap = window.setTimeout(() => setToolStatusForWidth(showsTool), 250);
    return () => {
      window.clearTimeout(swap);
      curtain.cancel();
    };
  }, [collapsed, notchHeight, reducedMotion, showsTool]);

  // The outgoing panel remains inert while the contour finishes closing.
  useLayoutEffect(() => {
    if (panel) setRetainedPanel(panel);
  }, [panel]);

  useLayoutEffect(() => {
    const host = surfaceRef.current;
    if (!host) return;
    const target = collapsed ? 0 : 1;
    const openingPop = !collapsed && pop;
    const response = openingPop
      ? 0.3
      : (collapsed ? CLOSE_RESPONSE_MS : OPEN_RESPONSE_MS) / speed / 1000;
    const damping = collapsed ? 1 : openingPop ? 0.65 : 0.82;
    const publish = () => {
      const position = Math.min(1.025, Math.max(0, spring.current.position));
      host.style.setProperty("--island-progress", String(position));
    };
    const finish = () => {
      spring.current = { position: target, velocity: 0 };
      publish();
      if (collapsed) setRetainedPanel(null);
    };

    const start = spring.current;
    if (
      reducedMotion ||
      !window.requestAnimationFrame ||
      (start.position === target && start.velocity === 0)
    ) {
      finish();
      return;
    }

    // A reversal starts at the current position and velocity, just like the
    // interruptible SwiftUI spring. Every visual layer reads this progress.
    const startedAt = performance.now();
    let frame = 0;
    const animate = (now: number) => {
      const elapsed = Math.max(0, (now - startedAt) / 1000);
      spring.current = sampleSpring(start, target, elapsed, response, damping);
      const { position, velocity } = spring.current;
      if (Math.abs(position - target) < 0.001 && Math.abs(velocity) < 0.025) {
        finish();
      } else {
        publish();
        frame = window.requestAnimationFrame(animate);
      }
    };
    frame = window.requestAnimationFrame(animate);
    return () => window.cancelAnimationFrame(frame);
  }, [collapsed, pop, speed, reducedMotion]);

  useLayoutEffect(() => {
    const host = surfaceRef.current;
    const strip = stripRef.current;
    const expanded = collapsed ? null : panelRef.current;
    if (!host || !strip) return;

    const measure = (layer: HTMLDivElement, compact: boolean) => {
      const { offsetWidth: width, offsetHeight: height } = layer;
      if (!width || !height) return;
      const prefix = compact ? "--island-strip" : "--island-panel";
      const widthPx = `${width}px`;
      const heightPx = `${height}px`;
      if (
        host.style.getPropertyValue(`${prefix}-width`) === widthPx &&
        host.style.getPropertyValue(`${prefix}-height`) === heightPx
      ) return;
      host.style.setProperty(`${prefix}-width`, widthPx);
      host.style.setProperty(`${prefix}-height`, heightPx);
      void setHoverPanelSize(width, height, compact).catch(console.warn);
    };

    const measureLayers = () => {
      measure(strip, true);
      if (expanded) measure(expanded, false);
    };
    measureLayers();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measureLayers);
    observer.observe(strip);
    if (expanded) observer.observe(expanded);
    return () => observer.disconnect();
  }, [collapsed]);

  const notchWidth = Math.min(Math.max(screenWidth * 0.14, 160), 240) * widthScale / 100;
  const mascotSize = Math.min(27, notchHeight - 6);
  const idleBaseWidth = notchWidth + 2 * (mascotSize + 14);
  const baseCompactWidth = idleBaseWidth + statusExtra + (toolStatusForWidth ? screenWidth * 0.04 : 0);
  const maxPanelWidth = Math.min(620, screenWidth - 40);
  const quotaReserve = !idle && quotaWidth > 0
    ? Math.min(quotaWidth + 6, Math.max(0, maxPanelWidth - baseCompactWidth - (prehover ? 7 : 0)))
    : 0;
  const compactWidth = baseCompactWidth + quotaReserve;
  const expandedWidth = Math.min(Math.max(notchWidth + 200, 580), maxPanelWidth);
  const style = {
    "--island-compact-width": idle ? undefined : `${compactWidth}px`,
    "--island-idle-base-width": `${idleBaseWidth}px`,
    "--island-panel-target-width": `${expandedWidth}px`,
    "--island-notch-height": `${notchHeight}px`,
  } as CSSProperties;

  const shownPanel = panel ?? retainedPanel;
  const mascotMotion = animateMascots && !reducedMotion;
  return (
    <div
      ref={surfaceRef}
      className="island-surface"
      data-collapsed={collapsed}
      data-idle={idle}
      data-idle-hovered={idleHovered}
      data-prehover={collapsed && !idle && prehover}
      data-mascot-paused={!mascotMotion || undefined}
      style={style}
    >
      <MascotAnimationContext.Provider value={mascotMotion}>
        <div ref={stripRef} className="island-strip-layer" inert={!collapsed}>
          <MascotAnimationContext.Provider value={mascotMotion && collapsed}>
            <CollapsedStrip idleHovered={idleHovered} active={collapsed} onQuotaWidth={setQuotaWidth} />
          </MascotAnimationContext.Provider>
        </div>
        {shownPanel && (
          <div ref={panelRef} className="island-panel-layer" inert={collapsed}>
            <ExpandedPanel surface={shownPanel} active={!collapsed} />
          </div>
        )}
      </MascotAnimationContext.Provider>
    </div>
  );
}
