import { invoke } from "@tauri-apps/api/core";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PixelButton } from "../../components/PixelButton";
import { isTauri } from "../../lib/ipc";
import { parseShortcut } from "../../lib/settings";
import type { AppSettings } from "../../lib/types";
import { SettingRow, SettingsSection, type SettingsTabProps } from "../controls";

const FIELDS = [
  { key: "toggle_shortcut", label: "toggle" },
  { key: "approve_shortcut", label: "approve" },
  { key: "approve_always_shortcut", label: "approveAlways" },
  { key: "deny_shortcut", label: "deny" },
  { key: "skip_question_shortcut", label: "skipQuestion" },
  { key: "jump_to_terminal_shortcut", label: "jumpToTerminal" },
] as const;

type ShortcutKey = (typeof FIELDS)[number]["key"];

const PUNCTUATION: Readonly<Record<string, string>> = {
  Minus: "-",
  Equal: "=",
  Comma: ",",
  Period: ".",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Slash: "/",
  Backquote: "`",
};

const NAMED_KEYS = new Set([
  "Space", "Enter", "Tab", "Backspace", "Delete", "Home", "End", "PageUp", "PageDown",
  "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight",
]);

function recordedChord(event: KeyboardEvent): string | null {
  if (!event.ctrlKey && !event.altKey && !event.metaKey) return null;
  const code = event.code;
  const character = /^(?:Key([A-Z])|Digit([0-9]))$/.exec(code);
  const key = character?.[1] ?? character?.[2] ?? PUNCTUATION[code]
    ?? (NAMED_KEYS.has(code) || /^F(?:[1-9]|1[0-9]|2[0-4])$/.test(code) ? code : null);
  if (key === null) return null;

  const parts = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.shiftKey) parts.push("Shift");
  if (event.altKey) parts.push("Alt");
  if (event.metaKey) parts.push("Meta");
  return [...parts, key].join("+");
}

function conflictKey(chord: string): string | undefined {
  return parseShortcut(chord)?.sort().join("+");
}

async function setNativeRecording(recording: boolean): Promise<boolean> {
  return isTauri() ? invoke<boolean>("set_shortcut_recording", { recording }) : true;
}

export function ShortcutsTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const id = useId();
  const [recording, setRecording] = useState<ShortcutKey | null>(null);
  const mounted = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      void setNativeRecording(false).catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!recording) return;
    const stop = () => {
      setRecording(null);
      void setNativeRecording(false).catch(() => {});
    };
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.key === "Escape") {
        stop();
        return;
      }
      const chord = recordedChord(event);
      if (chord !== null) {
        onChange({ [recording]: chord, [`${recording}_enabled`]: true } as Partial<AppSettings>);
        stop();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", stop);
    };
  }, [onChange, recording]);

  const start = async (key: ShortcutKey) => {
    try {
      if (await setNativeRecording(true)) {
        if (mounted.current) setRecording(key);
        else await setNativeRecording(false);
      }
    } catch {
      setRecording(null);
    }
  };

  return (
    <SettingsSection>
      {FIELDS.map(({ key, label }) => {
        const enabledKey = `${key}_enabled` as const;
        const enabled = settings[enabledKey];
        const action = t(`settings.shortcuts.${label}`);
        const isRecording = recording === key;
        const conflict = enabled && FIELDS.find((other) =>
          other.key !== key
          && settings[`${other.key}_enabled`]
          && conflictKey(settings[other.key]) === conflictKey(settings[key]),
        );
        const description = conflict
          ? t("settings.shortcuts.conflict", { action: t(`settings.shortcuts.${conflict.label}`) })
          : t(`settings.shortcuts.${label}Hint`);
        const buttonId = `${id}-${key}`;
        return (
          <SettingRow key={key} htmlFor={buttonId} label={action} description={description}>
            <PixelButton
              id={buttonId}
              variant={isRecording ? "primary" : "ghost"}
              aria-label={action}
              onClick={() => void start(key)}
            >
              {isRecording
                ? t("settings.shortcuts.record")
                : enabled ? settings[key] : t("settings.shortcuts.none")}
            </PixelButton>
            {enabled && !isRecording && (
              <PixelButton
                variant="ghost"
                aria-label={t("settings.shortcuts.clear", { action })}
                onClick={() => onChange({ [enabledKey]: false })}
              >
                ×
              </PixelButton>
            )}
          </SettingRow>
        );
      })}
    </SettingsSection>
  );
}
