import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { testPushChannel } from "../../lib/ipc";
import type { PushChannelSetting } from "../../lib/types";
import { SelectField, SettingRow, SettingsSection, TextField, Toggle, type SettingsTabProps } from "../controls";

const SERVICES = ["bark", "ntfy", "dingtalk", "feishu", "wecom", "slack", "telegram"];
const EVENTS = ["permission", "question", "completion", "error", "reminder"];
const PERSONAL = new Set(["bark", "ntfy", "telegram"]);
const DEFAULT_ENDPOINTS: Record<string, string> = {
  bark: "https://api.day.app",
  ntfy: "https://ntfy.sh",
  telegram: "https://api.telegram.org",
};

type TextKey = "endpoint" | "target" | "token" | "secret" | "group" | "icon" | "sound";
type Field = { key: TextKey; label: string; masked?: boolean; optional?: boolean };
const WEBHOOK_FIELD: Field = { key: "endpoint", label: "webhook", masked: true };
const SECRET_FIELD: Field = { key: "secret", label: "secret", masked: true, optional: true };
const FIELDS: Record<string, Field[]> = {
  bark: [
    { key: "endpoint", label: "server" },
    { key: "target", label: "device_key", masked: true },
    { key: "group", label: "group", optional: true },
    { key: "icon", label: "icon", optional: true },
    { key: "sound", label: "sound", optional: true },
  ],
  ntfy: [
    { key: "endpoint", label: "server" },
    { key: "target", label: "topic", masked: true },
    { key: "token", label: "token", masked: true, optional: true },
  ],
  dingtalk: [WEBHOOK_FIELD, SECRET_FIELD],
  feishu: [WEBHOOK_FIELD, SECRET_FIELD],
  telegram: [
    { key: "token", label: "bot_token", masked: true },
    { key: "target", label: "chat_id" },
    { key: "endpoint", label: "api_base" },
  ],
};

function channelDefaults(kind: string): PushChannelSetting {
  return {
    kind, enabled: false, events: [...EVENTS], endpoint: "", target: "", token: "",
    secret: "", group: "", icon: "", sound: "", priority: 4, include_details: PERSONAL.has(kind),
  };
}

function channelsFor(settings: SettingsTabProps["settings"]): PushChannelSetting[] {
  if (settings.push_channels.length || !settings.push_target.trim()) return settings.push_channels;
  const kind = settings.push_channel === "lark" ? "feishu" : settings.push_channel;
  const channel = { ...channelDefaults(kind), enabled: true, include_details: true };
  if (kind === "telegram") {
    const slash = settings.push_target.lastIndexOf("/");
    channel.token = settings.push_target.slice(0, slash);
    channel.target = settings.push_target.slice(slash + 1);
  } else if (!PERSONAL.has(kind) || /^https?:\/\//i.test(settings.push_target)) {
    channel.endpoint = settings.push_target;
  } else {
    channel.target = settings.push_target;
  }
  return [channel];
}

