// Compact Canvas scenes ported from CursorView.swift and GeminiView.swift.
import {
  alertGlow,
  blink,
  exclamation,
  interpolate,
  quirk,
  rect,
  viewport,
} from "./compactMascotArt";

type Canvas = CanvasRenderingContext2D;
type Point = readonly [x: number, y: number];

const TAU = 2 * Math.PI;
const ALERT = "rgb(100% 24% 0%)";
const CURSOR_DARK = "rgb(8% 7% 4%)";
const CURSOR_MID = "rgb(15% 14% 12%)";
const CURSOR_EDGE = "rgb(30% 28% 24%)";
const CURSOR_LIGHT = "rgb(93% 93% 93%)";
const GEMINI_BLUE = "rgb(27.8% 58.8% 89.4%)";
const GEMINI_PURPLE = "rgb(51.8% 47.8% 80.8%)";
const GEMINI_ROSE = "rgb(76.5% 40.4% 49.8%)";

function polygon(context: Canvas, [first, ...rest]: readonly Point[]) {
  context.beginPath();
  context.moveTo(...first);
  for (const point of rest) context.lineTo(...point);
  context.closePath();
}

function cursorGem(context: Canvas, lift: number, shimmer = 0) {
  const centerY = 10 + lift;
  const top = [7.5, centerY - 4.5] as const;
  const topRight = [12.5, centerY - 4.5 * 0.45] as const;
  const bottomRight = [12.5, centerY + 4.5 * 0.45] as const;
  const bottom = [7.5, centerY + 4.5] as const;
  const bottomLeft = [2.5, centerY + 4.5 * 0.45] as const;
  const topLeft = [2.5, centerY - 4.5 * 0.45] as const;
  const center = [7.5, centerY] as const;
  polygon(context, [topLeft, top, center, bottomLeft]);
  context.fillStyle = CURSOR_DARK;
  context.fill();
  polygon(context, [top, topRight, bottomRight, center]);
  context.fillStyle = CURSOR_MID;
  context.fill();
  polygon(context, [bottomLeft, center, bottomRight, bottom]);
  context.fillStyle = CURSOR_EDGE;
  context.fill();
  polygon(context, [[8.5, centerY - 4], [12, topRight[1] + 0.3], [8, centerY + 0.5]]);
  context.fillStyle = `rgb(93% 93% 93% / ${0.7 + shimmer * 0.3})`;
  context.fill();
  polygon(context, [top, topRight, bottomRight, bottom, bottomLeft, topLeft]);
  context.strokeStyle = "rgb(93% 93% 93% / 0.35)";
  context.lineWidth = 0.5;
  context.stroke();
}

function cursorEyes(context: Canvas, lift: number, scale: number, color = CURSOR_LIGHT) {
  const height = 1.3 * scale;
  const y = 9.5 + (1.3 - height) / 2 + lift;
  rect(context, color, 4.2, y, 1.3, Math.max(0.3, height));
  rect(context, color, 6.8, y, 1.3, Math.max(0.3, height));
}

function cursorLegs(context: Canvas) {
  rect(context, CURSOR_EDGE, 5.5, 14.5, 1, 1.5);
  rect(context, CURSOR_EDGE, 8.5, 14.5, 1, 1.5);
}

function geminiStar(context: Canvas, lift: number, scale: number, degrees = 0) {
  const centerY = 10 + lift;
  const outer = 4.5 * scale;
  const inner = 1.8 * scale;
  const rotation = degrees * Math.PI / 180;
  const vertices = Array.from({ length: 8 }, (_, index): Point => {
    const angle = index * Math.PI / 4 - Math.PI / 2 + rotation;
    const radius = index % 2 === 0 ? outer : inner;
    return [7.5 + Math.cos(angle) * radius, centerY + Math.sin(angle) * radius];
  });
  polygon(context, vertices);
  const gradient = context.createLinearGradient(7.5 - outer, centerY - outer, 7.5 + outer, centerY + outer);
  gradient.addColorStop(0, GEMINI_BLUE);
  gradient.addColorStop(0.5, GEMINI_PURPLE);
  gradient.addColorStop(1, GEMINI_ROSE);
  context.fillStyle = gradient;
  context.fill();
}

