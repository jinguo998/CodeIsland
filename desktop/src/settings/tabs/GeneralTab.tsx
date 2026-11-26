import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { listDisplays, type DisplayInfo } from "../../lib/ipc";
import { LANGUAGE_OPTIONS } from "../../lib/settings";
import {
  SelectField,
  SettingRow,
  SettingsSection,
  Toggle,
  type SettingsTabProps,
} from "../controls";

const LANGUAGE_LABEL_KEYS: Record<(typeof LANGUAGE_OPTIONS)[number], string> = {
  system: "settings.general.languageSystem",
  "en-US": "settings.general.languageEnglish",
  "zh-CN": "settings.general.languageChinese",
  "zh-TW": "settings.general.languageTraditional",
  "de-DE": "settings.general.languageGerman",
  "ja-JP": "settings.general.languageJapanese",
  "ko-KR": "settings.general.languageKorean",
  "tr-TR": "settings.general.languageTurkish",
};

function pinnedOption(value: string, monitors: DisplayInfo[]): string {
  const id = value.replace(/^(?:display|monitor|screen)[:_]/, "");
  const key = id.toLowerCase();
  const indexed = /^\d+$/.test(id) ? monitors[Number(id)] : undefined;
  const identified = monitors.find((monitor) => monitor.id.toLowerCase() === key || monitor.legacyId?.toLowerCase() === key);
  const named = monitors.filter((monitor) => monitor.name.toLowerCase() === key);
  const match = indexed ?? identified ?? (named.length === 1 ? named[0] : undefined);
  return `display:${match?.id ?? id}`;
}

/**
 * Use the same native monitor identity as the window placement code. Legacy
 * numeric and device-name pins still select the connected monitor's row.
 */
export function GeneralTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const languageId = useId();
  const startupId = useId();
  const horizontalDragId = useId();
  const displayId = useId();

  const [monitors, setMonitors] = useState<DisplayInfo[]>([]);
  const [monitorsReady, setMonitorsReady] = useState(false);

  useEffect(() => {
    let revision = 0;
    const refresh = async () => {
      const requested = ++revision;
      const list = await listDisplays().catch(() => []);
      if (requested === revision) {
        setMonitors(list);
        setMonitorsReady(true);
      }
    };
    void refresh();
    window.addEventListener("focus", refresh);
    return () => {
      revision += 1;
      window.removeEventListener("focus", refresh);
    };
  }, []);

  const monitorOptions = monitors.map((monitor, index) => {
    const name =
      monitor.name ||
      t("settings.general.displayMonitorFallback", { index: index + 1 });
    return {
      value: `display:${monitor.id}`,
      label: t("settings.general.displayMonitor", {
        name,
        width: monitor.width,
        height: monitor.height,
      }),
    };
  });

  const pinnedValue = settings.pinned_display
    ? pinnedOption(settings.pinned_display, monitors)
    : null;
  const displayValue = (() => {
    const choice = settings.display_choice;
    if (choice === "cursor") return "cursor";
    if (choice === "pinned") return pinnedValue ?? "pinned";
    if (choice && choice !== "auto" && choice !== "primary") return pinnedOption(choice, monitors);
    // Preserve a legacy cursor/pinned choice while display_choice is still auto.
    if (settings.display_mode === "cursor") return "cursor";
    if (settings.display_mode === "pinned" && pinnedValue) {
      return pinnedValue;
    }
    if (settings.display_mode === "pinned") return "pinned";
    return "auto";
  })();

  const displayOptions = [
    { value: "auto", label: t("settings.general.displayAuto") },
    { value: "cursor", label: t("settings.general.displayCursor") },
    ...monitorOptions,
  ];
  if (monitorsReady && monitors.length === 0) {
    displayOptions.push({
      value: "pinned",
      label: t("settings.general.displayPinned"),
    });
  }
  if (monitorsReady && !displayOptions.some((option) => option.value === displayValue)) {
    displayOptions.push({
      value: displayValue,
      label: t("settings.general.displayMissing"),
    });
  }

  const changeDisplay = (value: string) => {
    if (value === "auto") {
      onChange({
        display_choice: "auto",
        display_mode: "primary",
        pinned_display: null,
      });
    } else if (value === "cursor") {
      onChange({
        display_choice: "cursor",
        display_mode: "cursor",
        pinned_display: null,
      });
    } else if (value === "pinned") {
      onChange({ display_choice: "pinned", display_mode: "pinned", pinned_display: null });
    } else {
      onChange({
        display_choice: value,
        display_mode: "pinned",
        pinned_display: value.slice("display:".length),
      });
    }
  };

  const displayHint =
    monitorsReady && monitors.length === 0
      ? t("settings.general.displayChoiceUnavailable")
      : undefined;

  return (
    <SettingsSection>
      <SettingRow htmlFor={languageId} label={t("settings.general.language")}>
        <SelectField
          id={languageId}
          value={settings.language}
          onChange={(language) => onChange({ language })}
          options={LANGUAGE_OPTIONS.map((language) => ({
            value: language,
            label: t(LANGUAGE_LABEL_KEYS[language]),
          }))}
        />
      </SettingRow>

      <SettingRow htmlFor={startupId} label={t("settings.general.startup")}>
        <Toggle
          id={startupId}
          label={t("settings.general.startup")}
          checked={settings.startup}
          onChange={(startup) => onChange({ startup })}
        />
      </SettingRow>

      <SettingRow
        htmlFor={horizontalDragId}
        label={t("settings.general.allowHorizontalDrag")}
        description={t("settings.general.allowHorizontalDragHint")}
      >
        <Toggle
          id={horizontalDragId}
          label={t("settings.general.allowHorizontalDrag")}
          checked={settings.allow_horizontal_drag}
          onChange={(allow_horizontal_drag) =>
            onChange({ allow_horizontal_drag })
          }
        />
      </SettingRow>

      <SettingRow
        htmlFor={displayId}
        label={t("settings.general.displayChoice")}
        description={displayHint}
      >
        <SelectField
          id={displayId}
          value={displayValue}
          onChange={changeDisplay}
          options={displayOptions}
        />
      </SettingRow>
    </SettingsSection>
  );
}