function hasRequiredFields(channel: PushChannelSetting) {
  let url: URL;
  try {
    const endpoint = channel.endpoint.trim() || DEFAULT_ENDPOINTS[channel.kind];
    if (!endpoint) return false;
    url = new URL(endpoint.includes("://") ? endpoint : `https://${endpoint}`);
  } catch {
    return false;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return false;
  const last = url.pathname.split("/").filter(Boolean).pop();
  if (channel.kind === "bark") return Boolean(channel.target.trim() || (last && last !== "push"));
  if (channel.kind === "ntfy") return Boolean((channel.target.trim() || last) &&
    (!channel.target.trim() || !last || channel.target.trim() === last));
  if (channel.kind === "telegram") return Boolean(channel.token.trim() && channel.target.trim());
  return true;
}

function ChannelField({ field, channel, change }: {
  field: Field;
  channel: PushChannelSetting;
  change: (patch: Partial<PushChannelSetting>) => void;
}) {
  const { t } = useTranslation();
  const id = useId();
  const [revealed, setRevealed] = useState(false);
  return (
    <SettingRow htmlFor={id} label={t(`push.push_field_${field.label}`)}>
      <TextField
        id={id}
        value={channel[field.key]}
        type={field.masked && !revealed ? "password" : "text"}
        placeholder={field.optional ? t("push.push_optional") : field.key === "endpoint"
          ? DEFAULT_ENDPOINTS[channel.kind] : undefined}
        onChange={(value) => change({ [field.key]: value })}
        className="w-44 max-w-full"
      />
      {field.masked && (
        <button
          type="button"
          className="rounded border border-white/10 px-2 py-1 text-[11px]"
          onClick={() => setRevealed(!revealed)}
          aria-controls={id}
          aria-pressed={revealed}
        >{t(revealed ? "push.push_hide_value" : "push.push_show_value")}</button>
      )}
    </SettingRow>
  );
}

function PushChannelEditor({ channel, update }: {
  channel: PushChannelSetting;
  update: (channel: PushChannelSetting) => void;
}) {
  const { t } = useTranslation();
  const toggleId = useId();
  const detailsId = useId();
  const priorityId = useId();
  const name = t(`push.push_channel_${channel.kind}`, { defaultValue: channel.kind });
  const [sending, setSending] = useState(false);
  const [result, setResult] = useState<string>();
  const version = useRef(0);
  const change = (patch: Partial<PushChannelSetting>) => {
    version.current += 1;
    setResult(undefined);
    update({ ...channel, ...patch });
  };
  const send = async () => {
    const requestVersion = version.current;
    setSending(true);
    setResult(undefined);
    try {
      await testPushChannel(channel, t("push.push_msg_test_headline"), t("push.push_msg_test_body"));
      if (version.current === requestVersion) setResult(t("push.push_test_ok"));
    } catch (error) {
      if (version.current === requestVersion) setResult(`${t("push.push_test_failed")} · ${String(error)}`);
    } finally {
      setSending(false);
    }
  };
  return (
    <div className="border-b border-white/[0.06] last:border-b-0">
      <div className="flex items-start justify-between gap-3 px-3.5 py-2">
        <details className="min-w-0 flex-1">
          <summary className="cursor-pointer py-1 text-[13px]">{name}</summary>
          <div className="mt-1">
            {(FIELDS[channel.kind] ?? [WEBHOOK_FIELD]).map((field) => (
              <ChannelField key={field.key} field={field} channel={channel} change={change} />
            ))}
            {channel.kind === "ntfy" && (
              <SettingRow htmlFor={priorityId} label={t("push.push_field_priority")}>
                <SelectField id={priorityId} value={String(channel.priority)}
                  options={[3, 4, 5].map((value) => ({ value: String(value), label: String(value) }))}
                  onChange={(value) => change({ priority: Number(value) })} />
              </SettingRow>
            )}
            {SERVICES.includes(channel.kind) && (
              <p className="px-3.5 py-2 text-[11px] text-white/50">{t(`push.push_hint_${channel.kind}`)}</p>
            )}
            <fieldset className="px-3.5 py-2">
              <legend className="text-[12px] text-white/70">{t("push.push_events")}</legend>
              <div className="flex flex-wrap gap-x-3 gap-y-1">
                {EVENTS.map((event) => (
                  <label key={event} className="inline-flex items-center gap-1.5 text-[12px]"
                    title={event === "reminder" ? t("push.push_event_reminder_help") : undefined}>
                    <input type="checkbox" checked={channel.events.includes(event)} onChange={({ target }) => change({
                      events: target.checked ? [...channel.events, event] : channel.events.filter((item) => item !== event),
                    })} />
                    {t(`push.push_event_${event}`)}
                  </label>
                ))}
              </div>
            </fieldset>
            <SettingRow htmlFor={detailsId} label={t("push.push_include_details")}
              description={t("push.push_include_details_desc")}>
              <Toggle id={detailsId} label={t("push.push_include_details")} checked={channel.include_details}
                onChange={(include_details) => change({ include_details })} />
            </SettingRow>
            <div className="flex flex-wrap items-center gap-2 px-3.5 py-2 text-[12px]">
              <button type="button" disabled={sending || !hasRequiredFields(channel)}
                onClick={() => void send()} className="rounded border border-white/15 bg-white/[0.08] px-2 py-1 disabled:opacity-40">
                {t(sending ? "push.push_sending" : "push.push_send_test")}
              </button>
              <span role="status">{result}</span>
            </div>
          </div>
        </details>
        <Toggle id={toggleId} label={name} checked={channel.enabled} onChange={(enabled) => change({ enabled })} />
      </div>
    </div>
  );
}

export function PushSettings({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const enabledId = useId();
  const awayId = useId();
  const minutesId = useId();
  const summaryId = useId();
  const channels = channelsFor(settings);
  const services = [...new Set([...SERVICES, ...channels.map(({ kind }) => kind)])];
  const update = (channel: PushChannelSetting) => onChange({
    push_channels: [...channels.filter(({ kind }) => kind !== channel.kind), channel],
  });
  return (
    <SettingsSection title={t("push.push_title")}>
      <SettingRow htmlFor={enabledId} label={t("push.push_enable")} description={t("push.push_desc")}>
        <Toggle id={enabledId} label={t("push.push_enable")} checked={settings.push_enabled}
          onChange={(push_enabled) => onChange({ push_enabled })} />
      </SettingRow>
      {settings.push_enabled && (
        <>
          <SettingRow htmlFor={awayId} label={t("push.push_only_when_away")} description={t("push.push_only_when_away_desc")}>
            <Toggle id={awayId} label={t("push.push_only_when_away")} checked={settings.push_only_when_away}
              onChange={(push_only_when_away) => onChange({ push_only_when_away })} />
          </SettingRow>
          <SettingRow htmlFor={minutesId} label={t("push.push_idle_minutes")}>
            <SelectField id={minutesId} disabled={!settings.push_only_when_away}
              value={String(settings.push_away_idle_minutes)}
              options={[1, 2, 5, 10, 15, 30].map((value) => ({ value: String(value), label: t("push.push_minutes_format", { count: value }) }))}
              onChange={(value) => onChange({ push_away_idle_minutes: Number(value) })} />
          </SettingRow>
          <SettingRow htmlFor={summaryId} label={t("push.push_summary_length")}>
            <SelectField id={summaryId} value={String(settings.push_summary_length)}
              options={[100, 200, 500, 1000].map((value) => ({ value: String(value), label: t("push.push_chars_format", { count: value }) }))}
              onChange={(value) => onChange({ push_summary_length: Number(value) })} />
          </SettingRow>
          {services.map((kind) => (
            <PushChannelEditor key={kind} channel={channels.find((channel) => channel.kind === kind) ?? channelDefaults(kind)}
              update={update} />
          ))}
          <p className="px-3.5 py-2 text-[11px] text-white/50">{t("push.push_storage_note")}</p>
        </>
      )}
    </SettingsSection>
  );
}
