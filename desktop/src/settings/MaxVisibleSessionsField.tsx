import { useId } from "react";
import { useTranslation } from "react-i18next";
import { UNLIMITED_VISIBLE_SESSIONS } from "../lib/defaults";
import { SelectField, SettingRow, type SettingsTabProps } from "./controls";

const PRESETS = [3, 5, 8, 10, UNLIMITED_VISIBLE_SESSIONS];

export function MaxVisibleSessionsField({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const id = useId();
  const current = settings.max_visible_sessions;
  const choices = PRESETS.includes(current) ? PRESETS : [...PRESETS, current].sort((a, b) => a - b);
  return (
    <SettingRow
      htmlFor={id}
      label={t("settings.behavior.maxVisible")}
      description={t(settings.density === "compact"
        ? "settings.appearance.maxVisibleCompactHint"
        : "settings.appearance.maxVisibleHint")}
    >
      <SelectField
        id={id}
        value={String(current)}
        onChange={(value) => onChange({ max_visible_sessions: Number(value) })}
        options={choices.map((limit) => ({
          value: String(limit),
          label: limit === UNLIMITED_VISIBLE_SESSIONS ? t("settings.appearance.maxVisibleUnlimited") : String(limit),
        }))}
      />
    </SettingRow>
  );
}
