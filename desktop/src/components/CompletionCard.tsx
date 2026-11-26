import ReactMarkdown from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { useTranslation } from "react-i18next";
import { agentLabelKey } from "../lib/agents";
import { DEFAULT_SETTINGS } from "../lib/defaults";
import { sessionElapsed } from "../lib/format";
import { statusKeyFor } from "../lib/status";
import { useIslandStore } from "../lib/store";
import type { SessionSnapshot } from "../lib/types";
import { Mascot } from "./Mascot";
import { PixelButton } from "./PixelButton";
import { StatusChip } from "./StatusChip";
import { useSessionJump } from "./useSessionJump";

export interface CompletionCardProps {
  session: SessionSnapshot;
  now: number;
  onOpenWorkspace?: () => void;
  onRemove?: () => void;
}

/** Green/red completion card with sanitized GFM markdown reply. */
export function CompletionCard({
  session,
  now,
  onOpenWorkspace,
  onRemove,
}: CompletionCardProps) {
  const { t } = useTranslation();
  const jump = useSessionJump(session.session_id, "completion_card");
  const settings = useIslandStore(
    (state) => state.ui?.settings ?? DEFAULT_SETTINGS,
  );
  const status = statusKeyFor(session);
  const failed = status === "error";
  const project = session.project?.trim() || null;
  const title = session.title?.trim() || null;
  const displayName = settings.show_project_name
    ? (project ?? title ?? session.session_id)
    : (title ?? project ?? session.session_id);
  const markdown =
    session.last_message ??
    (settings.show_session_recap ? session.recap : null) ??
    "";
  const elapsed = sessionElapsed(session, now);
  const branch = session.git_branch?.trim() || session.cwd?.trim() || null;
  const subCount = session.child_session_ids.length;

  return (
    <article
      {...jump}
      className="session-card"
      data-status={status}
      aria-label={`${t(failed ? "completion.failed" : "completion.done")} · ${displayName}`}
    >
      <div className="session-grid">
        <div className="mascot-box">
          <Mascot status={status} agent={session.agent} />
        </div>
        <div className="session-body">
          <div className="session-top">
            <StatusChip status={status} />
            <span className="session-project">{displayName}</span>
            {subCount > 0 && (
              <span className="session-tag session-tag--sub">
                {t("session.subCount", { count: subCount })}
              </span>
            )}
            {settings.show_git_branch && branch && (
              <span className="session-branch" title={session.cwd ?? branch}>
                <span className="branch-icon" aria-hidden="true">
                  ⑂
                </span>
                {branch}
              </span>
            )}
            <span className="session-meta">
              <span className="session-tag">{elapsed}</span>
              {settings.show_model_label && session.model && (
                <span className="session-tag session-tag--model">
                  {session.model}
                </span>
              )}
              {settings.show_agent_details && (
                <span className="session-tag">
                  {t(agentLabelKey(session.agent))}
                </span>
              )}
              {onRemove && (
                <button
                  type="button"
                  className="session-remove"
                  aria-label={t("completion.dismiss")}
                  title={t("completion.dismiss")}
                  onClick={onRemove}
                >
                  ×
                </button>
              )}
            </span>
          </div>

          <div className="completion-reply markdown mt-2 text-muted">
            {markdown ? (
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                rehypePlugins={[rehypeSanitize]}
              >
                {markdown}
              </ReactMarkdown>
            ) : (
              <p className="text-dim">{t("common.none")}</p>
            )}
          </div>

          {onOpenWorkspace && (
            <div className="request-actions">
              <PixelButton
                variant={failed ? "danger" : "success"}
                onClick={onOpenWorkspace}
                aria-label={`${t("completion.open")} ${displayName}`}
              >
                {t("completion.open")}
              </PixelButton>
            </div>
          )}
        </div>
      </div>
    </article>
  );
}
