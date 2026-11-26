import { useTranslation } from "react-i18next";
import { useIslandStore } from "../lib/store";

/** Polite live region announcing pending approvals/questions. */
export function LiveRegion() {
  const { t } = useTranslation();
  const pending = useIslandStore((state) => state.ui?.store.pending);
  const visible = (pending ?? []).filter((request) => !request.hidden);
  const approvals = visible.filter(
    (request) => request.kind === "permission",
  ).length;
  const questions = visible.filter(
    (request) => request.kind === "question",
  ).length;

  const message =
    approvals > 0
      ? t("a11y.pendingApprovals", { count: approvals })
      : questions > 0
        ? t("a11y.waitingQuestions", { count: questions })
        : "";

  return (
    <div
      className="sr-only"
      role="status"
      aria-live="polite"
      aria-atomic="true"
      aria-label={t("a11y.liveRegion")}
    >
      {message}
    </div>
  );
}
