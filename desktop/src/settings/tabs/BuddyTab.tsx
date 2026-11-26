import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { PixelButton } from "../../components/PixelButton";
import * as ipc from "../../lib/ipc";
import { SettingRow, SettingsSection, SliderField, Toggle, type SettingsTabProps } from "../controls";

export function BuddyTab(props: SettingsTabProps) {
  const { t } = useTranslation();
  const [scanning, setScanning] = useState(false);
  const [status, setStatus] = useState("not_connected");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void ipc
      .buddyStatus()
      .then(setStatus)
      .catch((cause) => setError(String(cause)));
  }, []);

  const scan = async () => {
    setScanning(true);
    setError(null);
    try {
      setStatus(await ipc.buddyConnect());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setStatus("not_connected");
    } finally {
      setScanning(false);
    }
  };

  const disconnect = async () => {
    try {
      await ipc.buddyDisconnect();
      setStatus("not_connected");
    } catch (cause) {
      setError(String(cause));
    }
  };

  const label =
    status === "connected"
      ? t("settings.buddy.connected")
      : status === "unsupported"
        ? t("settings.buddy.unsupported")
        : t("settings.buddy.notConnected");

  return (
    <div>
      <SettingsSection title={t("settings.buddy.title")}>
        <SettingRow
          htmlFor="buddy-status"
          label={t("settings.buddy.status")}
          description={t("settings.buddy.hint")}
        >
          <div className="flex items-center gap-2">
            <span className="text-[12px] text-white/55">
              {scanning ? t("settings.buddy.scanning") : label}
            </span>
            {status === "connected" ? (
              <PixelButton variant="ghost" onClick={() => void disconnect()}>
                {t("settings.buddy.disconnect")}
              </PixelButton>
            ) : (
              <PixelButton
                variant="primary"
                disabled={scanning}
                onClick={() => void scan()}
              >
                {t("settings.buddy.scan")}
              </PixelButton>
            )}
          </div>
        </SettingRow>
        {error && (
          <div className="px-3.5 pb-2 text-[11px] text-[#ff7b7b]">{error}</div>
        )}
      </SettingsSection>

      <SettingsSection title={t("settings.buddy.pairing")}>
        <div className="px-3.5 py-4 text-[12px] leading-relaxed text-white/45">
          {t("settings.buddy.pairHint")}
        </div>
      </SettingsSection>
      <AppleCompanionSection {...props} />
    </div>
  );
}

function AppleCompanionSection({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const enabledId = useId();
  const intervalId = useId();
  const statusId = useId();
  const [status, setStatus] = useState<ipc.AppleCompanionStatus | null>(null);
  const [restarting, setRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const enabled = settings.apple_companion_enabled;
  const heartbeat = settings.apple_companion_heartbeat_seconds;

  useEffect(() => {
    if (!enabled) {
      setStatus(null);
      setError(null);
      return;
    }
    let stopped = false;
    let timer: number | undefined;
    const check = async () => {
      try {
        const latest = await ipc.appleCompanionStatus();
        if (!stopped) {
          setStatus(latest);
          setError(null);
        }
      } catch (cause) {
        if (!stopped) setError(String(cause));
      }
      if (!stopped) timer = window.setTimeout(() => void check(), 1000);
    };
    void check();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [enabled]);

  const restart = async () => {
    setRestarting(true);
    setError(null);
    try {
      await ipc.appleCompanionRestart();
      setStatus(await ipc.appleCompanionStatus());
    } catch (cause) {
      setError(String(cause));
    } finally {
      setRestarting(false);
    }
  };

  const problem = error ?? status?.error;
  const clients = enabled ? (status?.subscribed_clients ?? 0) : 0;
  let statusKey = "off";
  let color = "bg-white/35";
  if (enabled) {
    if (problem || status?.powered_on === false) {
      statusKey = "unavailable";
      color = "bg-[#ff453a]";
    } else if (clients > 0) {
      statusKey = "connected";
      color = "bg-[#4dd966]";
    } else {
      statusKey = status?.advertising ? "waiting" : "starting";
      color = "bg-[#ff9f0a]";
    }
  }
  const seconds = t("settings.buddy.appleCompanion.seconds", { count: heartbeat });

  return (
    <SettingsSection title={t("settings.buddy.appleCompanion.title")}>
      <SettingRow htmlFor={enabledId} label={t("settings.buddy.appleCompanion.enabled")}>
        <Toggle
          id={enabledId}
          label={t("settings.buddy.appleCompanion.enabled")}
          checked={enabled}
          onChange={(apple_companion_enabled) => onChange({ apple_companion_enabled })}
        />
      </SettingRow>
      <SettingRow htmlFor={statusId} label={t("settings.buddy.appleCompanion.status")}>
        <output id={statusId} className="flex items-center gap-2 text-[12px] text-white/60">
          <span className={`h-2 w-2 rounded-full ${color}`} aria-hidden="true" />
          {t(statusKey === "starting" ? "common.loading" : `settings.buddy.appleCompanion.${statusKey}`)}
          {clients > 0 && <span>({clients})</span>}
        </output>
      </SettingRow>
      {clients === 0 && (
        <p className="px-3.5 pt-2 text-[11px] text-white/45">
          {t("settings.buddy.appleCompanion.noDevices")}
        </p>
      )}
      <SettingRow htmlFor={intervalId} label={t("settings.buddy.appleCompanion.interval")}>
        <SliderField
          id={intervalId}
          value={heartbeat}
          min={1}
          max={30}
          step={1}
          valueText={seconds}
          disabled={!enabled}
          onChange={(apple_companion_heartbeat_seconds) => onChange({ apple_companion_heartbeat_seconds })}
        />
        <span className="w-10 text-right text-[12px] tabular-nums text-white/55">{seconds}</span>
      </SettingRow>
      <div className="space-y-2 px-3.5 py-3 text-[11px] leading-relaxed text-white/55">
        <PixelButton disabled={!enabled || restarting} onClick={() => void restart()}>
          {t("settings.buddy.appleCompanion.restart")}
        </PixelButton>
        {enabled && problem && <p role="alert" className="text-[#ff7b7b]">{problem}</p>}
        <p>{t("settings.buddy.appleCompanion.transport")}</p>
        <p>{t("settings.buddy.appleCompanion.readOnly")}</p>
      </div>
    </SettingsSection>
  );
}
