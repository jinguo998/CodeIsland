import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ToastHost } from "../components/ToastHost";
import { useIslandStore } from "../lib/store";
import type { AppSettings } from "../lib/types";
import { AppearanceTab } from "./tabs/AppearanceTab";
import { AboutTab } from "./tabs/AboutTab";
import { BehaviorTab } from "./tabs/BehaviorTab";
import { BuddyTab } from "./tabs/BuddyTab";
import { GeneralTab } from "./tabs/GeneralTab";
import { HooksTab } from "./tabs/HooksTab";
import { MascotsTab } from "./tabs/MascotsTab";
import { RemoteTab } from "./tabs/RemoteTab";
import { ShortcutsTab } from "./tabs/ShortcutsTab";
import { SoundTab } from "./tabs/SoundTab";

const TAB_IDS = [
  "general",
  "behavior",
  "appearance",
  "mascots",
  "sound",
  "shortcuts",
  "remote",
  "hooks",
  "buddy",
  "about",
] as const;

type TabId = (typeof TAB_IDS)[number];

const GROUPS: Array<{ title?: string; pages: TabId[] }> = [
  {
    pages: ["general", "behavior", "appearance", "mascots", "sound", "shortcuts"],
  },
  { title: "CodeIsland", pages: ["remote", "hooks", "buddy", "about"] },
];

const TAB_ICONS: Record<TabId, { glyph: string; color: string }> = {
  general: { glyph: "⚙", color: "#8e8e93" },
  behavior: { glyph: "☰", color: "#ff9f0a" },
  appearance: { glyph: "◐", color: "#0a84ff" },
  mascots: { glyph: "☺", color: "#ff375f" },
  sound: { glyph: "♪", color: "#30d158" },
  shortcuts: { glyph: "⌘", color: "#5e5ce6" },
  remote: { glyph: "⇄", color: "#63e6e2" },
  hooks: { glyph: "⛓", color: "#bf5af2" },
  buddy: { glyph: "◉", color: "#ff453a" },
  about: { glyph: "i", color: "#64d2ff" },
};

export function SettingsApp() {
  const { t } = useTranslation();
  const ui = useIslandStore((state) => state.ui);
  const updateSettings = useIslandStore((state) => state.updateSettings);
  const retrySettings = useIslandStore((state) => state.retrySettings);
  const settingsDraft = useIslandStore((state) => state.settingsDraft);
  const saveStatus = useIslandStore((state) => state.settingsSaveStatus);
  const savedSettings = ui?.settings;
  const draft = useMemo(() => savedSettings && { ...savedSettings, ...settingsDraft }, [savedSettings, settingsDraft]);

  const [tab, setTab] = useState<TabId>(() => {
    const requested = new URLSearchParams(window.location.search).get("tab");
    return (TAB_IDS as readonly string[]).includes(requested ?? "")
      ? (requested as TabId)
      : "general";
  });
  if (!ui || !draft) {
    return (
      <div className="flex h-full items-center justify-center bg-[#1c1c1e] text-[12px] text-white/50">
        {t("common.loading")}
      </div>
    );
  }

  const change = (patch: Partial<AppSettings>) => void updateSettings(patch);

  const renderTab = () => {
    switch (tab) {
      case "general":
        return <GeneralTab settings={draft} onChange={change} />;
      case "behavior":
        return <BehaviorTab settings={draft} onChange={change} />;
      case "appearance":
        return <AppearanceTab settings={draft} onChange={change} />;
      case "mascots":
        return <MascotsTab settings={draft} onChange={change} />;
      case "sound":
        return <SoundTab settings={draft} onChange={change} />;
      case "shortcuts":
        return <ShortcutsTab settings={draft} onChange={change} />;
      case "remote":
        return <RemoteTab settings={draft} onChange={change} />;
      case "hooks":
        return <HooksTab settings={draft} onChange={change} />;
      case "buddy":
        return <BuddyTab settings={draft} onChange={change} />;
      case "about":
        return <AboutTab version={ui.version} settings={draft} onChange={change} />;
      default:
        return null;
    }
  };

  const status = saveStatus === "error" ? t("settings.saveError")
    : saveStatus === "saving" ? t("settings.saving")
      : saveStatus === "saved" ? t("settings.saved") : "";

  return (
    <div className="flex h-full bg-[#1c1c1e] text-white" role="region" aria-label={t("a11y.settingsLabel")}>
      <aside className="flex w-[200px] flex-none flex-col border-r border-white/[0.08] bg-[#171719]">
        <div
          data-tauri-drag-region
          className="select-none px-3 pb-2 pt-3 text-[13px] font-semibold text-white/90"
        >
          {t("app.name")}
        </div>

        <nav
          role="tablist"
          aria-label={t("settings.tabsLabel")}
          aria-orientation="vertical"
          className="flex flex-col gap-0.5 overflow-y-auto px-2"
          onKeyDown={(event) => {
            const index = TAB_IDS.indexOf(tab);
            if (event.key === "ArrowDown" || event.key === "ArrowRight") {
              event.preventDefault();
              setTab(TAB_IDS[(index + 1) % TAB_IDS.length]);
            } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
              event.preventDefault();
              setTab(TAB_IDS[(index - 1 + TAB_IDS.length) % TAB_IDS.length]);
            }
          }}
        >
          {GROUPS.map((group, groupIndex) => (
            <div key={group.title ?? groupIndex} className="mb-1">
              {group.title && (
                <div className="px-2 pb-1 pt-3 text-[11px] font-medium text-white/40">
                  {group.title}
                </div>
              )}
              {group.pages.map((id) => {
                const selected = tab === id;
                const icon = TAB_ICONS[id];
                return (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    id={`settings-tab-${id}`}
                    aria-selected={selected}
                    aria-controls={`settings-panel-${id}`}
                    tabIndex={selected ? 0 : -1}
                    onClick={() => setTab(id)}
                    className={`flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] ${
                      selected
                        ? "bg-white/[0.12] text-white"
                        : "text-white/80 hover:bg-white/[0.06]"
                    }`}
                  >
                    <span
                      aria-hidden="true"
                      className="grid h-6 w-6 flex-none place-items-center rounded-md text-[12px] font-semibold text-white"
                      style={{ backgroundColor: icon.color }}
                    >
                      {icon.glyph}
                    </span>
                    <span className="truncate">{t(`settings.tabs.${id}`)}</span>
                  </button>
                );
              })}
            </div>
          ))}
        </nav>

        <div className="mt-auto flex items-baseline justify-between gap-2 px-3 py-2 text-[10px] text-white/55">
          <span role="status">{status}</span>
          {saveStatus === "error" && (
            <button type="button" className="text-white underline" onClick={() => void retrySettings()}>
              {t("common.retry")}
            </button>
          )}
        </div>
      </aside>

      <main className="min-w-0 flex-1 overflow-y-auto px-6 py-5">
        <div
          role="tabpanel"
          id={`settings-panel-${tab}`}
          aria-labelledby={`settings-tab-${tab}`}
          className="mx-auto max-w-[760px]"
        >
          {renderTab()}
        </div>
      </main>
      <ToastHost showAttention={false} />
    </div>
  );
}
