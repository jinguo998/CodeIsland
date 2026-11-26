// State scenes ported from OpenCodeView.swift and QoderView.swift.
import {
  MASCOT_ALERT_JUMP,
  alertGlow,
  blink,
  exclamation,
  interpolate,
  quirk,
  rect,
  viewport,
  type MascotPose,
} from "./compactMascotArt";

type Canvas = CanvasRenderingContext2D;
type Art = Record<MascotPose, (context: Canvas, size: number, time: number) => void>;
type PoseStyle = {
  body: (context: Canvas, lift: number, scaleX: number, scaleY: number) => void;
  face: (context: Canvas, lift: number, eyeScale: number, sleeping: boolean) => void;
  legs: (context: Canvas) => void;
  idlePeriods: readonly [number, number];
  idleShadowY: number;
  workPauseCycle: number;
  workSeed: number;
  blinkSeed: number;
  keyboard: readonly [base: string, key: string, flash: string];
  alert: string;
};

const TAU = 2 * Math.PI;
const OPENCODE_BODY = "rgb(22% 22% 24%)";
const OPENCODE_FRAME_HORIZONTAL = "rgb(55% 55% 57% / 0.6)";
const OPENCODE_FRAME_VERTICAL = "rgb(55% 55% 57% / 0.4)";
const OPENCODE_LEG = "rgb(35% 35% 37%)";
const QODER_BODY = "rgb(16.5% 85.9% 36.1%)";
const QODER_LEG = "rgb(12% 65% 28%)";

function bodyTransform(context: Canvas, lift: number, scaleX: number, scaleY: number) {
  context.save();
  context.translate(7.5, 10 + lift);
  context.scale(scaleX, scaleY);
  context.translate(-7.5, -10);
}

function openCodeBody(context: Canvas, lift: number, scaleX: number, scaleY: number) {
  bodyTransform(context, lift, scaleX, scaleY);
  rect(context, OPENCODE_BODY, 3, 5, 9, 1);
  for (let row = 6; row < 13; row++) rect(context, OPENCODE_BODY, 2, row, 11, 1);
  rect(context, OPENCODE_BODY, 3, 13, 9, 1);
  rect(context, OPENCODE_FRAME_HORIZONTAL, 3, 6, 9, 0.7);
  rect(context, OPENCODE_FRAME_HORIZONTAL, 3, 12, 9, 0.7);
  for (let row = 7; row < 12; row++) {
    rect(context, OPENCODE_FRAME_VERTICAL, 3, row, 0.7, 1);
    rect(context, OPENCODE_FRAME_VERTICAL, 11.3, row, 0.7, 1);
  }
  context.restore();
}

function openCodeFace(context: Canvas, lift: number, eyeScale: number, sleeping: boolean) {
  const eyeHeight = 2 * eyeScale;
  const y = 8.5 + (2 - eyeHeight) / 2 + lift;
  const color = sleeping ? "rgb(85% 85% 87% / 0.4)" : "rgb(85% 85% 87%)";
  rect(context, color, 4.5, y, 0.8, Math.max(0.3, eyeHeight));
  rect(context, color, 4, y + eyeHeight * 0.3, 0.7, Math.max(0.3, eyeHeight * 0.4));
  rect(context, color, 9.7, y, 0.8, Math.max(0.3, eyeHeight));
  rect(context, color, 10.2, y + eyeHeight * 0.3, 0.7, Math.max(0.3, eyeHeight * 0.4));
  if (eyeScale > 0.5) rect(context, "rgb(85% 85% 87% / 0.8)", 7.1, 9.2 + lift, 0.8, 0.8);
}

const QODER_ROWS = [
  [14, 4, 7], [13, 2, 11], [12, 1, 13], [11, 1, 13], [10, 1, 13],
  [9, 1, 13], [8, 1, 13], [7, 2, 11], [6, 3, 9], [5, 4, 7],
] as const;

function qoderBody(context: Canvas, lift: number, scaleX: number, scaleY: number) {
  bodyTransform(context, lift, scaleX, scaleY);
  for (const [y, x, width] of QODER_ROWS) rect(context, QODER_BODY, x, y, width, 1);
  context.restore();
}

function qoderFace(context: Canvas, lift: number, eyeScale: number, sleeping: boolean) {
  const eyeHeight = 1.5 * eyeScale;
  const y = 9 + (1.5 - eyeHeight) / 2 + lift;
  const color = sleeping ? "rgb(0 0 0 / 0.5)" : "black";
  rect(context, color, 4, y, 1.2, Math.max(0.3, eyeHeight));
  rect(context, color, 9.8, y, 1.2, Math.max(0.3, eyeHeight));
  if (!sleeping) {
    rect(context, color, 5, 11.5 + lift, 1, 0.8);
    rect(context, color, 6, 12 + lift, 3, 0.8);
    rect(context, color, 9, 11.5 + lift, 1, 0.8);
  }
}

