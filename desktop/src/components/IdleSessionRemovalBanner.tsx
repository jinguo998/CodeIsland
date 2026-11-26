import { useLayoutEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { TrashIcon } from "./icons";

export function IdleSessionRemovalBanner({
  count,
  onCancel,
  onRemove,
}: {
  count: number;
  onCancel: () => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const [lastCount, setLastCount] = useState(count);
  const visible = count > 0;
  const title = t("idleRemoval.count", { count: count || lastCount });

  // Keep the outgoing content in place during the short collapse.
  useLayoutEffect(() => {
    if (count) setLastCount(count);
  }, [count]);

  return (
    <>
      <span className="sr-only" role="status">{visible ? title : ""}</span>
      <div className="idle-session-removal-slot" data-visible={visible} inert={!visible} aria-hidden={!visible}>
        <div className="idle-session-removal-clip">
          <div className="idle-session-removal-banner">
            <TrashIcon className="idle-session-removal-icon" />
            <div className="idle-session-removal-copy">
              <div>{title}</div>
              <small>{t("idleRemoval.confirmHelp")}</small>
            </div>
            <button type="button" onClick={onCancel}>{t("idleRemoval.keep")}</button>
            <button type="button" data-destructive="true" onClick={onRemove}>{t("idleRemoval.remove")}</button>
          </div>
        </div>
      </div>
    </>
  );
}
