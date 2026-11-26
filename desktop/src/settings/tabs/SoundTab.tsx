import { useEffect, useId, useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { PlayCircleIcon, SpeakerIcon, WaveformIcon } from "../../components/icons";
import { CUE_LABEL_KEYS, SOUND_VOLUME_MAX, SOUND_VOLUME_MIN, type AudibleCue } from "../../lib/defaults";
import { pickCustomSound, resetCustomSound } from "../../lib/ipc";
import { previewSound, soundFileName } from "../../lib/sound";
import { useIslandStore } from "../../lib/store";
import { SettingRow, SettingsSection, SliderField, TimeField, Toggle, type SettingsTabProps } from "../controls";

const GROUPS: ReadonlyArray<{ section: string; cues: AudibleCue[] }> = [
  { section: "sessions", cues: ["SessionStart", "Stop", "StopFailure"] },
  { section: "interaction", cues: ["PermissionRequest", "UserPromptSubmit"] },
  { section: "system", cues: ["AppLaunch"] },
];

function SoundEventRow({
  cue,
  checked,
  customPath,
  volume,
  onToggle,
}: {
  cue: AudibleCue;
  checked: boolean;
  customPath?: string;
  volume: number;
  onToggle: (checked: boolean) => void;
}) {
  const { t } = useTranslation();
  const id = useId();
  const menuId = useId();
  const menu = useRef<HTMLDivElement>(null);
  const [localPath, setLocalPath] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const label = t(CUE_LABEL_KEYS[cue]);
  const fileName = soundFileName(cue);
  const path = localPath ?? customPath;
  const name = path?.split(/[\\/]/).pop();
  const chooseLabel = t("settings.sound.chooseFile");
  const previewLabel = `${t("settings.sound.preview")} — ${label}`;

  useEffect(() => setLocalPath(undefined), [customPath]);

  const positionMenu = (event: MouseEvent<HTMLButtonElement>) => {
    if (!menu.current) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const openDown = window.innerHeight - rect.bottom >= 92;
    menu.current.style.right = `${Math.max(8, window.innerWidth - rect.right)}px`;
    menu.current.style.top = openDown ? `${rect.bottom + 4}px` : "auto";
    menu.current.style.bottom = openDown ? "auto" : `${window.innerHeight - rect.top + 4}px`;
  };

  const changeSound = async (reset: boolean) => {
    menu.current?.hidePopover?.();
    setBusy(true);
    setFailed(false);
    try {
      if (reset) {
        await resetCustomSound(fileName);
        setLocalPath("");
      } else {
        const selected = await pickCustomSound(fileName, chooseLabel);
        if (selected) setLocalPath(selected);
      }
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  };

  const preview = async () => {
    setFailed(false);
    if (!await previewSound(cue, volume, path)) setFailed(true);
  };

  return (
    <SettingRow
      htmlFor={id}
      label={label}
      description={
        <>
          {name ? <span className="sound-custom-name">{t("settings.sound.customFile", { name })}</span> : t(`${CUE_LABEL_KEYS[cue]}Hint`)}
          {failed && <span className="sound-preview-error" role="status">{t("settings.sound.previewFailed")}</span>}
        </>
      }
    >
      <button
        className="sound-option-button"
        data-custom={Boolean(path)}
        type="button"
        popoverTarget={menuId}
        title={chooseLabel}
        aria-label={`${chooseLabel} — ${label}`}
        disabled={busy}
        onClick={positionMenu}
      >
        <WaveformIcon custom={Boolean(path)} width={14} height={14} />
      </button>
      <div ref={menu} id={menuId} popover="auto" className="sound-file-menu">
        <button type="button" onClick={() => void changeSound(false)}>{chooseLabel}</button>
        {path && <button type="button" onClick={() => void changeSound(true)}>{t("settings.sound.resetFile")}</button>}
      </div>
      <button className="sound-option-button" type="button" title={previewLabel} aria-label={previewLabel} onClick={() => void preview()}>
        <PlayCircleIcon width={22} height={22} />
      </button>
      <Toggle id={id} label={label} checked={checked} onChange={onToggle} />
    </SettingRow>
  );
}

export function SoundTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const configuredSounds = useIslandStore((state) => state.ui?.settings.custom_sound_paths) ?? settings.custom_sound_paths;
  const enabledId = useId();
  const volumeId = useId();
  const quietId = useId();
  const quietStartId = useId();
  const quietEndId = useId();
  const muteId = useId();

  const toggleCue = (cue: AudibleCue, enabled: boolean) => {
    const cues = enabled
      ? Array.from(new Set([...settings.audible_cues, cue]))
      : settings.audible_cues.filter((item) => item !== cue && !(cue === "Stop" && item === "TaskRoundComplete"));
    onChange({ audible_cues: cues });
  };

  return (
    <>
      <SettingsSection>
        <SettingRow htmlFor={enabledId} label={t("settings.sound.enabled")}>
          <Toggle
            id={enabledId}
            label={t("settings.sound.enabled")}
            checked={settings.sound_enabled}
            onChange={(sound_enabled) => onChange({ sound_enabled })}
          />
        </SettingRow>
        {settings.sound_enabled && (
          <SettingRow htmlFor={volumeId} label={t("settings.sound.volume")}>
            <div className="flex items-center gap-2 text-white/55">
              <SpeakerIcon muted />
              <SliderField
                id={volumeId}
                value={settings.sound_volume}
                min={SOUND_VOLUME_MIN}
                max={SOUND_VOLUME_MAX}
                step={1}
                onChange={(sound_volume) => onChange({ sound_volume })}
              />
              <SpeakerIcon />
              <span className="w-9 text-right text-[12px] tabular-nums">
                {t("settings.sound.volumeValue", { volume: settings.sound_volume })}
              </span>
            </div>
          </SettingRow>
        )}
      </SettingsSection>

      {settings.sound_enabled && (
        <>
          {GROUPS.map(({ section, cues }) => (
            <SettingsSection key={section} title={t(`settings.sound.${section}`)}>
              {cues.map((cue) => (
                <SoundEventRow
                  key={cue}
                  cue={cue}
                  checked={settings.audible_cues.includes(cue) ||
                    (cue === "Stop" && settings.audible_cues.includes("TaskRoundComplete"))}
                  customPath={configuredSounds[soundFileName(cue)]}
                  volume={settings.sound_volume}
                  onToggle={(enabled) => toggleCue(cue, enabled)}
                />
              ))}
            </SettingsSection>
          ))}
          <SettingsSection>
            <SettingRow
              htmlFor={quietId}
              label={t("settings.sound.quietHours")}
              description={t("settings.sound.quietHoursHint")}
            >
              <Toggle
                id={quietId}
                label={t("settings.sound.quietHours")}
                checked={settings.quiet_hours_enabled}
                onChange={(quiet_hours_enabled) => onChange({ quiet_hours_enabled })}
              />
            </SettingRow>
            {settings.quiet_hours_enabled && (
              <div className="sound-quiet-times">
                <label htmlFor={quietStartId}>{t("settings.sound.quietStart")}</label>
                <TimeField
                  id={quietStartId}
                  value={settings.quiet_hours_start}
                  onChange={(quiet_hours_start) => onChange({ quiet_hours_start })}
                />
                <label htmlFor={quietEndId}>{t("settings.sound.quietEnd")}</label>
                <TimeField
                  id={quietEndId}
                  value={settings.quiet_hours_end}
                  onChange={(quiet_hours_end) => onChange({ quiet_hours_end })}
                />
              </div>
            )}
            <SettingRow
              htmlFor={muteId}
              label={t("settings.sound.muteWhenAway")}
              description={t("settings.sound.muteWhenAwayHint")}
            >
              <Toggle
                id={muteId}
                label={t("settings.sound.muteWhenAway")}
                checked={settings.mute_when_away}
                onChange={(mute_when_away) => onChange({ mute_when_away })}
              />
            </SettingRow>
          </SettingsSection>
        </>
      )}
    </>
  );
}
