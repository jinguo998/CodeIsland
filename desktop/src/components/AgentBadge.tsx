import { useTranslation } from "react-i18next";
import { agentColor, agentGlyph, agentLabelKey } from "../lib/agents";
import type { AgentKind } from "../lib/types";

/** CSS-only pixel badge standing in for future agent icons. */
export function AgentBadge({
  agent,
  className = "",
}: {
  agent: AgentKind;
  className?: string;
}) {
  const { t } = useTranslation();
  const label = t(agentLabelKey(agent));
  const color = agentColor(agent);
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={`pixelated inline-flex h-6 w-6 flex-none items-center justify-center border bg-island text-[11px] font-bold ${className}`}
      style={{
        color,
        borderColor: color,
        boxShadow: "2px 2px 0 0 var(--pixel-shadow)",
      }}
    >
      {agentGlyph(agent)}
    </span>
  );
}
