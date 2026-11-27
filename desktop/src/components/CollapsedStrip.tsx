import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { compactMarkdownPreview } from "../lib/compactMarkdown";
import { DEFAULT_SETTINGS } from "../lib/defaults";
import { openPendingQuestion } from "../lib/ipc";
import { presentedSessions } from "../lib/sessions";
import { inQuietHours } from "../lib/sound";
import { isWaitingStatus, statusKeyFor, type StatusKey } from "../lib/status";
import { useIslandStore } from "../lib/store";
import type { SessionSnapshot } from "../lib/types";
import { ClaudeQuotaChip } from "./ClaudeQuota";
import { CompactMorphText } from "./CompactMorphText";
import { IslandHeaderControls } from "./IslandHeaderControls";
import { BellIcon, MoonIcon, QuestionBadgeIcon } from "./icons";
import { Mascot } from "./Mascot";

const MASCOT_FADE_MS = 300;
const COMPACT_TOOL_LIMIT = 32;

function compactToolName(name: string) {
  const characters = Array.from(name);
  if (characters.length <= COMPACT_TOOL_LIMIT) return name;
  const parts = name.split("__");
  const ending = parts.length > 1 ? parts[parts.length - 1] : "";
  const suffix = ending ? Math.min(Array.from(ending).length, COMPACT_TOOL_LIMIT - 7) : 16;
  return `${characters.slice(0, COMPACT_TOOL_LIMIT - suffix - 3).join("")}...${characters.slice(-suffix).join("")}`;
}

function toolTone(name: string) {
  switch (name.toLowerCase()) {
    case "bash": case "running command": return "command";
    case "edit": case "write": case "editing": return "edit";
    case "read": case "reading": return "read";
    case "grep": case "glob": case "searching": case "calling mcp": return "search";
    case "agent": case "delegating": return "agent";
    case "compacting": return "compacting";
    default: return undefined;
  }
}

function pathLeaf(value: string | null | undefined) {
  return value?.trim().split(/[\\/]/).filter(Boolean).pop();
}

function compactPriority(session: SessionSnapshot): number {
  switch (session.status) {
    case "waiting_permission": return 5;
    case "waiting_answer": return 4;
    case "running": return statusKeyFor(session) === "working" ? 3 : 2;
    case "starting": return 2;
    default: return 0;
  }
}

function useCompactSession(
  sessions: SessionSnapshot[],
  currentId: string | null | undefined,
  rotationInterval: number,
) {
  const ranked = useMemo(() => sessions
    .map((session) => ({ session, priority: compactPriority(session) }))
    .filter(({ priority }) => priority > 0)
    .sort((a, b) => b.priority - a.priority ||
      Date.parse(b.session.last_activity) - Date.parse(a.session.last_activity)), [sessions]);
  const rotationActive = ranked.length > 1;
  const [rotatingId, setRotatingId] = useState<string | null>(null);
  const latest = useRef(ranked);

  useLayoutEffect(() => {
    latest.current = ranked;
  }, [ranked]);

  useLayoutEffect(() => {
    setRotatingId((previous) => {
      if (ranked.length < 2) return null;
      const top = ranked[0];
      const displayed = ranked.find(({ session }) => session.session_id === previous);
      return !displayed || top.priority > displayed.priority ? top.session.session_id : previous;
    });
  }, [ranked]);

  useEffect(() => {
    if (!rotationActive) return;
    const timer = window.setInterval(() => {
      setRotatingId((previous) => {
        const members = latest.current;
        if (members.length < 2) return null;
        const index = members.findIndex(({ session }) => session.session_id === previous);
        return members[(index + 1) % members.length].session.session_id;
      });
    }, rotationInterval * 1000);
    return () => window.clearInterval(timer);
  }, [rotationActive, rotationInterval]);

  const current = ranked.find(({ session }) => session.session_id === rotatingId) ?? ranked[0];
  return {
    current: current?.session ?? sessions.find((session) => session.session_id === currentId) ?? sessions[0],
    activeCount: ranked.length,
  };
}

