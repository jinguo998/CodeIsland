import { useEffect } from "react";
import { IslandApp } from "./components/IslandApp";
import { useIslandStore } from "./lib/store";
import { SettingsApp } from "./settings/SettingsApp";

/** Settings lives in its own window opened with `?window=settings`. */
function isSettingsWindow(): boolean {
  if (typeof window === "undefined") return false;
  const params = new URLSearchParams(window.location.search);
  if (params.get("window") === "settings") return true;
  try {
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__?: {
          metadata?: { currentWindow?: { label?: string } };
        };
      }
    ).__TAURI_INTERNALS__;
    return internals?.metadata?.currentWindow?.label === "settings";
  } catch {
    return false;
  }
}

export default function App() {
  const init = useIslandStore((state) => state.init);
  const settingsWindow = isSettingsWindow();
  const theme = useIslandStore((state) => state.ui?.settings.theme ?? "dark");
  const density = useIslandStore(
    (state) => state.ui?.settings.density ?? "comfortable",
  );

  useEffect(() => {
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void init(!settingsWindow).then((off) => {
      if (cancelled) off();
      else dispose = off;
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [init, settingsWindow]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.density = density;
  }, [theme, density]);

  return settingsWindow ? <SettingsApp /> : <IslandApp />;
}
