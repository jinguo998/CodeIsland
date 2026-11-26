import type { DragEvent, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { agentLabelKey } from "../lib/agents";
import { DEFAULT_SETTINGS } from "../lib/defaults";
import { parseIso, previewText } from "../lib/format";
import i18n from "../lib/i18n";
import { isActiveStatus, isWaitingStatus, statusKeyFor, statusLabelKey } from "../lib/status";
import { useIslandStore } from "../lib/store";
import type { SessionSnapshot } from "../lib/types";
import { CliIcon } from "./CliIcon";
import { Mascot } from "./Mascot";
import { StatusChip } from "./StatusChip";
import { useSessionJump } from "./useSessionJump";

export interface SessionCardProps {
  session: SessionSnapshot;
  now: number;
  dragging?: boolean;
  dragOver?: boolean;
  onDragStart?: () => void;
  onDragEnd?: () => void;
  onDragOver?: () => void;
  onDrop?: () => void;
  onRemove?: () => void;
  markedForRemoval?: boolean;
  children?: ReactNode;
}

function shortSessionId(sessionId: string): string {
  const compact = sessionId.replace(/-/g, "");
  return compact.length >= 8 ? compact.slice(-4) : sessionId.slice(0, 4);
}

export function sessionHeadline(session: SessionSnapshot, showProjectName: boolean): string {
  const title = session.title?.trim();
  const label = title || i18n.t(agentLabelKey(session.agent));
  return showProjectName ? session.project?.trim() || label : label;
}

function sessionAge(startedAt: string, now: number): string {
  const start = parseIso(startedAt);
  if (start === null) return "—";
  const minutes = Math.floor((now - start) / 60_000);
  if (minutes < 1) return "<1m";
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  return `${Math.floor(minutes / 1440)}d`;
}

function modelLabel(rawModel: string | null | undefined, effort: string | null | undefined) {
  const model = rawModel?.trim();
  if (!model || model.startsWith("<")) return null;
  const context = model.match(/\[([^\]]*)\]$/);
  const base = (context ? model.slice(0, context.index).trimEnd() : model)
    .replace(/^.*\/(?!$)/, "")
    .replace(/^.*anthropic\./, "")
    .replace(/@.*$/, "")
    .replace(/-v\d[\d:]*:[\d:]*$/, "")
    .replace(/-(?:\d{8}|\d{4}-\d{2}-\d{2})$/, "");
  if (!base) return null;
  const claude = base.toLowerCase();
  const parts = claude.split("-");
  const family = parts.find((part) => /^[a-z]+$/.test(part) && part !== "claude");
  const versions = parts.filter((part) => /^\d{1,2}$|^\d[\d.]*\.[\d.]*$/.test(part));
  const knownAlias = ["opus", "sonnet", "haiku", "fable"].includes(claude);
  const knownVersion = parts[0] === "claude" && versions.length <= 2 && versions.length + 2 === parts.length;
  const name = family && (knownAlias || knownVersion)
    ? [family[0].toUpperCase() + family.slice(1), versions.join(".")].filter(Boolean).join(" ")
    : base;
  const variant = context?.[1].trim().toUpperCase();
  const reasoning = effort?.trim().toLowerCase();
  return [variant ? `${name} ${variant}` : name, reasoning].filter(Boolean).join(" · ");
}

