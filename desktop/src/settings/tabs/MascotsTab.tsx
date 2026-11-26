import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AGENT_KINDS, agentLabelKey } from "../../lib/agents";
import {
  MASCOT_SPEED_MAX,
  MASCOT_SPEED_MIN,
  MASCOT_SPEED_STEP,
} from "../../lib/defaults";
import type { StatusKey } from "../../lib/status";
import type { AgentKind } from "../../lib/types";
import { Mascot } from "../../components/Mascot";
import {
  SegmentedControl,
  SettingRow,
  SettingsSection,
  SliderField,
  SelectField,
  type SettingsTabProps,
  useControlId,
} from "../controls";

/** Display names from the macOS mascot roster. */
const MASCOT_NAMES: Record<Exclude<AgentKind, "unknown">, string> = {
  claude: "Clawd",
  codex: "Dex",
  gemini: "Gemini",
  cursor: "CursorBot",
  opencode: "OpBot",
  qoder: "QoderBot",
  droid: "Droid",
  codebuddy: "Buddy",
  copilot: "CopilotBot",
  grok: "Grok",
  kimi: "KimiBot",
  cline: "ClineBot",
  pi: "Pi",
  qwen: "QwenBot",
  hermes: "Hermes",
  antigravity: "AntiGravity",
  kiro: "Kiro",
  stepfun: "StepFun",
  workbuddy: "WorkBuddy",
  openclaw: "Molty",
  zcode: "ZCodeBot",
  minimax: "MiniMax",
  deepseek: "DeepSeek",
  aiwork: "AiWorkBot",
  mimo: "MiMoBot",
  trae: "TraeBot",
  traecn: "TraeCNBot",
  traecli: "TraeCLIBot",
  codybuddycn: "BuddyCN",
  qodercn: "QoderCNBot",
  qoderwork: "QoderWorkBot",
};

/** The full 31-agent roster, in the same order as the wire enum. */
const MASCOTS = AGENT_KINDS.filter(
  (agent): agent is Exclude<AgentKind, "unknown"> => agent !== "unknown",
).map((agent) => ({ agent, name: MASCOT_NAMES[agent] }));

export function MascotsTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const speedId = useControlId("mascot-speed");
  const defaultId = useControlId("default-mascot");
  const [preview, setPreview] = useState<StatusKey>("thinking");

  const defaultOptions: Array<{ value: string; label: string }> = MASCOTS.map(
    (mascot) => ({
      value: mascot.agent,
      label: `${mascot.name} — ${t(agentLabelKey(mascot.agent))}`,
    }),
  );
  if (!MASCOTS.some((mascot) => mascot.agent === settings.default_mascot)) {
    defaultOptions.push({
      value: settings.default_mascot,
      label: settings.default_mascot,
    });
  }

  return (
    <>
      <SettingsSection>
        <SettingRow htmlFor={speedId} label={t("settings.mascots.previewStatus")}>
          <SegmentedControl
            value={preview}
            onChange={setPreview}
            options={[
              { value: "thinking", label: t("settings.mascots.processing") },
              { value: "idle", label: t("settings.mascots.idle") },
              { value: "needs_you", label: t("settings.mascots.waiting") },
            ]}
          />
        </SettingRow>
        <SettingRow
          htmlFor={speedId}
          label={t("settings.mascots.speed")}
          description={t("settings.mascots.speedHint")}
        >
          <div className="flex items-center gap-3">
            <SliderField
              id={speedId}
              value={settings.mascot_speed}
              min={MASCOT_SPEED_MIN}
              max={MASCOT_SPEED_MAX}
              step={MASCOT_SPEED_STEP}
              onChange={(mascot_speed) => onChange({ mascot_speed })}
            />
            <span className="w-10 text-right text-[12px] tabular-nums text-white/55">
              {settings.mascot_speed === 0
                ? t("settings.mascots.speedOff")
                : `${(settings.mascot_speed / 100).toFixed(1)}×`}
            </span>
          </div>
        </SettingRow>
        <SettingRow
          htmlFor={defaultId}
          label={t("settings.mascots.defaultMascot")}
          description={t("settings.mascots.defaultMascotHint")}
        >
          <SelectField
            id={defaultId}
            value={settings.default_mascot}
            onChange={(default_mascot) => onChange({ default_mascot })}
            options={defaultOptions}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection>
        {MASCOTS.map((mascot) => (
          <div
            key={mascot.agent}
            className="flex items-center gap-4 border-b border-white/[0.06] px-3.5 py-2.5 last:border-b-0"
          >
            <span className="grid h-14 w-14 flex-none place-items-center rounded-[10px] bg-black">
              <span className="settings-mascot-preview block h-10 w-10">
                <Mascot agent={mascot.agent} status={preview} speedPercent={settings.mascot_speed} />
              </span>
            </span>
            <span className="min-w-0">
              <span className="block font-mono text-[14px] font-bold text-white/90">
                {mascot.name}
              </span>
              <span className="block text-[12px] text-white/45">
                {t(agentLabelKey(mascot.agent))}
              </span>
            </span>
          </div>
        ))}
      </SettingsSection>
    </>
  );
}
