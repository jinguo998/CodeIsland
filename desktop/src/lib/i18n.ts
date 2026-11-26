import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import enUS from "../i18n/en-US.json";
import zhCN from "../i18n/zh-CN.json";
import zhTW from "../i18n/zh-TW.json";
import deDE from "../i18n/de-DE.json";
import jaJP from "../i18n/ja-JP.json";
import koKR from "../i18n/ko-KR.json";
import trTR from "../i18n/tr-TR.json";

export const SUPPORTED_LANGUAGES = [
  "en-US",
  "zh-CN",
  "zh-TW",
  "de-DE",
  "ja-JP",
  "ko-KR",
  "tr-TR",
] as const;
export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number];

const LANGUAGE_ALIASES: Record<string, SupportedLanguage> = {
  en: "en-US",
  "en-us": "en-US",
  zh: "zh-CN",
  "zh-cn": "zh-CN",
  "zh-hans": "zh-CN",
  "zh-tw": "zh-TW",
  "zh-hk": "zh-TW",
  "zh-mo": "zh-TW",
  "zh-hant": "zh-TW",
  de: "de-DE",
  "de-de": "de-DE",
  ja: "ja-JP",
  "ja-jp": "ja-JP",
  ko: "ko-KR",
  "ko-kr": "ko-KR",
  tr: "tr-TR",
  "tr-tr": "tr-TR",
};

function isSupportedLanguage(value: string): value is SupportedLanguage {
  return (SUPPORTED_LANGUAGES as readonly string[]).includes(value);
}

function resolveNavigatorLanguage(): SupportedLanguage {
  const navigatorLanguage =
    typeof navigator !== "undefined" ? navigator.language : "en-US";
  const lower = navigatorLanguage.toLowerCase();
  if (lower.startsWith("zh")) {
    return /hant|tw|hk|mo/.test(lower) ? "zh-TW" : "zh-CN";
  }
  return LANGUAGE_ALIASES[lower] ?? LANGUAGE_ALIASES[lower.split("-")[0]] ?? "en-US";
}

/**
 * The backend stores a language setting (`system` plus a BCP-47 tag). Resolve
 * `system` against the browser/OS locale; unknown tags fall back to English.
 */
export function resolveLanguage(setting: string): SupportedLanguage {
  if (isSupportedLanguage(setting)) return setting;
  const alias = LANGUAGE_ALIASES[setting.toLowerCase()];
  if (alias) return alias;
  if (setting === "system" || !setting) return resolveNavigatorLanguage();
  return "en-US";
}

if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    resources: {
      "en-US": { translation: enUS },
      "zh-CN": { translation: zhCN },
      "zh-TW": { translation: zhTW },
      "de-DE": { translation: deDE },
      "ja-JP": { translation: jaJP },
      "ko-KR": { translation: koKR },
      "tr-TR": { translation: trTR },
    },
    lng: resolveLanguage("system"),
    fallbackLng: "en-US",
    supportedLngs: [...SUPPORTED_LANGUAGES],
    interpolation: { escapeValue: false },
    returnNull: false,
  });
}

export function applyLanguage(setting: string): SupportedLanguage {
  const language = resolveLanguage(setting);
  if (i18n.language !== language) {
    void i18n.changeLanguage(language);
  }
  if (typeof document !== "undefined") {
    document.documentElement.lang = language;
  }
  return language;
}

export default i18n;