function CompactMascot({ agent, status }: { agent: string; status: StatusKey }) {
  const last = useRef({ agent, status });
  const [outgoing, setOutgoing] = useState<typeof last.current | null>(null);

  useLayoutEffect(() => {
    if (last.current.agent !== agent) setOutgoing(last.current);
    last.current = { agent, status };
  }, [agent, status]);

  useEffect(() => {
    if (!outgoing) return;
    const timer = window.setTimeout(() => setOutgoing(null), MASCOT_FADE_MS);
    return () => window.clearTimeout(timer);
  }, [outgoing]);

  const fading = outgoing && outgoing.agent !== agent;
  return (
    <span className="island-compact-mascot" data-agent={agent}>
      {fading && (
        <span key={outgoing.agent} data-fade="out" aria-hidden="true">
          <Mascot agent={outgoing.agent} status={outgoing.status} />
        </span>
      )}
      <span key={agent} data-fade={fading ? "in" : undefined}>
        <Mascot agent={agent} status={status} />
      </span>
    </span>
  );
}

export function CollapsedStrip({
  idleHovered = false,
  active = true,
  onQuotaWidth,
}: {
  idleHovered?: boolean;
  active?: boolean;
  onQuotaWidth?: (width: number) => void;
}) {
  const { t } = useTranslation();
  const ui = useIslandStore((state) => state.ui);
  const planQuota = useIslandStore((state) => state.claudePlanQuota);
  const store = ui?.store;
  const codexSubsessionMode = ui?.settings.codex_subsession_mode;
  const { sessions } = useMemo(() => presentedSessions(store, codexSubsessionMode), [store, codexSubsessionMode]);
  const { current, activeCount } = useCompactSession(
    sessions, ui?.store.current_session_id, ui?.settings.rotation_interval ?? DEFAULT_SETTINGS.rotation_interval,
  );
  const status = current && activeCount > 0 ? statusKeyFor(current) : "idle";
  const activeTool = current?.active_tool?.trim() || null;
  const activeDetail = activeTool ? current?.active_tool_detail?.trim() || null : null;
  const sessionId = current?.session_id ?? null;
  const [tool, setTool] = useState({ sessionId, name: activeTool, detail: activeDetail });

  useEffect(() => {
    if (activeTool || tool.sessionId !== sessionId) {
      setTool({ sessionId, name: activeTool, detail: activeDetail });
    } else if (tool.name) {
      const timer = window.setTimeout(() => setTool({ sessionId, name: null, detail: null }), 2000);
      return () => window.clearTimeout(timer);
    }
  }, [activeTool, activeDetail, sessionId, tool.name, tool.sessionId]);

  const shownTool = activeTool ?? (tool.sessionId === sessionId ? tool.name : null);
  const liveOutput = active && ui?.settings.show_tool_status && status !== "idle" &&
    !shownTool && current?.agent === "codex" ? current.live_codex_output : null;
  const output = useMemo(() => liveOutput ? compactMarkdownPreview(liveOutput) : "", [liveOutput]);

  if (!ui) return null;

  const settings = ui.settings;
  // The native command chooses the visible head or reopens a dismissed question.
  const pendingQuestion = ui.surface?.kind !== "question_card" &&
    ui.store.pending.some((request) => request.kind === "question");
  const waitingCount = sessions.filter((session) => isWaitingStatus(session.status)).length;
  const agent = status === "idle" ? settings.default_mascot : current?.agent ?? settings.default_mascot;
  const detail = activeTool ? activeDetail : tool.sessionId === sessionId ? tool.detail : null;
  const toolLabel = shownTool
    ? compactToolName(shownTool)
    : !output && status === "thinking" ? t("status.thinkingActivity") : null;
  const tone = shownTool ? toolTone(shownTool) : "thinking";
  const context = settings.show_project_name
    ? pathLeaf(current?.cwd) ?? current?.project?.trim()
    : current?.title?.trim();
  const quiet = settings.sound_enabled && settings.quiet_hours_enabled &&
    (ui.quiet_hours_active ?? inQuietHours(settings.quiet_hours_start, settings.quiet_hours_end));

  if (sessions.length === 0) {
    return (
      <div className="collapsed-strip" data-idle="true" data-idle-hovered={idleHovered}>
        <div className="island-compact-start">
          <Mascot status="idle" agent={settings.default_mascot} />
        </div>
        <span className="footer-spacer" />
        <div className="island-compact-end" inert={!active || !idleHovered}>
          <IslandHeaderControls active={active && idleHovered} showIdleCount />
        </div>
      </div>
    );
  }

  const content = (
    <>
      <div className="island-compact-start">
        <CompactMascot status={status} agent={agent} />
        {settings.show_claude_quota && planQuota?.snapshot && settings.claude_quota_chip !== "off" && (
          <ClaudeQuotaChip
            state={planQuota}
            mode={settings.claude_quota_chip}
            showsPace={settings.notch_height_mode === "custom" && settings.custom_notch_height >= 30}
            onWidth={onQuotaWidth}
          />
        )}
      </div>
      {settings.show_tool_status && (
        <div className="island-compact-activity">
          {status !== "idle" && context && (
            <span className="island-compact-context" title={context}>{context}</span>
          )}
          {toolLabel && (
            <span className="island-compact-tool" data-tone={tone} title={shownTool ?? undefined}>
              {toolLabel}
              <span className="island-compact-tool-sweep" aria-hidden="true" data-label={toolLabel} />
            </span>
          )}
          {shownTool && detail && (
            <CompactMorphText
              className="island-compact-detail"
              title={detail}
              text={pathLeaf(detail) ?? detail}
              streamsRapidly={agent === "aiwork" || agent === "aiwork-cli"}
            />
          )}
          {output && (
            <>
              <span className="island-compact-output-glyph" aria-hidden="true">$</span>
              <CompactMorphText className="island-compact-output" title={output} text={output} />
            </>
          )}
        </div>
      )}
      <span className="footer-spacer" />
      <div className="island-compact-end" data-detailed={settings.show_tool_status}>
        {quiet && (
          <span className="island-quiet-badge" title={t("settings.sound.quietHours")}>
            <MoonIcon />
          </span>
        )}
        {ui.glance_completion_active && <span className="island-glance-dot" />}
        {pendingQuestion ? (
          <span
            key={ui.follow_up_hint_pulse}
            className="island-waiting-badge"
            data-kind="question"
            data-pulse="true"
            data-follow-up-bounce={Boolean(ui.follow_up_hint_pulse)}
            title={t("request.questionWaiting")}
          >
            <QuestionBadgeIcon />
          </span>
        ) : ui.follow_up_hint_active ? (
          <span
            key={ui.follow_up_hint_pulse}
            className="island-waiting-badge island-follow-up-badge"
            data-kind="follow-up"
            data-follow-up-bounce="true"
            title={t("request.followUpHint")}
          >
            <BellIcon />
          </span>
        ) : waitingCount > 0 ? (
          <span className="island-waiting-badge" data-kind="waiting" data-pulse="true" title={t("header.waiting", { count: waitingCount })}>
            <BellIcon />
          </span>
        ) : null}
        <span className="island-compact-count" aria-label={t("footer.sessions", { count: sessions.length })}>
          {activeCount > 0 && (
            <>
              <span className="island-active-count">{activeCount}</span>
              <span className="island-count-divider">/</span>
            </>
          )}
          {sessions.length}
        </span>
      </div>
    </>
  );

  if (pendingQuestion) {
    return (
      <button
        type="button"
        className="collapsed-strip"
        aria-label={t("request.questionWaiting")}
        onClick={() => void openPendingQuestion()}
      >
        {content}
      </button>
    );
  }

  return (
    <div
      className="collapsed-strip"
      title={settings.show_session_recap ? current?.recap ?? undefined : undefined}
    >
      {content}
    </div>
  );
}
