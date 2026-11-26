// Canvas scenes ported from PixelCharacterView.swift and DexView.swift.
export type MascotPose = "idle" | "working" | "waiting";
type Canvas = CanvasRenderingContext2D;
type Keyframe = readonly [time: number, value: number];

const APPLE_EPOCH_SECONDS = 978_307_200;
const TAU = 2 * Math.PI;
const CLAWD_BODY = "rgb(87.1% 53.3% 42.7%)";
const CLAWD_ALERT = "rgb(100% 24% 0%)";
const DEX_CLOUD = "rgb(92% 92% 93%)";
const DEX_ALERT = "rgb(100% 55% 0%)";

export function mascotTime(interval: number, speed: number) {
  return Math.floor((Date.now() / 1000 - APPLE_EPOCH_SECONDS) / interval) * interval * speed;
}

function hash(slot: number, seed: number) {
  const wrap = (value: bigint) => BigInt.asUintN(64, value);
  let x = wrap(BigInt(slot) + 0x9e3779b97f4a7c15n + BigInt(seed) * 0xbf58476d1ce4e5b9n);
  x = wrap((x ^ (x >> 30n)) * 0xbf58476d1ce4e5b9n);
  x = wrap((x ^ (x >> 27n)) * 0x94d049bb133111ebn);
  return Number((x ^ (x >> 31n)) % 1_000_000n) / 1_000_000;
}

export function quirk(time: number, cycle: number, duration: number, seed: number) {
  const slot = Math.floor(time / cycle);
  if (hash(slot, seed ^ 0x9d2c) <= 0.3) return 0;
  const start = cycle * (0.2 + 0.55 * hash(slot, seed));
  const local = time - slot * cycle - start;
  return local >= 0 && local < duration ? Math.sin(Math.PI * local / duration) : 0;
}

function breath(time: number) {
  const phase = time / 4.5 % 1;
  if (phase < 0.35) return 0.5 - 0.5 * Math.cos(Math.PI * phase / 0.35);
  if (phase < 0.8) return 0.5 + 0.5 * Math.cos(Math.PI * (phase - 0.35) / 0.45);
  return 0;
}

export function blink(time: number, seed: number) {
  const slot = Math.floor(time / 4);
  const local = time - slot * 4;
  const start = 4 * (0.15 + 0.6 * hash(slot, seed));
  const lid = (elapsed: number) => {
    if (elapsed < 0 || elapsed >= 0.14) return 1;
    const phase = elapsed / 0.14;
    return phase < 0.4 ? 1 - phase / 0.4 : (phase - 0.4) / 0.6;
  };
  const openness = lid(local - start);
  return hash(slot, seed ^ 0xb11) < 0.18 ? Math.min(openness, lid(local - start - 0.22)) : openness;
}

function stroke(time: number, cadence: number, seed: number) {
  const slot = Math.floor(time / cadence);
  return { slot, active: hash(slot, seed) > 0.18 };
}

export function interpolate(frames: readonly Keyframe[], time: number) {
  for (let index = 1; index < frames.length; index++) {
    const [end, target] = frames[index];
    if (time <= end) {
      const [start, from] = frames[index - 1];
      return from + (target - from) * (time - start) / (end - start);
    }
  }
  return frames[frames.length - 1][1];
}

export function viewport(context: Canvas, size: number, width: number, height: number, top: number) {
  const scale = size / Math.max(width, height);
  context.translate((size - width * scale) / 2, (size - height * scale) / 2);
  context.scale(scale, scale);
  context.translate(0, -top);
}

export function rect(context: Canvas, color: string, x: number, y: number, width: number, height: number) {
  context.fillStyle = color;
  context.fillRect(x, y, width, height);
}

function arm(context: Canvas, x: number, pivot: number, angle: number, rise: number) {
  context.save();
  context.translate(pivot, 10 + rise);
  context.rotate(angle * Math.PI / 180);
  rect(context, CLAWD_BODY, x - pivot, -1, 2, 2);
  context.restore();
}

function sleepLetters(context: Canvas, size: number, time: number) {
  context.textAlign = "center";
  context.textBaseline = "middle";
  for (let index = 0; index < 3; index++) {
    const cycle = 2.8 + index * 0.3;
    const phase = Math.max(0, ((time - index * 0.9) % cycle) / cycle);
    const fontSize = Math.max(6, size * (0.18 + phase * 0.1));
    const opacity = (0.7 - index * 0.1) * (phase < 0.8 ? 1 : (1 - phase) * 3.5);
    const x = size * (0.58 + index * 0.06 + Math.sin(phase * TAU) * 0.03);
    const y = size * (0.35 - phase * 0.38);
    context.font = `900 ${fontSize}px "JetBrains Mono", monospace`;
    context.fillStyle = `rgb(255 255 255 / ${opacity})`;
    context.fillText("z", x, y);
  }
}

