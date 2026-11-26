import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { agentLabelKey } from "../lib/agents";
import { focusSessionTerminal, setIslandSurface } from "../lib/ipc";
import { playCue } from "../lib/sound";
import { useIslandStore } from "../lib/store";
import type { UserAction } from "../lib/types";
import { CliIcon } from "./CliIcon";
import { FocusWindowIcon } from "./icons";
import { PixelButton } from "./PixelButton";
import type { RequestCardProps } from "./RequestCard";

function mcpParts(tool: string) {
  const match = /^mcp__(.+?)__(.+)$/.exec(tool);
  return match ? { server: match[1], tool: match[2] } : undefined;
}

function shortTool(tool: string) {
  return tool.length > 32 ? `${tool.slice(0, 15)}…${tool.slice(-16)}` : tool;
}

function approvalAction(tool: string) {
  const name = tool.toLowerCase().replace(/-/g, "_");
  if (["task", "agent"].includes(name) || /subagent|spawn_agent/.test(name)) return "agent";
  if (/web_?fetch|web_?search|fetch_url/.test(name)) return "web";
  if (/^(bash|command)$|shell|exec_command|write_stdin|run_command|execute_command|terminal/.test(name)) return "command";
  if (/edit|write|apply_patch|replace|create_file|delete_file|move_file/.test(name)) return "change";
  if (/^read$|read_file|view_file|list_dir/.test(name)) return "read";
  if (/^(grep|glob|find)$|search/.test(name)) return "search";
  return "other";
}

