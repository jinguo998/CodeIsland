import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { applyLanguage } from "../lib/i18n";
import { resetMockBackend } from "../lib/mock";
import { useIslandStore } from "../lib/store";

// Tests assert against the English copy.
applyLanguage("en-US");

afterEach(() => {
  cleanup();
  resetMockBackend();
  useIslandStore.setState({
    ui: null,
    ready: false,
    attention: null,
    attentionSeq: 0,
    toast: null,
    settingsDraft: {},
    settingsSaveStatus: "idle",
  });
});
