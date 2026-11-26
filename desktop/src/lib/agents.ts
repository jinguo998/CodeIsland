import type { AgentKind } from "./types";

/** Every wire tag from the Rust `AgentKind` enum, in display order. */
export const AGENT_KINDS: AgentKind[] = [
  "claude",
  "codex",
  "gemini",
  "cursor",
  "opencode",
  "qoder",
  "droid",
  "codebuddy",
  "copilot",
  "grok",
  "kimi",
  "cline",
  "pi",
  "qwen",
  "hermes",
  "antigravity",
  "kiro",
  "stepfun",
  "workbuddy",
  "openclaw",
  "zcode",
  "minimax",
  "deepseek",
  "aiwork",
  "mimo",
  "trae",
  "traecn",
  "traecli",
  "codybuddycn",
  "qodercn",
  "qoderwork",
  "unknown",
];

/** Short pixel-badge glyphs; no external images are used. */
export const AGENT_GLYPHS: Record<AgentKind, string> = {
  claude: "C",
  codex: "X",
  gemini: "G",
  cursor: "▸",
  opencode: "O",
  qoder: "Q",
  droid: "D",
  codebuddy: "B",
  copilot: "P",
  grok: "K",
  kimi: "K",
  cline: "L",
  pi: "π",
  qwen: "W",
  hermes: "H",
  antigravity: "A",
  kiro: "K",
  stepfun: "S",
  workbuddy: "W",
  openclaw: "C",
  zcode: "Z",
  minimax: "M",
  deepseek: "D",
  aiwork: "A",
  mimo: "M",
  trae: "T",
  traecn: "T",
  traecli: "T",
  codybuddycn: "B",
  qodercn: "Q",
  qoderwork: "Q",
  unknown: "?",
};

/** Accent color per agent, used for the badge border and glyph. */
export const AGENT_COLORS: Record<AgentKind, string> = {
  claude: "#d97757",
  codex: "#8fe388",
  gemini: "#7aa2f7",
  cursor: "#c7c7d1",
  opencode: "#f4c95d",
  qoder: "#b48ef7",
  droid: "#ff8a65",
  codebuddy: "#6ee7a8",
  copilot: "#9ad0ff",
  grok: "#e5e5e5",
  kimi: "#7dd3fc",
  cline: "#ffb86c",
  pi: "#f0abfc",
  qwen: "#a3e635",
  hermes: "#fbbf24",
  antigravity: "#60a5fa",
  kiro: "#f472b6",
  stepfun: "#34d399",
  workbuddy: "#fb923c",
  openclaw: "#f87171",
  zcode: "#93c5fd",
  minimax: "#facc15",
  deepseek: "#818cf8",
  aiwork: "#2dd4bf",
  mimo: "#f9a8d4",
  trae: "#e879f9",
  traecn: "#e879f9",
  traecli: "#e879f9",
  codybuddycn: "#5eead4",
  qodercn: "#c084fc",
  qoderwork: "#c084fc",
  unknown: "#8a8a93",
};

export function agentLabelKey(agent: AgentKind): string {
  return `agents.${agent}`;
}

export function agentGlyph(agent: AgentKind): string {
  return AGENT_GLYPHS[agent] ?? "?";
}

export function agentColor(agent: AgentKind): string {
  return AGENT_COLORS[agent] ?? AGENT_COLORS.unknown;
}