function patternWords(pattern: string[]) {
  return pattern.map((word) => /^[a-z0-9@%+=:,./_-]+$/i.test(word)
    ? word
    : "'" + word.replace(/\n/g, "\\n").replace(/\r/g, "\\r").replace(/\t/g, "\\t").replace(/'/g, "'\\''") + "'",
  ).join(" ");
}

function ApprovalDetails({ tool, input }: { tool: string; input: Record<string, unknown> }) {
  const { t } = useTranslation();
  const text = (key: string) => typeof input[key] === "string" ? input[key] : undefined;
  const path = (value?: string) => value && (
    <div className="approval-detail-path" title={value}><bdi>{value}</bdi></div>
  );
  const line = (value: string | undefined, glyph: string, tone: string, limit?: number) => value && (
    <div className="approval-detail-line" data-tone={tone}>
      <span aria-hidden="true">{glyph}</span>
      <div className={limit ? "approval-detail-excerpt" : undefined}>
        {limit ? [...value].slice(0, limit).join("") : value}
      </div>
    </div>
  );

  switch (tool) {
    case "Bash":
      return <>{line(text("command"), "$", "command")}{text("description") && <small>{text("description")}</small>}</>;
    case "Edit":
      return <>{path(text("file_path"))}{line(text("old_string"), "−", "removed", 120)}{line(text("new_string"), "+", "added", 120)}</>;
    case "Write":
      return <>{path(text("file_path"))}<small className="approval-detail-write">{[...(text("content") ?? "")].slice(0, 200).join("")}</small></>;
    case "Read": {
      const offset = input.offset;
      const limit = input.limit;
      return <>{path(text("file_path"))}{typeof offset === "number" && typeof limit === "number" && (
        <small>{t("approval.lines")} {offset + 1}–{offset + limit}</small>
      )}</>;
    }
    case "Grep":
      return <>{line(text("pattern"), "/", "pattern", 160)}{path(text("path"))}</>;
    case "Glob":
      return <><span className="approval-detail-glob">{text("pattern")}</span>{path(text("path"))}</>;
    default:
      return Object.keys(input).sort().slice(0, 4).map((key) => (
        <div key={key} className="approval-detail-line" data-tone="field">
          <span>{key}</span>
          <small className="approval-detail-excerpt">{[...(text(key) ?? JSON.stringify(input[key]) ?? "")].slice(0, 160).join("")}</small>
        </div>
      ));
  }
}

export function ApprovalCard({
  request,
  standalone = true,
  position = 1,
  total = 1,
}: RequestCardProps) {
  const { t } = useTranslation();
  const resolveRequest = useIslandStore((state) => state.resolveRequest);
  const hideRequest = useIslandStore((state) => state.hideRequest);
  const settings = useIslandStore((state) => state.ui?.settings);
  const session = useIslandStore((state) =>
    state.ui?.store.sessions.find((entry) => entry.session_id === request.session_id),
  );
  const [showDetails, setShowDetails] = useState(false);
  const [jumpFailed, setJumpFailed] = useState(false);
  const jumpAttempt = useRef(0);
  useEffect(() => () => { jumpAttempt.current += 1; }, []);
  const tool = request.tool || t("request.tool");
  const input = request.tool_input;
  const structured = input && typeof input === "object" && !Array.isArray(input);
  const mcp = mcpParts(tool);
  const displayName = mcp?.tool ?? shortTool(tool);
  const filePath = [input?.file_path, input?.notebook_path, input?.path].find((path) => typeof path === "string" && path.length);
  const fileName = typeof filePath === "string" ? filePath.split(/[\\/]/).filter(Boolean).pop() : undefined;
  const action = approvalAction(tool);
  const titleKey = fileName && (action === "change" || action === "read") ? `${action}File` : action;
  const title = mcp
    ? t("approval.mcp", mcp)
    : !structured && request.title && request.title !== tool
      ? request.title
      : t(`approval.${titleKey}`, { tool: displayName, file: fileName });
  const patterns = request.always_scope?.kind === "codex_rules" ? request.always_scope.patterns : null;
  const savesMcp = request.always_scope?.kind === "codex_mcp_config";
  const commands = patterns?.map(patternWords).join(t("approval.or"));
  const alwaysHint = patterns ? t("approval.savedRulesHint", { commands })
    : savesMcp ? t("approval.savedMcpHint") : t("approval.sessionHint");
  const alwaysLabel = standalone
    ? patterns ? t("approval.alwaysSavedPrefix", { commands, path: "~/.codex/rules" })
      : savesMcp ? t("approval.alwaysSaved", { path: "~/.codex/config.toml" })
        : t("approval.alwaysSession", { tool: displayName })
    : patterns ? t("approval.inlineAlwaysPrefix", { commands: patterns.map(patternWords).join(" …" + t("approval.or")) })
      : t("approval.inlineAlways", { tool: shortTool(tool) });

  const project = session?.project ?? session?.cwd?.split(/[\\/]/).filter(Boolean).pop();
  const context = settings?.show_project_name === false ? session?.title ?? project : project ?? session?.title;
  const branch = settings?.show_git_branch !== false ? session?.git_branch : undefined;
  const server = typeof input?.server_name === "string" ? input.server_name : undefined;
  const description = typeof input?.description === "string" ? input.description.trim() : "";
  const command = tool === "Bash" && typeof input?.command === "string" ? input.command.trim() : "";
  const summary = description || command || request.body || (!structured && request.title !== tool ? request.title : null);
  const queued = !standalone && position !== 1;
  const queueHint = queued ? t("approval.queued") : undefined;
  const resolve = (value: UserAction) => void resolveRequest(request.event_id, value);
  const shortcut = (enabled?: boolean, binding?: string) =>
    standalone && enabled && binding ? binding : undefined;
  const approveKey = shortcut(settings?.approve_shortcut_enabled, settings?.approve_shortcut);
  const denyKey = shortcut(settings?.deny_shortcut_enabled, settings?.deny_shortcut);
  const alwaysKey = shortcut(settings?.approve_always_shortcut_enabled, settings?.approve_always_shortcut);
  const keyBadge = (key?: string) => key && <kbd aria-hidden="true">{key}</kbd>;
  const accessibleShortcut = (key?: string) => key?.replace(/\bCtrl\b/g, "Control").replace(/\bSuper\b/g, "Meta");
  const detail: ReactNode = structured && Object.keys(input).length ? <ApprovalDetails tool={tool} input={input} /> : request.body;
  const canJump = standalone && session && !session.session_id.startsWith("ssh:") &&
    Boolean(session.pid || session.terminal_kind);
  const focusTerminal = async () => {
    if (!canJump) return;
    const attempt = ++jumpAttempt.current;
    setJumpFailed(false);
    const before = useIslandStore.getState().ui;
    const focused = await focusSessionTerminal(session.session_id).catch(() => false);
    if (attempt !== jumpAttempt.current || !before?.settings.auto_collapse_after_session_jump) return;

    const current = useIslandStore.getState().ui;
    if (current?.surface_revision !== before.surface_revision ||
        current?.surface?.kind !== "approval_card" ||
        current.surface.event_id !== request.event_id) return;
    if (focused) {
      await setIslandSurface({ kind: "collapsed" });
    } else {
      playCue("error", current.settings.sound_volume, current.settings.custom_sound_paths["8bit_error"]);
      setJumpFailed(true);
    }
  };
  const clickDetail = canJump ? () => {
    if (!window.getSelection()?.toString()) void focusTerminal();
  } : undefined;
  const contextContents = (
    <>
      {session && (
        <span className="approval-agent-icon" role="img" aria-label={t(agentLabelKey(request.agent))}>
          <CliIcon source={request.agent} />
        </span>
      )}
      <span className="approval-context-title">{context}</span>
      {branch && <span className="approval-context-branch" title={branch}>⑂ {branch}</span>}
    </>
  );

  return (
    <div
      className="request-card"
      data-request-kind="permission"
      data-standalone={standalone}
      data-jump-failed={jumpFailed || undefined}
      onAnimationEnd={(event) => {
        if (event.target === event.currentTarget) setJumpFailed(false);
      }}
    >
      {standalone && (session || total > 1) && (
        <div className="request-context">
          {canJump ? (
            <button
              type="button"
              className="approval-context-target"
              title={t("approval.jumpHint")}
              aria-label={t("approval.jumpHint")}
              onClick={() => void focusTerminal()}
            >
              {contextContents}<FocusWindowIcon />
            </button>
          ) : (
            <div className="approval-context-target">{contextContents}</div>
          )}
          {total > 1 && <span className="request-progress">{position}/{total}</span>}
        </div>
      )}
      <div
        className="request-head"
        aria-label={standalone ? undefined : t("approval.queueLabel", { position, total, tool })}
        onClick={clickDetail}
      >
        <span className="request-kind-tag">{standalone ? t("approval.permissionTag") : `▸ ${tool}`}</span>
        <span className="request-title" title={standalone ? `${title}\n${tool}` : summary ?? undefined}>
          {standalone ? title : summary}
        </span>
        {standalone && server && <span className="approval-server">({server})</span>}
        {!standalone && total > 1 && <span className="request-progress">{position}/{total}</span>}
      </div>
      {standalone && detail && (
        <div className="request-body" tabIndex={0} aria-label={title} onClick={clickDetail}>{detail}</div>
      )}
      <div className="request-actions approval-actions">
        <span className="approval-always" title={queueHint ?? alwaysHint}>
          <PixelButton variant="link" disabled={queued} aria-keyshortcuts={accessibleShortcut(alwaysKey)} onClick={() => resolve("always_allow")}>
            <span>{alwaysLabel}</span>{keyBadge(alwaysKey)}
          </PixelButton>
        </span>
        <div className="approval-decisions" title={queueHint}>
          {standalone ? (
            <PixelButton variant="ghost" title={t("approval.hideHint")} onClick={() => void hideRequest(request.event_id)}>
              {t("approval.hide")}
            </PixelButton>
          ) : detail && (
            <PixelButton variant="ghost" aria-expanded={showDetails} onClick={() => setShowDetails(!showDetails)}>
              {t(showDetails ? "approval.collapseDetails" : "approval.details")}
            </PixelButton>
          )}
          <PixelButton variant="danger" disabled={queued} aria-keyshortcuts={accessibleShortcut(denyKey)} onClick={() => resolve("deny")}>
            {t("approval.deny")}{keyBadge(denyKey)}
          </PixelButton>
          <PixelButton variant="primary" disabled={queued} aria-keyshortcuts={accessibleShortcut(approveKey)} onClick={() => resolve("approve")}>
            {t("approval.allowOnce")}{keyBadge(approveKey)}
          </PixelButton>
        </div>
      </div>
      {!standalone && showDetails && detail && <div className="request-body" tabIndex={0} aria-label={title}>{detail}</div>}
    </div>
  );
}
