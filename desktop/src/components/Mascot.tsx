import { createContext, useContext, useLayoutEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { StatusKey } from "../lib/status";
import { useIslandStore } from "../lib/store";
import { compactMascotArt, mascotTime, type MascotPose } from "./compactMascotArt";
import { cursorPoses, geminiPoses } from "./compactCursorGeminiArt";
import { openCodePoses, qoderPoses } from "./providerMascotArt";

const MASCOT_ART = new Map<string, typeof compactMascotArt.claude>(Object.entries({
  ...compactMascotArt,
  cursor: cursorPoses,
  "cursor-cli": cursorPoses,
  gemini: geminiPoses,
  "google-antigravity": geminiPoses,
  opencode: openCodePoses,
  qoder: qoderPoses,
  "qoder-cli": qoderPoses,
  qodercn: qoderPoses,
  qoderwork: qoderPoses,
}));

export const MascotAnimationContext = createContext(true);

type Frame = (time: number) => void;

const activeFrames = new Set<Frame>();
const sleepingFrames = new Set<Frame>();
let frameTimer: number | undefined;
let lastSleepFrame = 0;

function subscribeMascotFrames(pose: MascotPose, draw: Frame) {
  const frames = pose === "idle" ? sleepingFrames : activeFrames;
  frames.add(draw);
  // Like Swift's FrameClock, all moving canvases share a 50 ms clock.
  // The 120 ms sleep cadence rounds to two ticks.
  frameTimer ??= window.setInterval(() => {
    const time = mascotTime(0.05, 1);
    activeFrames.forEach((frame) => frame(time));
    const sleepTime = mascotTime(0.1, 1);
    if (sleepTime !== lastSleepFrame) {
      lastSleepFrame = sleepTime;
      sleepingFrames.forEach((frame) => frame(sleepTime));
    }
  }, 50);
  return () => {
    frames.delete(draw);
    if (!activeFrames.size && !sleepingFrames.size) {
      window.clearInterval(frameTimer);
      frameTimer = undefined;
    }
  };
}

function MascotCanvas({ art, pose, animate, speed }: {
  art: typeof compactMascotArt.claude;
  pose: MascotPose;
  animate: boolean;
  speed: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || typeof CanvasRenderingContext2D === "undefined") return;
    const context = canvas.getContext("2d");
    if (!context) return;
    const interval = pose === "idle" ? 0.1 : 0.05;
    const moving = animate && speed > 0;
    let size = 0;
    let disposed = false;
    const draw = (time: number) => {
      if (disposed || !size) return;
      const scale = canvas.width / size;
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.clearRect(0, 0, size, size);
      context.save();
      art[pose](context, size, time * speed);
      context.restore();
    };
    const drawCurrent = () => draw(moving ? mascotTime(interval, 1) : 0);
    const resize = () => {
      size = canvas.clientWidth;
      const pixels = Math.max(1, Math.round(size * window.devicePixelRatio));
      if (canvas.width !== pixels || canvas.height !== pixels) {
        canvas.width = canvas.height = pixels;
      }
      drawCurrent();
    };
    resize();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(resize);
    observer?.observe(canvas);
    window.addEventListener("resize", resize);
    if (pose === "idle") void document.fonts?.load('900 6px "JetBrains Mono"').then(drawCurrent).catch(() => {});
    const unsubscribe = moving ? subscribeMascotFrames(pose, draw) : undefined;
    return () => {
      disposed = true;
      unsubscribe?.();
      window.removeEventListener("resize", resize);
      observer?.disconnect();
    };
  }, [art, animate, pose, speed]);

  return <canvas ref={canvasRef} className="mascot-canvas" data-pose={pose} aria-hidden="true" />;
}

export function Mascot({
  status,
  agent,
  speedPercent,
}: {
  status: StatusKey;
  agent?: string;
  speedPercent?: number;
}) {
  const { t } = useTranslation();
  const animate = useContext(MascotAnimationContext);
  const speed = useIslandStore((state) => (speedPercent ?? state.ui?.settings.mascot_speed ?? 100) / 100);
  const canvasArt = !agent || agent === "unknown" ? compactMascotArt.claude : MASCOT_ART.get(agent);
  const pose = status === "needs_you" ? "waiting"
    : status === "working" || status === "thinking" ? "working"
    : "idle";

  return (
    <span
      role="img"
      aria-label={t("a11y.mascotLabel")}
      className={`mascot mascot--${status}`}
      data-canvas={canvasArt ? "" : undefined}
    >
      {canvasArt ? (
        <MascotCanvas art={canvasArt} pose={pose} animate={animate} speed={speed} />
      ) : (
        <>
          <span className="mascot-sprite" />
          <span className="mascot-eyes" />
        </>
      )}
    </span>
  );
}