export function alertGlow(context: Canvas, color: string, size: number, time: number) {
  context.save();
  context.filter = `blur(${size * 0.05}px)`;
  context.globalAlpha = 0.12 * (0.5 + 0.5 * Math.cos(TAU * time));
  context.fillStyle = color;
  context.beginPath();
  context.arc(size / 2, size / 2, size * 0.4, 0, TAU);
  context.fill();
  context.restore();
}

const BANG_OPACITY = [[0, 0], [0.03, 1], [0.1, 1], [0.55, 1], [0.62, 0], [1, 0]] as const;
const BANG_SCALE = [[0, 0.3], [0.03, 1.3], [0.1, 1], [0.55, 1], [0.62, 0.6], [1, 0.6]] as const;

export function exclamation(context: Canvas, color: string, progress: number, top: number, jump: number) {
  const opacity = interpolate(BANG_OPACITY, progress);
  if (opacity <= 0.01) return;
  const scale = interpolate(BANG_SCALE, progress);
  const y = top + jump * 0.15;
  context.save();
  context.globalAlpha = opacity;
  rect(context, color, 13, y, 2 * scale, 3.5 * scale);
  rect(context, color, 13, y + 4 * scale, 2 * scale, 1.5 * scale);
  context.restore();
}

function clawdSleep(context: Canvas, size: number, time: number) {
  context.save();
  viewport(context, size, 17, 7, 9);
  const breathe = breath(time);
  const puff = breathe * 0.25;
  const width = 13 * (1 + breathe * 0.015);
  const height = 5 * (1 + puff);
  const kick = 0.9 * quirk(time, 6, 0.55, 0xc1a);
  const kickLeg = Math.floor(hash(Math.floor(time / 6), 0xc1a ^ 0x51de) * 4);
  const eyeHeight = 1 - quirk(time, 9, 0.7, 0x5ee) * 0.5;
  const eyeY = 12.2 - puff * 2.5 + (1 - eyeHeight) / 2;
  rect(context, `rgb(0 0 0 / ${0.35 + breathe * 0.08})`, -1, 15, 17 * (1 + breathe * 0.03), 1);
  [3, 5, 9, 11].forEach((x, index) => {
    const lift = index === kickLeg ? kick : 0;
    rect(context, CLAWD_BODY, x, 8.5 - lift, 1, 1.5 + lift);
  });
  rect(context, CLAWD_BODY, 1 - (width - 13) / 2, 15 - height, width, height);
  rect(context, CLAWD_BODY, -1, 13, 2, 2);
  rect(context, CLAWD_BODY, 14, 13, 2, 2);
  for (const x of [3, 9.5]) rect(context, "black", x, eyeY, 2.5, eyeHeight);
  context.restore();
  sleepLetters(context, size, time);
}

function clawdWork(context: Canvas, size: number, time: number) {
  viewport(context, size, 16, 11, 5.5);
  const pause = quirk(time, 11, 1.4, 0x7a9);
  const intensity = 1 - pause;
  const bounce = Math.sin(time * TAU / 0.35) * 1.2 * intensity + Math.sin(time * TAU / 2.8) * 0.35 * pause;
  const left = stroke(time, 0.15, 0x1ef7);
  const right = stroke(time, 0.12, 0x819);
  const leftArm = (left.active ? Math.sin(time * TAU / 0.15) : -0.6) * intensity;
  const rightArm = (right.active ? Math.sin(time * TAU / 0.12) : -0.6) * intensity;
  const scan = time % 10;
  const wideEyes = (scan > 5.7 && scan < 6.9) || pause > 0.3;
  const eyeHeight = (wideEyes ? 2 : 1) * Math.max(0.1, blink(time, 0xb1));
  const eyeY = 8 + (2 - eyeHeight) / 2 + (wideEyes ? -0.5 : 1);
  const shadow = 9 - Math.abs(bounce) * 0.3;
  const torso = 11 * (1 + Math.sin(time * TAU / 3.2) * 0.015);
  rect(context, `rgb(0 0 0 / ${Math.max(0.1, 0.4 - Math.abs(bounce) * 0.03)})`, 3 + (9 - shadow) / 2, 15, shadow, 1);
  for (const x of [3, 5, 9, 11]) rect(context, CLAWD_BODY, x, 13, 1, 2);
  rect(context, CLAWD_BODY, 2 - (torso - 11) / 2, 6 + bounce, torso, 7);
  for (const x of [4, 10]) rect(context, "black", x, eyeY + bounce, 1, eyeHeight);
  rect(context, "rgb(38% 44% 50%)", -0.5, 11.8, 16, 3.5);
  for (let row = 0; row < 3; row++) {
    for (let column = 0; column < 6; column++) {
      rect(context, "rgb(60% 66% 72%)", 0.3 + column * 2.5, 12.2 + row, column === 2 && row === 1 ? 4.5 : 2, 0.7);
    }
  }
  if (left.active && leftArm > 0.3) {
    const column = Math.floor(hash(left.slot, 0xface0) * 3);
    rect(context, "rgb(255 255 255 / .9)", 0.3 + column * 2.5, 12.2 + column % 3, 2, 0.7);
  }
  if (right.active && rightArm > 0.3) {
    const column = 3 + Math.floor(hash(right.slot, 0xface1) * 3);
    rect(context, "rgb(255 255 255 / .9)", 0.3 + column * 2.5, 12.2 + (column - 3) % 3, 2, 0.7);
  }
  arm(context, 0, 2, leftArm * 22.5 - 32.5, bounce);
  arm(context, 13, 13, rightArm * 22.5 + 32.5, bounce);
}