function geminiEyes(context: Canvas, lift: number, scale: number) {
  const height = 1.5 * scale;
  const y = 9.5 + (1.5 - height) / 2 + lift;
  rect(context, "white", 5.5, y, 1.2, Math.max(0.3, height));
  rect(context, "white", 8.3, y, 1.2, Math.max(0.3, height));
}

function geminiLegs(context: Canvas, lift: number) {
  const color = "rgb(51.8% 47.8% 80.8% / 0.7)";
  const y = 14 + lift * 0.3;
  rect(context, color, 5.5, y, 1, 2);
  rect(context, color, 8.5, y, 1, 2);
}

function sleepLetters(context: Canvas, size: number, time: number, drift: number) {
  context.textAlign = "center";
  context.textBaseline = "middle";
  for (let index = 0; index < 3; index++) {
    const cycle = 2.8 + index * 0.3;
    const phase = Math.max(0, ((time - index * 0.9) % cycle) / cycle);
    const fontSize = Math.max(6, size * (0.18 + phase * 0.10));
    const opacity = (0.7 - index * 0.1) * (phase < 0.8 ? 1 : (1 - phase) * 3.5);
    context.font = `900 ${fontSize}px "JetBrains Mono", monospace`;
    context.fillStyle = `rgb(255 255 255 / ${opacity})`;
    context.fillText(
      "z",
      size * (0.65 + index * 0.08 + Math.sin(phase * TAU) * drift),
      size * (0.35 - phase * 0.38),
    );
  }
}

function cursorSleep(context: Canvas, size: number, time: number) {
  const lift = Math.sin(time * TAU / 4.32) * 0.51 + Math.sin(time * TAU / 6.29) * 0.27;
  const shadow = 7 + Math.abs(lift) * 0.2;
  context.save();
  viewport(context, size, 15, 12, 4);
  rect(context, "rgb(0 0 0 / 0.2)", 7.5 - shadow / 2, 15.5, shadow, 1);
  cursorLegs(context);
  cursorGem(context, lift);
  cursorEyes(context, lift, 0.3, "rgb(93% 93% 93% / 0.4)");
  context.restore();
  sleepLetters(context, size, time, 0);
}

function geminiSleep(context: Canvas, size: number, time: number) {
  const lift = Math.sin(time * TAU / 4.67) * 0.68 + Math.sin(time * TAU / 7.01) * 0.36;
  const spin = Math.sin((time / 4.67 % 1) * TAU) * 5;
  const blinkCycle = time % 4;
  const eyeScale = blinkCycle > 3.5 && blinkCycle < 3.7 ? 0.15 : 0.5;
  const shadow = 6 + Math.abs(lift) * 0.3;
  context.save();
  viewport(context, size, 15, 12, 4);
  rect(context, "rgb(0 0 0 / 0.2)", 7.5 - shadow / 2, 15, shadow, 1);
  geminiLegs(context, lift);
  geminiStar(context, lift, 0.9, spin);
  geminiEyes(context, lift, eyeScale);
  context.restore();
  sleepLetters(context, size, time, 0.03);
}

function workLift(time: number, pause: number) {
  return Math.sin(time * TAU / 0.4) * (1 - pause) + Math.sin(time * TAU / 2.9) * 0.3 * pause;
}

function keyboard(context: Canvas, time: number, base: string, key: string, flash: string) {
  rect(context, base, 0, 13, 15, 3);
  for (let row = 0; row < 2; row++) {
    for (let column = 0; column < 6; column++) {
      rect(context, key, 0.5 + column * 2.4, 13.5 + row * 1.2, 1.8, 0.7);
    }
  }
  const phase = Math.trunc(time / 0.1) % 6;
  rect(context, flash, 0.5 + phase * 2.4, 13.5 + Math.trunc(phase / 3) * 1.2, 1.8, 0.7);
}