function sleepLetters(context: Canvas, size: number, time: number) {
  context.textAlign = "center";
  context.textBaseline = "middle";
  for (let index = 0; index < 3; index++) {
    const cycle = 2.8 + index * 0.3;
    const phase = Math.max(0, ((time - index * 0.9) % cycle) / cycle);
    const fontSize = Math.max(6, size * (0.18 + phase * 0.1));
    const opacity = (0.7 - index * 0.1) * (phase < 0.8 ? 1 : (1 - phase) * 3.5);
    context.font = `900 ${fontSize}px "JetBrains Mono", monospace`;
    context.fillStyle = `rgb(255 255 255 / ${opacity})`;
    context.fillText("z", size * (0.65 + index * 0.08), size * (0.35 - phase * 0.38));
  }
}

function drawKeyboard(context: Canvas, time: number, [base, key, flash]: PoseStyle["keyboard"]) {
  rect(context, base, 0, 13, 15, 3);
  for (let row = 0; row < 2; row++) {
    for (let column = 0; column < 6; column++) {
      rect(context, key, 0.5 + column * 2.4, 13.5 + row * 1.2, 1.8, 0.7);
    }
  }
  const phase = Math.trunc(time / 0.1) % 6;
  rect(context, flash, 0.5 + phase * 2.4, 13.5 + Math.trunc(phase / 3) * 1.2, 1.8, 0.7);
}

function poses(style: PoseStyle): Art {
  return {
    idle(context, size, time) {
      const [first, second] = style.idlePeriods;
      const lift = Math.sin(time * TAU / first) * 0.68 + Math.sin(time * TAU / second) * 0.36;
      const shadow = 7 + Math.abs(lift) * 0.3;
      context.save();
      viewport(context, size, 15, 12, 4);
      rect(context, "rgb(0 0 0 / 0.2)", 7.5 - shadow / 2, style.idleShadowY, shadow, 1);
      style.legs(context);
      style.body(context, lift, 1, 1);
      style.face(context, lift, 0.3, true);
      context.restore();
      sleepLetters(context, size, time);
    },
    working(context, size, time) {
      const pause = quirk(time, style.workPauseCycle, 1.2, style.workSeed);
      const lift = Math.sin(time * TAU / 0.4) * (1 - pause) + Math.sin(time * TAU / 2.9) * 0.3 * pause;
      const shadow = 8 - Math.abs(lift) * 0.3;
      viewport(context, size, 16, 14, 3);
      rect(context, `rgb(0 0 0 / ${Math.max(0.1, 0.35 - Math.abs(lift) * 0.03)})`, 4 + (8 - shadow) / 2, 16, shadow, 1);
      style.legs(context);
      drawKeyboard(context, time, style.keyboard);
      style.body(context, lift, 1, 1);
      style.face(context, lift, Math.max(0.1, blink(time, style.blinkSeed)), false);
    },
    waiting(context, size, time) {
      alertGlow(context, style.alert, size, time);
      const phase = time % 3.5 / 3.5;
      const jump = interpolate(MASCOT_ALERT_JUMP, phase);
      const squash = jump > 0.5 ? jump : 0;
      const height = Math.abs(Math.min(0, jump));
      const shadow = 8 * (1 - height * 0.04);
      const shake = phase > 0.15 && phase < 0.55 ? Math.sin(phase * 80) * 0.6 : 0;
      const eyeScale = phase > 0.03 && phase < 0.15 ? 1.3 : 1;
      viewport(context, size, 16, 14, 3);
      rect(context, `rgb(0 0 0 / ${Math.max(0.08, 0.4 - height * 0.04)})`, 4 + (8 - shadow) / 2, 16, shadow, 1);
      style.legs(context);
      context.save();
      context.translate(shake, 0);
      style.body(context, jump, 1 + squash * 0.03, 1 - squash * 0.02);
      style.face(context, jump, eyeScale, false);
      context.restore();
      exclamation(context, style.alert, phase, 4, jump);
    },
  };
}

export const openCodePoses = poses({
  body: openCodeBody,
  face: openCodeFace,
  legs(context) {
    rect(context, OPENCODE_LEG, 4, 13.5, 1, 1.5);
    rect(context, OPENCODE_LEG, 10, 13.5, 1, 1.5);
  },
  idlePeriods: [3.85, 7.24],
  idleShadowY: 14.5,
  workPauseCycle: 11.1,
  workSeed: 0xc88,
  blinkSeed: 0xc89,
  keyboard: ["rgb(12% 12% 14%)", "rgb(30% 30% 32%)", "rgb(255 255 255 / 0.9)"],
  alert: "rgb(100% 55% 0%)",
});

export const qoderPoses = poses({
  body: qoderBody,
  face: qoderFace,
  legs(context) {
    rect(context, QODER_LEG, 5, 14.5, 1, 1.5);
    rect(context, QODER_LEG, 9, 14.5, 1, 1.5);
  },
  idlePeriods: [4.42, 6.96],
  idleShadowY: 15.5,
  workPauseCycle: 11.4,
  workSeed: 0x1bc,
  blinkSeed: 0x1bd,
  keyboard: ["rgb(10% 18% 12%)", "rgb(20% 38% 24%)", "rgb(16.5% 85.9% 36.1% / 0.9)"],
  alert: "rgb(100% 24% 0%)",
});