const CLAWD_JUMP = [
  [0, 0], [0.03, 0], [0.1, -1], [0.15, 1.5], [0.175, -10], [0.2, -10], [0.25, 1.5],
  [0.275, -8], [0.3, -8], [0.35, 1.2], [0.375, -5], [0.4, -5], [0.45, 1],
  [0.475, -3], [0.5, -3], [0.55, 0.5], [0.62, 0], [1, 0],
] as const;

const CLAWD_WAVE = [
  [0, 0], [0.03, 0], [0.1, 25], [0.15, 30], [0.2, 155], [0.25, 115], [0.3, 140],
  [0.35, 100], [0.4, 115], [0.45, 80], [0.5, 80], [0.55, 40], [0.62, 0], [1, 0],
] as const;

const CLAWD_RIGHT_WAVE = CLAWD_WAVE.map(([phase, angle]) => [phase, phase === 0.1 ? 30 : angle] as const);

function clawdWait(context: Canvas, size: number, time: number) {
  alertGlow(context, CLAWD_ALERT, size, time);
  viewport(context, size, 15, 12, 4);
  const phase = time % 3.5 / 3.5;
  const jump = interpolate(CLAWD_JUMP, phase);
  const squash = jump > 0.5 ? jump : 0;
  const torsoWidth = 11 * (1 + squash * 0.05);
  const torsoHeight = 7 * (1 - squash * 0.04);
  const wideEyes = phase > 0.03 && phase < 0.15;
  const eyeHeight = wideEyes ? 2.6 : 2;
  const eyeY = 8 + (2 - eyeHeight) / 2 + (wideEyes ? -0.5 : 0);
  const rise = Math.abs(Math.min(0, jump));
  const shadow = 9 * (1 - rise * 0.04);
  rect(context, `rgb(0 0 0 / ${Math.max(0.08, 0.5 - rise * 0.04)})`, 3 + (9 - shadow) / 2, 15, shadow, 1);
  for (const x of [3, 5, 9, 11]) rect(context, CLAWD_BODY, x, 11, 1, 4);
  rect(context, CLAWD_BODY, 2 - (torsoWidth - 11) / 2, 13 - torsoHeight + jump, torsoWidth, torsoHeight);
  for (const x of [4, 10]) rect(context, "black", x, eyeY + jump, 1, eyeHeight);
  arm(context, 0, 2, interpolate(CLAWD_WAVE, phase), jump);
  arm(context, 13, 13, -interpolate(CLAWD_RIGHT_WAVE, phase), jump);
  exclamation(context, CLAWD_ALERT, phase, 4.5, jump);
}

const CLOUD_ROWS = [
  [14, 4, 7], [13, 3, 9], [12, 2, 11], [11, 1, 13], [10, 1, 13], [9, 1, 13],
  [8, 2, 11], [7, 2, 11], [6, 3, 3], [6, 6, 3], [6, 9, 3], [5, 4, 2], [5, 6.5, 2], [5, 9, 2],
] as const;

function cloud(context: Canvas, lift: number, scaleX = 1, scaleY = 1) {
  for (const [y, x, width] of CLOUD_ROWS) {
    rect(context, DEX_CLOUD, 7.5 + (x - 7.5) * scaleX, y * scaleY + (1 - scaleY) * 10 + lift, width * scaleX, scaleY);
  }
}

