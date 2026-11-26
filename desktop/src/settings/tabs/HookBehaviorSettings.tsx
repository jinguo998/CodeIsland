import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { claudeConfigPath } from "../../lib/ipc";
import {
  SettingRow,
  SettingsSection,
  TextField,
  Toggle,
  type SettingsTabProps,
} from "../controls";

const AUTO_APPROVE_TOOLS = [
  "TaskCreate",
  "TaskUpdate",
  "TaskGet",
  "TaskList",
  "TaskOutput",
  "TaskStop",
  "TodoRead",
  "TodoWrite",
  "EnterPlanMode",
  "ExitPlanMode",
] as const;

export function HookBehaviorSettings({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const toolsId = useId();
  const sourcesId = useId();
  const pathsId = useId();
  const claudeConfigId = useId();
  const webhookEnabledId = useId();
  const webhookUrlId = useId();
  const webhookFilterId = useId();
  const approvedTools = new Set(settings.auto_approve_tools.split(",").filter(Boolean));
  const [resolvedClaudeConfig, setResolvedClaudeConfig] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void claudeConfigPath(settings.claude_config_dir)
      .then((path) => {
        if (!cancelled) setResolvedClaudeConfig(path);
      })
      .catch(() => {
        if (!cancelled) setResolvedClaudeConfig(null);
      });
    return () => {
      cancelled = true;
    };
  }, [settings.claude_config_dir]);

  return (
    <>
      <SettingsSection title={t("settings.behavior.hooks.autoApproveTools")}>
        <p className="px-3.5 py-2 text-[11px] leading-snug text-white/45">
          {t("settings.behavior.hooks.autoApproveToolsHint")}
        </p>
        {AUTO_APPROVE_TOOLS.map((tool) => {
          const id = `${toolsId}-${tool}`;
          return (
            <SettingRow
              key={tool}
              htmlFor={id}
              label={tool}
              description={t(`settings.behavior.hooks.toolDescriptions.${tool}`)}
            >
              <Toggle
                id={id}
                label={tool}
                checked={approvedTools.has(tool)}
                onChange={(enabled) => {
                  const next = new Set(approvedTools);
                  if (enabled) next.add(tool);
                  else next.delete(tool);
                  onChange({ auto_approve_tools: [...next].sort().join(",") });
                }}
              />
            </SettingRow>
          );
        })}
        <SettingRow
          htmlFor={sourcesId}
          label={t("settings.behavior.hooks.autoApproveSources")}
          description={t("settings.behavior.hooks.autoApproveSourcesHint")}
        >
          <TextField
            id={sourcesId}
            value={settings.auto_approve_sources}
            placeholder={t("settings.behavior.hooks.autoApproveSourcesPlaceholder")}
            onChange={(auto_approve_sources) => onChange({ auto_approve_sources })}
            className="w-48"
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection>
        <SettingRow
          htmlFor={claudeConfigId}
          label={t("settings.behavior.hooks.claudeConfig")}
          description={t("settings.behavior.hooks.claudeConfigHint")}
        >
          <TextField
            id={claudeConfigId}
            value={settings.claude_config_dir}
            placeholder={t("settings.behavior.hooks.claudeConfigPlaceholder")}
            onChange={(claude_config_dir) => onChange({ claude_config_dir })}
            className="w-48"
          />
        </SettingRow>
        {resolvedClaudeConfig && (
          <p className="break-all px-3.5 py-2 text-[11px] text-white/45">
            {t("settings.behavior.hooks.claudeConfigResolved", { path: resolvedClaudeConfig })}
          </p>
        )}
      </SettingsSection>

      <SettingsSection>
        <SettingRow
          htmlFor={pathsId}
          label={t("settings.behavior.hooks.excludedPaths")}
          description={t("settings.behavior.hooks.excludedPathsHint")}
        >
          <TextField
            id={pathsId}
            value={settings.excluded_hook_cwd_substrings}
            placeholder={t("settings.behavior.hooks.excludedPathsPlaceholder")}
            onChange={(excluded_hook_cwd_substrings) =>
              onChange({ excluded_hook_cwd_substrings })
            }
            className="w-48"
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection title={t("settings.behavior.hooks.webhook")}>
        <SettingRow
          htmlFor={webhookEnabledId}
          label={t("settings.behavior.hooks.webhookEnabled")}
          description={t("settings.behavior.hooks.webhookHint")}
        >
          <Toggle
            id={webhookEnabledId}
            label={t("settings.behavior.hooks.webhookEnabled")}
            checked={settings.webhook_enabled}
            onChange={(webhook_enabled) => onChange({ webhook_enabled })}
          />
        </SettingRow>
        {settings.webhook_enabled && (
          <>
            <SettingRow htmlFor={webhookUrlId} label={t("settings.behavior.hooks.webhookUrl")}>
              <TextField
                id={webhookUrlId}
                value={settings.webhook_url}
                placeholder={t("settings.behavior.hooks.webhookUrlPlaceholder")}
                onChange={(webhook_url) => onChange({ webhook_url })}
                className="w-56"
              />
            </SettingRow>
            <SettingRow
              htmlFor={webhookFilterId}
              label={t("settings.behavior.hooks.webhookFilter")}
              description={t("settings.behavior.hooks.webhookFilterHint")}
            >
              <TextField
                id={webhookFilterId}
                value={settings.webhook_event_filter}
                placeholder={t("settings.behavior.hooks.webhookFilterPlaceholder")}
                onChange={(webhook_event_filter) => onChange({ webhook_event_filter })}
                className="w-56"
              />
            </SettingRow>
          </>
        )}
      </SettingsSection>
    </>
  );
}
