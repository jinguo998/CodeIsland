const iconFiles: Record<string, string> = {
  claude: "claude",
  codex: "codex",
  gemini: "gemini",
  antigravity: "antigravity",
  "google-antigravity": "gemini",
  cursor: "cursor",
  "cursor-cli": "cursor",
  trae: "trae",
  traecn: "trae",
  traecli: "trae",
  "traecli-next": "trae",
  copilot: "copilot",
  qoder: "qoder",
  "qoder-cli": "qoder",
  qoderwork: "qoder",
  droid: "factory",
  codebuddy: "codebuddy",
  codybuddycn: "codebuddy",
  stepfun: "stepfun",
  workbuddy: "workbuddy",
  hermes: "hermes",
  grok: "grok",
  qwen: "qwen",
  kimi: "kimi",
  pi: "pi",
  omp: "pi",
  opencode: "opencode",
  cline: "cline",
  deepseek: "dsh",
  dsh: "dsh",
  aiwork: "aiwork",
  "aiwork-cli": "aiwork",
  kiro: "kiro",
  openclaw: "openclaw",
  minimax: "minimax",
  mimo: "mimo",
};

export function CliIcon({ source }: { source: string }) {
  const filename = iconFiles[source];
  if (filename) {
    return <img className="cli-icon" src={`/cli-icons/${filename}.png`} alt="" />;
  }

  const hash = Array.from(source).reduce(
    (value, letter) => (value * 31 + (letter.codePointAt(0) ?? 0)) & 0x7fffffff,
    0,
  );
  return (
    <span
      className="cli-icon cli-icon-monogram"
      style={{ backgroundColor: `hsl(${hash % 360} 50% 42%)` }}
      aria-hidden="true"
    >
      {source.replace(/^\.+/, "").slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}
