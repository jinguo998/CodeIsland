import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { AppLogo } from "../../components/AppLogo";
import { PixelButton } from "../../components/PixelButton";
import * as ipc from "../../lib/ipc";
import type { AppSettings } from "../../lib/types";
import { SettingsSection } from "../controls";

const REPO_URL = "https://github.com/wxtsky/CodeIsland";
const DOCS_URL = `${REPO_URL}#readme`;
const ISSUES_URL = `${REPO_URL}/issues`;
const RELEASES_URL = `${REPO_URL}/releases/latest`;

function releaseNotesUrl(version: string): string {
  const tag = version.trim().replace(/^[vV]/, "");
  return /^[0-9][0-9A-Za-z.+-]*$/.test(tag)
    ? `${REPO_URL}/releases/tag/v${tag}`
    : `${REPO_URL}/releases`;
}

export function AboutTab({
  version,
  settings,
  onChange,
}: {
  version: string;
  settings: AppSettings;
  onChange: (patch: Partial<AppSettings>) => void;
}) {
  const { t } = useTranslation();
  const [imported, setImported] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [diagnosticsPath, setDiagnosticsPath] = useState<string | null>(null);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  const exportSettings = () => {
    const blob = new Blob([JSON.stringify(settings, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "codeisland-settings.json";
    link.click();
    URL.revokeObjectURL(url);
  };

  const importSettings = async (file: File) => {
    try {
      const parsed = JSON.parse(await file.text()) as Partial<AppSettings>;
      onChange(parsed);
      setImported(true);
      setImportError(null);
    } catch (cause) {
      setImported(false);
      setImportError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const exportNow = async () => {
    setExporting(true);
    setDiagnosticsError(null);
    setDiagnosticsPath(null);
    try {
      setDiagnosticsPath(await ipc.exportDiagnosticsAs(t("settings.about.diagnostics")));
    } catch (cause) {
      setDiagnosticsError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setExporting(false);
    }
  };

  return (
    <div>
      <SettingsSection>
        <div className="flex items-center gap-3 px-3.5 py-3">
          <AppLogo showBackground />
          <div>
            <div className="text-[14px] font-semibold text-white/90">
              {t("app.name")}
            </div>
            <div className="text-[12px] text-white/45">
              {t("settings.about.version", { version })}
            </div>
          </div>
        </div>
      </SettingsSection>

      <SettingsSection title={t("settings.about.updates")}>
        <div className="flex flex-wrap items-center gap-2 px-3.5 py-2.5">
          <PixelButton
            onClick={() => {
              void openUrl(RELEASES_URL).catch(() => {
                window.open(RELEASES_URL, "_blank", "noreferrer");
              });
            }}
          >
            {t("settings.about.checkUpdates")}
          </PixelButton>
          <span className="text-[11px] text-white/45">
            {t("settings.about.updatesHint")}
          </span>
        </div>
      </SettingsSection>

      <SettingsSection title={t("settings.about.links")}>
        {[
          { href: DOCS_URL, label: t("settings.about.docs") },
          { href: REPO_URL, label: t("settings.about.repo") },
          { href: ISSUES_URL, label: t("settings.about.issues") },
          { href: releaseNotesUrl(version), label: t("settings.about.releaseNotes", { version }) },
        ].map((link) => (
          <div
            key={link.href}
            className="border-b border-white/[0.06] px-3.5 py-2.5 last:border-b-0"
          >
            <a
              href={link.href}
              target="_blank"
              rel="noreferrer"
              onClick={(event) => {
                if (!ipc.isTauri()) return;
                event.preventDefault();
                void openUrl(link.href).catch(() => window.open(link.href, "_blank", "noreferrer"));
              }}
              className="text-[13px] text-[#73b8ff] hover:underline"
            >
              {link.label}
            </a>
          </div>
        ))}
      </SettingsSection>

      <SettingsSection title={t("settings.about.settingsFile")}>
        <div className="flex flex-wrap items-center gap-2 px-3.5 py-2.5">
          <PixelButton onClick={exportSettings}>
            {t("settings.about.exportSettings")}
          </PixelButton>
          <label className="pixel-button cursor-pointer">
            {t("settings.about.importSettings")}
            <input
              type="file"
              accept=".json"
              className="hidden"
              onChange={(event) => {
                const file = event.currentTarget.files?.[0];
                if (file) void importSettings(file);
                event.currentTarget.value = "";
              }}
            />
          </label>
          {imported && <span role="status" className="text-[11px] text-[#6ee7a8]">{t("settings.about.importDone")}</span>}
          {importError && (
            <span role="alert" className="text-[11px] text-[#ff7b7b]">
              {t("settings.about.importFailed")}: {importError}
            </span>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t("settings.about.diagnostics")}>
        <div className="flex flex-wrap items-center gap-2 px-3.5 py-2.5">
          <PixelButton disabled={exporting} onClick={() => void exportNow()}>
            {t("settings.about.diagnostics")}
          </PixelButton>
          <span className="text-[11px] text-white/45">{t("settings.about.diagnosticsHint")}</span>
          {diagnosticsPath && (
            <span role="status" className="text-[11px] text-[#6ee7a8]">
              {t("settings.about.diagnosticsDone", { path: diagnosticsPath })}
            </span>
          )}
          {diagnosticsError && (
            <span role="alert" className="text-[11px] text-[#ff7b7b]">
              {t("settings.about.diagnosticsFailed")}: {diagnosticsError}
            </span>
          )}
        </div>
      </SettingsSection>
    </div>
  );
}
