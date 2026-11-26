import { useCallback, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentBadge } from "../../components/AgentBadge";
import { PixelButton } from "../../components/PixelButton";
import { agentLabelKey } from "../../lib/agents";
import * as ipc from "../../lib/ipc";
import type { AgentKind, CustomCliSetting, ToolStatus } from "../../lib/types";
import {
  SelectField,
  SettingRow,
  SettingsSection,
  TextField,
  Toggle,
  type SettingsTabProps,
} from "../controls";

type CustomDraft = Omit<CustomCliSetting, "id">;

const EMPTY_CUSTOM: CustomDraft = {
  name: "",
  source: "",
  config_path: "",
  format: "claude_json",
  config_key: "hooks",
};

export function HooksTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const [statuses, setStatuses] = useState<ToolStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<AgentKind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const extraId = useId();
  const coworkId = useId();
  const [extraDraft, setExtraDraft] = useState("");
  const [customStatuses, setCustomStatuses] = useState<
    Record<string, ToolStatus>
  >({});
  const [customBusy, setCustomBusy] = useState<string | null>(null);
  const [customError, setCustomError] = useState<string | null>(null);
  const [draft, setDraft] = useState<CustomDraft>(EMPTY_CUSTOM);
  const customNameId = useId();
  const customSourceId = useId();
  const customPathId = useId();
  const customKeyId = useId();
  const customTemplateId = useId();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setStatuses(await ipc.hookStatus());
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (
    agent: AgentKind,
    action: (agent?: AgentKind) => Promise<ToolStatus[]>,
  ) => {
    setBusy(agent);
    try {
      setStatuses(await action(agent));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  const addExtraDir = () => {
    const value = extraDraft.trim();
    if (!value) return;
    onChange({
      extra_config_dirs: Array.from(new Set([...settings.extra_config_dirs, value])),
    });
    setExtraDraft("");
  };

  useEffect(() => {
    let cancelled = false;
    for (const item of settings.custom_clis) {
      void ipc
        .customCliStatus(item)
        .then((status) => {
          if (!cancelled) {
            setCustomStatuses((previous) => ({
              ...previous,
              [item.source]: status,
            }));
          }
        })
        .catch(() => {
          // A malformed saved entry must not break the whole page; the row
          // keeps its "not installed" chip and the error surfaces on action.
        });
    }
    return () => {
      cancelled = true;
    };
  }, [settings.custom_clis]);

  const runCustom = async (
    item: CustomCliSetting,
    action: (setting: CustomCliSetting) => Promise<ToolStatus>,
  ) => {
    setCustomBusy(item.source);
    try {
      const status = await action(item);
      setCustomStatuses((previous) => ({ ...previous, [item.source]: status }));
      setCustomError(null);
    } catch (cause) {
      setCustomError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setCustomBusy(null);
    }
  };

  const addCustom = () => {
    const name = draft.name.trim();
    const source = draft.source.trim().toLowerCase();
    const config_path = draft.config_path.trim();
    const config_key = draft.config_key.trim() || "hooks";
    if (!name || !source || !config_path) {
      setCustomError(t("settings.hooks.customInvalid"));
      return;
    }
    if (!/^[a-z0-9_-]+$/.test(source)) {
      setCustomError(t("settings.hooks.customSourceInvalid"));
      return;
    }
    if (settings.custom_clis.some((item) => item.source === source)) {
      setCustomError(t("settings.hooks.customDuplicate"));
      return;
    }
    const entry: CustomCliSetting = {
      id: `${Date.now()}`,
      name,
      source,
      config_path,
      config_key,
      format: draft.format,
    };
    onChange({ custom_clis: [...settings.custom_clis, entry] });
    setDraft(EMPTY_CUSTOM);
    setCustomError(null);
    void runCustom(entry, ipc.installCustomCli);
  };

  const removeCustom = async (item: CustomCliSetting) => {
    setCustomBusy(item.source);
    try {
      await ipc.uninstallCustomCli(item);
    } catch (cause) {
      setCustomError(cause instanceof Error ? cause.message : String(cause));
      setCustomBusy(null);
      return;
    }
    setCustomBusy(null);
    onChange({
      custom_clis: settings.custom_clis.filter((entry) => entry.id !== item.id),
    });
    setCustomStatuses((previous) => {
      const next = { ...previous };
      delete next[item.source];
      return next;
    });
  };

  return (
    <div>
      <div className="mb-3 flex items-start gap-2 px-1">
        <p className="min-w-0 flex-1 text-[12px] text-white/45">
          {t("settings.hooks.description")}
        </p>
        <PixelButton
          variant="ghost"
          disabled={loading}
          onClick={() => void load()}
        >
          {t("settings.hooks.refresh")}
        </PixelButton>
      </div>

      {loading && (
        <p className="text-[12px] text-white/40">{t("common.loading")}</p>
      )}
      {!loading && statuses.length === 0 && (
        <p className="text-[12px] text-white/40">{t("settings.hooks.empty")}</p>
      )}

      {statuses.map((status) => {
        const installed = status.hook_installed;
        const healthy = status.healthy;
        return (
          <div
            key={status.agent}
            className="mb-2 rounded-[10px] bg-white/[0.055] px-3.5 py-2.5"
          >
            <div className="flex items-center gap-1.5">
              <AgentBadge agent={status.agent} />
              <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-white/90">
                {status.display_name || t(agentLabelKey(status.agent))}
              </span>
              <span
                className={`flex-none border px-1 py-0.5 text-[10px] ${
                  installed
                    ? "border-[#4dd966]/60 text-[#6ee7a8]"
                    : "border-white/10 text-white/40"
                }`}
              >
                {installed
                  ? t("settings.hooks.installed")
                  : t("settings.hooks.notInstalled")}
              </span>
              <Toggle
                id={`hook-${status.agent}`}
                label={status.display_name}
                checked={installed}
                onChange={(enabled) =>
                  void run(
                    status.agent,
                    enabled ? ipc.installHooks : ipc.uninstallHooks,
                  )
                }
              />
              {installed && (
                <span
                  className={`flex-none border px-1 py-0.5 text-[9px] ${
                    healthy
                      ? "border-success/60 text-success"
                      : "border-danger/60 text-danger"
                  }`}
                >
                  {healthy
                    ? t("settings.hooks.healthy")
                    : t("settings.hooks.unhealthy")}
                </span>
              )}
            </div>

            {status.problem && (
              <p className="mt-1 text-[11px] text-[#ff7b7b]">
                {t("settings.hooks.problem")}: {status.problem}
              </p>
            )}
            {status.config_path && (
              <p
                className="mt-0.5 truncate text-[11px] text-white/40"
                title={status.config_path}
              >
                {t("settings.hooks.configPath")}: {status.config_path}
              </p>
            )}

            <div className="mt-1.5 flex flex-wrap gap-1.5">
              {!installed && (
                <PixelButton
                  disabled={busy === status.agent}
                  onClick={() => void run(status.agent, ipc.installHooks)}
                >
                  {t("settings.hooks.install")}
                </PixelButton>
              )}
              {installed && !healthy && (
                <PixelButton
                  disabled={busy === status.agent}
                  onClick={() => void run(status.agent, ipc.repairHooks)}
                >
                  {t("settings.hooks.repair")}
                </PixelButton>
              )}
              {installed && (
                <PixelButton
                  variant="danger"
                  disabled={busy === status.agent}
                  onClick={() => void run(status.agent, ipc.uninstallHooks)}
                >
                  {t("settings.hooks.uninstall")}
                </PixelButton>
              )}
            </div>
          </div>
        );
      })}

      <SettingsSection>
        <SettingRow
          htmlFor={coworkId}
          label={t("settings.hooks.coworkTracking")}
          description={t("settings.hooks.coworkTrackingHint")}
        >
          <Toggle
            id={coworkId}
            label={t("settings.hooks.coworkTracking")}
            checked={settings.track_claude_desktop_cowork}
            onChange={(track_claude_desktop_cowork) => onChange({ track_claude_desktop_cowork })}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection title={t("settings.hooks.extraDirs")}>
        {settings.extra_config_dirs.map((dir) => (
          <SettingRow key={dir} htmlFor={`extra-${dir}`} label={dir}>
            <PixelButton
              variant="ghost"
              onClick={() =>
                onChange({
                  extra_config_dirs: settings.extra_config_dirs.filter(
                    (entry) => entry !== dir,
                  ),
                })
              }
            >
              {t("common.remove")}
            </PixelButton>
          </SettingRow>
        ))}
        <div className="flex items-center gap-2 px-3.5 py-2">
          <TextField
            id={extraId}
            value={extraDraft}
            placeholder="C:\extra\.claude"
            onChange={setExtraDraft}
            className="flex-1"
          />
          <PixelButton onClick={addExtraDir}>{t("common.add")}</PixelButton>
        </div>
      </SettingsSection>

      <SettingsSection title={t("settings.hooks.customTitle")}>
        {settings.custom_clis.length === 0 && (
          <div className="px-3.5 py-4 text-[12px] text-white/45">
            {t("settings.hooks.customEmpty")}
          </div>
        )}
        {settings.custom_clis.map((item) => {
          const status = customStatuses[item.source];
          const installed = status?.hook_installed ?? false;
          return (
            <div
              key={item.id}
              className="border-b border-white/[0.06] px-3.5 py-2.5 last:border-b-0"
            >
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px] font-semibold text-white/90">
                    {item.name}
                  </div>
                  <div className="truncate font-mono text-[11px] text-white/40">
                    {item.source} · {item.config_path}
                  </div>
                </div>
                <span
                  className={`flex-none border px-1 py-0.5 text-[10px] ${
                    installed
                      ? "border-[#4dd966]/60 text-[#6ee7a8]"
                      : "border-white/10 text-white/40"
                  }`}
                >
                  {installed
                    ? t("settings.hooks.installed")
                    : t("settings.hooks.notInstalled")}
                </span>
                <PixelButton
                  disabled={customBusy === item.source}
                  onClick={() =>
                    void runCustom(
                      item,
                      installed ? ipc.uninstallCustomCli : ipc.installCustomCli,
                    )
                  }
                >
                  {installed
                    ? t("settings.hooks.uninstall")
                    : t("settings.hooks.install")}
                </PixelButton>
                <PixelButton
                  variant="danger"
                  onClick={() => void removeCustom(item)}
                >
                  {t("common.remove")}
                </PixelButton>
              </div>
              {installed && status?.problem && (
                <p className="mt-1 text-[11px] text-[#ff7b7b]">
                  {t("settings.hooks.problem")}: {status.problem}
                </p>
              )}
            </div>
          );
        })}
        <div className="grid gap-2 px-3.5 py-3 sm:grid-cols-2">
          <TextField
            id={customNameId}
            value={draft.name}
            placeholder={t("settings.hooks.customName")}
            onChange={(name) => setDraft((value) => ({ ...value, name }))}
          />
          <TextField
            id={customSourceId}
            value={draft.source}
            placeholder={t("settings.hooks.customSource")}
            onChange={(source) => setDraft((value) => ({ ...value, source }))}
          />
          <TextField
            id={customPathId}
            value={draft.config_path}
            placeholder={t("settings.hooks.customConfigPath")}
            onChange={(config_path) =>
              setDraft((value) => ({ ...value, config_path }))
            }
            className="sm:col-span-2"
          />
          <TextField
            id={customKeyId}
            value={draft.config_key}
            placeholder={t("settings.hooks.customConfigKey")}
            onChange={(config_key) =>
              setDraft((value) => ({ ...value, config_key }))
            }
          />
          <SelectField
            id={customTemplateId}
            value={draft.format}
            options={[
              { value: "claude_json", label: t("settings.hooks.templateClaude") },
              {
                value: "event_map_json",
                label: t("settings.hooks.templateEventMap"),
              },
              { value: "cursor_json", label: t("settings.hooks.templateCursor") },
              {
                value: "copilot_json",
                label: t("settings.hooks.templateCopilot"),
              },
            ]}
            onChange={(format) => setDraft((value) => ({ ...value, format }))}
          />
        </div>
        <div className="flex items-center gap-2 px-3.5 pb-3">
          <PixelButton onClick={addCustom}>
            {t("settings.hooks.customAdd")}
          </PixelButton>
          {customError && (
            <span className="text-[11px] text-[#ff7b7b]">{customError}</span>
          )}
        </div>
      </SettingsSection>

      {error && <p className="text-[12px] text-[#ff7b7b]">{error}</p>}
    </div>
  );
}
