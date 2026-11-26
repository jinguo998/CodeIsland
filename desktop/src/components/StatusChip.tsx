import { useTranslation } from "react-i18next";
import { statusLabelKey, type StatusKey } from "../lib/status";

export function StatusChip({
  status,
  className = "",
}: {
  status: StatusKey;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <span
      data-status={status}
      className={`status-chip ${className}`}
    >
      {t(statusLabelKey(status))}
    </span>
  );
}