function cursorWork(context: Canvas, size: number, time: number) {
  const lift = workLift(time, quirk(time, 11, 1.2, 0xe2e));
  const shimmer = Math.sin(time * TAU / 1.5) * 0.5 + 0.5;
  const shadow = 8 - Math.abs(lift) * 0.3;
  viewport(context, size, 16, 14, 3);
  rect(context, `rgb(0 0 0 / ${Math.max(0.1, 0.35 - Math.abs(lift) * 0.03)})`, 4 + (8 - shadow) / 2, 16, shadow, 1);
  cursorLegs(context);
  keyboard(context, time, "rgb(12% 11% 8%)", "rgb(30% 28% 22%)", "rgb(93% 93% 93% / 0.9)");
  cursorGem(context, lift, shimmer);
  cursorEyes(context, lift, Math.max(0.1, blink(time, 0xe2f)));
}

function geminiWork(context: Canvas, size: number, time: number) {
  const lift = workLift(time, quirk(time, 12.9, 1.2, 0x40d));
  const spin = Math.sin(time * TAU / 1.2) * 15;
  const shadow = 7 - Math.abs(lift) * 0.3;
  viewport(context, size, 16, 14, 3);
  rect(context, `rgb(0 0 0 / ${Math.max(0.1, 0.35 - Math.abs(lift) * 0.03)})`, 4 + (7 - shadow) / 2, 16, shadow, 1);
  geminiLegs(context, lift);
  keyboard(context, time, "rgb(22% 25% 38%)", "rgb(40% 44% 58%)", "rgb(255 255 255 / 0.9)");
  geminiStar(context, lift, 1, spin);
  geminiEyes(context, lift, Math.max(0.1, blink(time, 0x40e)));
}

const ALERT_JUMPS = [
  [0, 0], [0.03, 0], [0.10, -1], [0.15, 1.5], [0.175, -8], [0.20, -8], [0.25, 1.5],
  [0.275, -6], [0.30, -6], [0.35, 1.0], [0.375, -4], [0.40, -4], [0.45, 0.8],
  [0.475, -2], [0.50, -2], [0.55, 0.3], [0.62, 0], [1.0, 0],
] as const;

function waitingFrame(context: Canvas, size: number, time: number, width: number) {
  alertGlow(context, ALERT, size, time);
  viewport(context, size, 16, 14, 3);
  const phase = time % 3.5 / 3.5;
  const jump = interpolate(ALERT_JUMPS, phase);
  const height = Math.abs(Math.min(0, jump));
  const shadow = width * (1 - height * 0.04);
  rect(context, `rgb(0 0 0 / ${Math.max(0.08, 0.4 - height * 0.04)})`, 4 + (width - shadow) / 2, 16, shadow, 1);
  return {
    phase,
    jump,
    shake: phase > 0.15 && phase < 0.55 ? Math.sin(phase * 80) * 0.6 : 0,
    emphatic: phase > 0.03 && phase < 0.55,
    eyeScale: phase > 0.03 && phase < 0.15 ? 1.3 : 1,
  };
}

function cursorWait(context: Canvas, size: number, time: number) {
  const { phase, jump, shake, emphatic, eyeScale } = waitingFrame(context, size, time, 8);
  cursorLegs(context);
  context.save();
  context.translate(shake, 0);
  cursorGem(context, jump, emphatic ? Math.sin(phase * 30) * 0.5 + 0.5 : 0);
  cursorEyes(context, jump, eyeScale, emphatic && Math.sin(phase * 25) > 0 ? ALERT : CURSOR_LIGHT);
  context.restore();
  exclamation(context, ALERT, phase, 4, jump);
}

function geminiWait(context: Canvas, size: number, time: number) {
  const { phase, jump, shake, emphatic, eyeScale } = waitingFrame(context, size, time, 7);
  geminiLegs(context, jump);
  context.save();
  context.translate(shake, 0);
  geminiStar(context, jump, emphatic ? 1 + Math.sin(phase * 20) * 0.15 : 1);
  geminiEyes(context, jump, eyeScale);
  context.restore();
  exclamation(context, ALERT, phase, 4, jump);
}

export const cursorPoses = { idle: cursorSleep, working: cursorWork, waiting: cursorWait };
export const geminiPoses = { idle: geminiSleep, working: geminiWork, waiting: geminiWait };