export function SessionCard({
  session,
  now,
  dragging = false,
  dragOver = false,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDrop,
  onRemove,
  markedForRemoval = false,
  children,
}: SessionCardProps) {
  const { t } = useTranslation();
  const settings = useIslandStore(
    (state) => state.ui?.settings ?? DEFAULT_SETTINGS,
  );
  const childIds = session.child_session_ids;
  const childSessions = useIslandStore((state) => settings.show_agent_details && childIds.length
    ? state.ui?.store.sessions
    : undefined);
  const openWorkspace = useIslandStore((state) => state.openWorkspace);
  const jump = useSessionJump(session.session_id, "session_list", dragging);
  const status = statusKeyFor(session);
  const cowork = session.agent === "claude" && session.session_id.startsWith("cowork:");
  const coworkWait = cowork && session.status === "waiting_answer"
    ? session.active_tool_detail
      ? t("session.coworkWaitingQuestion", { detail: session.active_tool_detail })
      : t("session.coworkWaitingQuestionGeneric")
    : cowork && session.status === "waiting_permission"
      ? [
          t("session.coworkWaitingApproval", { detail: session.active_tool ?? "" }),
          session.active_tool_detail,
        ].filter(Boolean).join(" · ")
      : undefined;
  const coworkJump = cowork && (
    <button
      type="button"
      className="pixel-button"
      data-variant="ghost"
      title={coworkWait}
      onClick={() => void openWorkspace(session.session_id)}
    >
      {coworkWait || t("session.openClaudeDesktop")}
    </button>
  );
  const displayName = sessionHeadline(session, settings.show_project_name);
  const sessionTitle = settings.show_project_name && session.title?.trim();
  const showSessionTitle = sessionTitle && sessionTitle !== displayName;
  const displayId = shortSessionId(session.provider_session_id?.trim() || session.session_id);
  const prompt = session.last_prompt?.trim();
  const message = session.last_message?.trim();
  const recap = !isActiveStatus(session.status) && !isWaitingStatus(session.status)
    ? session.recap?.trim()
    : undefined;
  const age = sessionAge(session.started_at, now);
  const agent = t(agentLabelKey(session.agent));
  const branch = session.git_branch?.trim() || null;
  const terminal = session.terminal_kind?.trim();
  const terminalKey = terminal?.toLowerCase();
  const terminalLabel = terminalKey === "codex" ? "Codex" : terminalKey === "claude" ? "Claude" : terminal;
  const terminalIcon = terminalKey === "codex" || terminalKey === "claude" || terminalKey === "cursor";
  const model = settings.show_model_label && modelLabel(session.model, session.reasoning);
  const subCount = childIds.length;
  const tasksDone = session.tasks.filter(
    (task) => task.status === "completed",
  ).length;
  const currentTask = session.tasks.find(
    (task) => task.status === "in_progress",
  );
  const showToolRow =
    Boolean(session.error) ||
    (settings.show_tool_status && Boolean(session.active_tool) &&
      (status === "working" || status === "thinking" || (status === "needs_you" && !children)));

  const compact = settings.density === "compact";
  const compactTool = settings.show_tool_status && (status === "working" || status === "thinking")
    ? session.active_tool
    : null;
  const currentActivity = status === "needs_you"
    ? prompt
    : status === "working" || status === "thinking"
      ? compactTool ?? prompt ?? t(statusLabelKey(status))
      : (settings.show_session_recap ? recap : undefined) ?? message ?? prompt;

  const handleDragStart = (event: DragEvent<HTMLSpanElement>) => {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", session.session_id);
    onDragStart?.();
  };

  return (
    <article
      {...jump}
      className="session-card"
      data-status={status}
      data-layout={compact ? "compact" : "comfortable"}
      data-request-expanded={compact && status === "needs_you" && Boolean(children)}
      data-marked-for-removal={markedForRemoval || undefined}
      data-dragging={dragging}
      data-drag-over={dragOver}
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        onDragOver?.();
      }}
      onDrop={(event) => {
        event.preventDefault();
        onDrop?.();
      }}
      aria-label={`${displayName} — ${agent}`}
      aria-description={markedForRemoval ? t("idleRemoval.marked") : jump["aria-description"]}
    >
      {compact ? (
        <>
          <div
            className="session-compact-row"
            title={currentActivity ? `${displayName}\n${previewText(currentActivity, 280)}` : displayName}
          >
            <span className="session-compact-mascot"><Mascot status={status} agent={session.agent} /></span>
            <span className="session-project">{displayName}</span>
            <span className="session-compact-status">{t(statusLabelKey(status))}</span>
            <span className="session-compact-activity">
              {compactTool
                ? <span className="tool-chip">▸ {compactTool}</span>
                : currentActivity}
            </span>
          </div>
          {(children || coworkJump) && (
            <div className="session-compact-request">{children}{coworkJump}</div>
          )}
        </>
      ) : (
        <div className="session-grid">
          <div className="mascot-box">
            <Mascot status={status} agent={session.agent} />
            {settings.show_agent_details && subCount > 0 && (
              <span className="session-subagents" aria-hidden="true">
                {childIds.map((id) => {
                  const child = childSessions?.find((entry) => entry.session_id === id);
                  return <span key={id} data-active={child ? child.status !== "idle" : undefined} />;
                })}
              </span>
            )}
          </div>

          <div className="session-body">
            <div className="session-top">
              <StatusChip status={status} />
              <span className="session-identity">
                <span className="session-project" title={displayName}>
                  {displayName}
                </span>
                {settings.show_git_branch && branch && (
                  <span className="session-branch" title={branch}>
                    <svg className="branch-icon" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeLinecap="round" aria-hidden="true">
                      <path d="M3 4v4m1.4 0C7 8 9 6.5 9 4" />
                      <circle cx="3" cy="2.5" r="1.5" /><circle cx="9" cy="2.5" r="1.5" /><circle cx="3" cy="9.5" r="1.5" />
                    </svg>
                    <span className="session-branch-name">{branch}</span>
                  </span>
                )}
                {showSessionTitle && (
                  <span className="session-label" title={sessionTitle}>#{sessionTitle}</span>
                )}
                <span className="session-id">
                  <span>#{displayId}</span>
                </span>
              </span>
              <span className="session-meta">
                {subCount > 0 && (
                  <span className="session-tag session-tag--sub" title={t("session.subCount", { count: subCount })}>
                    {t("session.subCount", { count: subCount })}
                  </span>
                )}
                {model && <span className="session-tag session-tag--model" title={session.model ?? undefined}>{model}</span>}
                <span className="session-tag session-tag--age">{age}</span>
                {terminalLabel && (
                  <span className="session-terminal" title={terminalLabel}>
                    {terminalIcon && <CliIcon source={terminalKey} />}
                    <span className="session-terminal-name">{terminalLabel}</span>
                  </span>
                )}
              </span>
              {onRemove && (
                <button
                  type="button"
                  className="session-remove"
                  aria-label={t("session.remove", { name: displayName })}
                  title={t("session.remove", { name: displayName })}
                  onClick={onRemove}
                >
                  ×
                </button>
              )}
            </div>

          {children}
          {coworkJump}

          {prompt && (
            <div className="session-prompt" title={prompt}>
              <span className="glyph">›</span>
              <span className="text">{previewText(prompt, 200)}</span>
            </div>
          )}

          {message && (
            <div className="session-activity" title={message}>
              <span className="glyph">●</span>
              <span className="text">{previewText(message, 280)}</span>
            </div>
          )}

          {settings.show_session_recap && recap && (
            <div className="session-recap" title={recap}>
              <span className="glyph">↺</span>
              <span className="text">{previewText(recap, 280)}</span>
            </div>
          )}

          {settings.show_task_progress && session.tasks.length > 0 && (
            <div className="session-task">
              <span className="glyph">▣</span>
              <span className="text">
                {t("session.taskProgress", {
                  done: tasksDone,
                  total: session.tasks.length,
                })}
                {currentTask ? ` · ${currentTask.title}` : ""}
              </span>
            </div>
          )}

          {showToolRow && (
            <div className="session-tool">
              {settings.show_tool_status && session.active_tool && (
                <span className="tool-chip">▸ {session.active_tool}</span>
              )}
              {(session.error || settings.show_tool_status) && (
                <span
                  className="tool-text"
                  title={session.error ?? session.active_tool_detail ?? session.cwd ?? undefined}
                >
                  {session.error ?? (!coworkWait ? session.active_tool_detail : undefined) ?? session.cwd ?? session.model ?? ""}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
      )}

      <span
        role="button"
        tabIndex={0}
        draggable
        aria-label={t("session.drag", { name: displayName })}
        title={t("a11y.dragHandle")}
        className="drag-handle"
        onDragStart={handleDragStart}
        onDragEnd={() => onDragEnd?.()}
      >
        ⠿
      </span>
    </article>
  );
}
