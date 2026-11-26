import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useIslandStore } from "../lib/store";

/** Shows backend toasts and a short-lived message for attention events. */
export function ToastHost({ showAttention = true }: { showAttention?: boolean }) {
  const { t } = useTranslation();
  const toast = useIslandStore((state) => state.toast);
  const attention = useIslandStore((state) => showAttention ? state.attention : null);
  const clearToast = useIslandStore((state) => state.clearToast);
  const clearAttention = useIslandStore((state) => state.clearAttention);

  useEffect(() => {
    if (!toast && !attention) return;
    const id = window.setTimeout(() => {
      clearToast();
      clearAttention();
    }, 4500);
    return () => window.clearTimeout(id);
  }, [toast, attention, clearToast, clearAttention]);

  let message = toast?.message ?? "";
  if (!message && attention) {
    const completion = attention.decision.completion;
    const push = attention.decision.push;
    if (completion && completion.presentation !== "status_only") {
      message = t(completion.turn_failed ? "toast.failed" : "toast.completed");
    } else if (push?.kind === "approval") {
      message = t("toast.newApproval");
    } else if (push?.kind === "question") {
      message = t("toast.newQuestion");
    } else if (push?.kind === "error") {
      message = t("toast.failed");
    } else if (push?.kind === "completion") {
      message = t("toast.completed");
    }
    // Sound-only attention events (session start, prompt, heartbeat) do not
    // produce a toast; the waiting badge covers approvals and questions.
  }

  if (!message) return null;

  const isError = toast?.kind === "error";
  return (
    <div
      role="status"
      className="toast pixel-frame-strong absolute right-2 top-2 z-20 flex max-w-[300px] items-start gap-1.5 bg-raised px-2 py-1.5"
    >
      <span
        aria-hidden="true"
        className={`text-[10px] ${isError ? "text-danger" : "text-accent"}`}
      >
        ◆
      </span>
      <span className="min-w-0 break-words text-[10px] leading-snug text-ink">
        {message}
      </span>
      <button
        type="button"
        aria-label={t("toast.dismiss")}
        title={t("toast.dismiss")}
        onClick={() => {
          clearToast();
          clearAttention();
        }}
        className="ml-auto flex-none text-[11px] leading-none text-dim hover:text-ink"
      >
        ×
      </button>
    </div>
  );
}