function cloudLegs(context: Canvas) {
  for (const x of [5, 9]) rect(context, "rgb(70% 70% 72%)", x, 14.5, 1, 1.5);
}

function cloudPrompt(context: Canvas, lift: number, color: string, cursor: boolean) {
  for (const [x, y] of [[3, 10], [4, 11], [3, 12]]) rect(context, color, x, y + lift, 1, 1);
  if (cursor) rect(context, color, 6, 12 + lift, 3, 1);
}

function dexSleep(context: Canvas, size: number, time: number) {
  context.save();
  viewport(context, size, 15, 12, 4);
  const drift = Math.sin(time * TAU / 4) * 0.7 + Math.sin(time * TAU / 6.3) * 0.35;
  const pulse = quirk(time, 8, 0.5, 0xde1);
  const shadow = 7 + Math.abs(drift) * 0.3;
  rect(context, "rgb(0 0 0 / .2)", 7.5 - shadow / 2, 15, shadow, 1);
  cloudLegs(context);
  cloud(context, drift);
  if (pulse > 0 ? (pulse * 4) % 1 < 0.5 : time % 1.2 < 0.6) {
    rect(context, pulse > 0 ? "rgb(0 0 0 / .5)" : "rgb(0 0 0 / .3)", 6, 12 + drift, 3, 1);
  }
  context.restore();
  sleepLetters(context, size, time);
}

function dexWork(context: Canvas, size: number, time: number) {
  viewport(context, size, 16, 14, 3);
  const pause = quirk(time, 12, 1.2, 0xde2);
  const bounce = Math.sin(time * TAU / 0.4) * (1 - pause);
  const key = stroke(time, 0.1, 0xde3);
  const shadow = 8 - Math.abs(bounce) * 0.3;
  rect(context, `rgb(0 0 0 / ${Math.max(0.1, 0.35 - Math.abs(bounce) * 0.03)})`, 4 + (8 - shadow) / 2, 16, shadow, 1);
  cloudLegs(context);
  rect(context, "rgb(18% 18% 20%)", 0, 13, 15, 3);
  for (let row = 0; row < 2; row++) {
    for (let column = 0; column < 6; column++) {
      rect(context, "rgb(40% 40% 42%)", 0.5 + column * 2.4, 13.5 + row * 1.2, 1.8, 0.7);
    }
  }
  if (key.active && pause < 0.3) {
    const phase = Math.floor(hash(key.slot, 0xde4) * 6);
    rect(context, "rgb(255 255 255 / .9)", 0.5 + phase * 2.4, 13.5 + Math.floor(phase / 3) * 1.2, 1.8, 0.7);
  }
  cloud(context, bounce);
  cloudPrompt(context, bounce, "black", pause > 0.3 || time % 0.3 < 0.15);
}

export const MASCOT_ALERT_JUMP = [
  [0, 0], [0.03, 0], [0.1, -1], [0.15, 1.5], [0.175, -8], [0.2, -8], [0.25, 1.5],
  [0.275, -6], [0.3, -6], [0.35, 1], [0.375, -4], [0.4, -4], [0.45, 0.8],
  [0.475, -2], [0.5, -2], [0.55, 0.3], [0.62, 0], [1, 0],
] as const;

function dexWait(context: Canvas, size: number, time: number) {
  alertGlow(context, DEX_ALERT, size, time);
  viewport(context, size, 16, 14, 3);
  const phase = time % 3.5 / 3.5;
  const jump = interpolate(MASCOT_ALERT_JUMP, phase);
  const squash = jump > 0.5 ? jump : 0;
  const rise = Math.abs(Math.min(0, jump));
  const shadow = 8 * (1 - rise * 0.04);
  const shake = phase > 0.15 && phase < 0.55 ? Math.sin(phase * 80) * 0.6 : 0;
  const prompt = phase > 0.03 && phase < 0.55 && Math.sin(phase * 25) > 0 ? DEX_ALERT : "black";
  rect(context, `rgb(0 0 0 / ${Math.max(0.08, 0.4 - rise * 0.04)})`, 4 + (8 - shadow) / 2, 16, shadow, 1);
  cloudLegs(context);
  context.save();
  context.translate(shake, 0);
  cloud(context, jump, 1 + squash * 0.03, 1 - squash * 0.02);
  cloudPrompt(context, jump, prompt, true);
  context.restore();
  exclamation(context, DEX_ALERT, phase, 4, jump);
}

export const compactMascotArt = {
  claude: { idle: clawdSleep, working: clawdWork, waiting: clawdWait },
  codex: { idle: dexSleep, working: dexWork, waiting: dexWait },
};
