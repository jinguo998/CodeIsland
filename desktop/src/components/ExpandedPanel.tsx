import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { UNLIMITED_VISIBLE_SESSIONS } from "../lib/defaults";
import { presentedSessions, retainedIdleSessionGroups } from "../lib/sessions";
import { isNeedsYou, statusKeyFor, statusLabelKey, type StatusKey } from "../lib/status";
import { useIslandStore } from "../lib/store";
import type { IslandSurfaceState, PendingRequest, SessionSnapshot } from "../lib/types";
import { useNow } from "../lib/useNow";
import { ackShortcutCard, openPendingQuestion, quotaSnapshot, setIslandSurface, type QuotaSnapshot } from "../lib/ipc";
import { AppLogo } from "./AppLogo";
import { ClaudeQuotaFooter, ClaudeUsageFooter } from "./ClaudeQuota";
import { CliIcon } from "./CliIcon";
import { CompletionCard } from "./CompletionCard";
import { IdleSessionRemovalBanner } from "./IdleSessionRemovalBanner";
import { IslandHeaderControls } from "./IslandHeaderControls";
import { PixelButton } from "./PixelButton";
import { RequestCard } from "./RequestCard";
import { SessionCard, sessionHeadline } from "./SessionCard";

type Tab = "all" | "status" | "agent";

interface SessionGroup {
  key: string;
  label?: string;
  source?: string;
  sessions: SessionSnapshot[];
}

const STATUS_SECTIONS = ["waiting", "running", "processing", "idle"] as const;
const CARD_STATUSES: StatusKey[] = ["needs_you", "working", "thinking", "done", "idle", "stopped", "error"];
type StatusSection = typeof STATUS_SECTIONS[number];

const CLI_SECTIONS = [
  ["claude", "Claude"], ["codex", "Codex"], ["gemini", "Gemini"],
  ["antigravity", "AntiGravity"], ["google-antigravity", "Google Antigravity"],
  ["cursor", "Cursor"], ["trae", "Trae"], ["traecn", "Trae CN"], ["traecli", "Trae CLI"],
  ["copilot", "Copilot"], ["qoder", "Qoder"], ["qoderwork", "QoderWork"], ["droid", "Factory"],
  ["codebuddy", "CodeBuddy"], ["codybuddycn", "CodyBuddyCN"], ["stepfun", "StepFun"],
  ["workbuddy", "WorkBuddy"], ["hermes", "Hermes"], ["openclaw", "OpenClaw"],
  ["grok", "Grok CLI"], ["qwen", "Qwen Code"], ["kimi", "Kimi Code CLI"], ["opencode", "OpenCode"],
  ["mimo", "MiMo"], ["pi", "Pi"], ["kiro", "Kiro"], ["cline", "Cline"], ["zcode", "ZCode"],
  ["minimax", "MiniMax Code CLI"], ["deepseek", "DeepSeek Harness"], ["aiwork", "AiWork"], ["aiwork-cli", "AiWork CLI"],
] as const;

function statusSection(session: SessionSnapshot): StatusSection {
  if (isNeedsYou(session)) return "waiting";
  const status = statusKeyFor(session);
  if (status === "working") return "running";
  if (status === "thinking") return "processing";
  return "idle";
}

function groupTab(value: string | undefined): Tab {
  return value === "status" ? "status" : value === "cli" ? "agent" : "all";
}

type ExpandedSurface = Exclude<IslandSurfaceState, { kind: "collapsed" }>;

export function ExpandedPanel({
  surface = { kind: "session_list" },
  active = true,
}: {
  surface?: ExpandedSurface;
  active?: boolean;
}) {
  const { t } = useTranslation();
  const ui = useIslandStore((state) => state.ui);
  const planQuota = useIslandStore((state) => state.claudePlanQuota);
  const updateSettings = useIslandStore((state) => state.updateSettings);
  const removeSession = useIslandStore((state) => state.removeSession);
  const removeIdleSessions = useIslandStore((state) => state.removeIdleSessions);
  const moveSession = useIslandStore((state) => state.moveSession);
  const openWorkspace = useIslandStore((state) => state.openWorkspace);
  const now = useNow(1000);
  const shellRef = useRef<HTMLElement>(null);

  const [dragId, setDragId] = useState<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const [quota, setQuota] = useState<QuotaSnapshot | null>(null);
  const [frozen, setFrozen] = useState<{
    ids: string[];
    sections: Map<string, StatusSection>;
  } | null>(null);
  const thawTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!active || surface.kind !== "session_list") return;
    if (!ui?.settings.show_usage_stats) {
      setQuota(null);
      return;
    }
    let cancelled = false;
    void quotaSnapshot().then((snapshot) => {
      if (!cancelled) setQuota(snapshot);
    });
    return () => {
      cancelled = true;
    };
  }, [active, surface.kind, ui?.settings.show_usage_stats]);
  const groupBy = useIslandStore((state) => state.settingsSaveStatus === "saving"
    ? state.settingsDraft.group_by ?? state.ui?.settings.group_by
    : state.ui?.settings.group_by);
  const tab = groupTab(groupBy);

  const [requestedIdleGroups, setRequestedIdleGroups] = useState<string[][] | null>(null);
  const [removingIdleSessions, setRemovingIdleSessions] = useState(false);
  const idleGroups = active && surface.kind === "session_list" ? ui?.idle_session_removal_groups : undefined;
  const markedIdleIds = new Set(
    requestedIdleGroups && idleGroups ? retainedIdleSessionGroups(requestedIdleGroups, idleGroups).flat() : [],
  );
  const confirmIdleRemoval = () => {
    if (!requestedIdleGroups || !markedIdleIds.size || removingIdleSessions) return;
    setRequestedIdleGroups(null);
    setRemovingIdleSessions(true);
    void removeIdleSessions(requestedIdleGroups).finally(() => setRemovingIdleSessions(false));
  };
  useEffect(() => {
    // A fully invalidated confirmation stays closed when those sessions go idle again.
    if (requestedIdleGroups && !markedIdleIds.size) setRequestedIdleGroups(null);
  }, [requestedIdleGroups, markedIdleIds.size]);

  const store = ui?.store;
  const codexSubsessionMode = ui?.settings.codex_subsession_mode;
  const { sessions, requests } = useMemo(() => presentedSessions(store, codexSubsessionMode), [store, codexSubsessionMode]);
  const pending = requests.filter((request) => !request.hidden);
  const maxVisible = ui?.settings.max_visible_sessions ?? 5;
  const compact = ui?.settings.density === "compact";
  const fontSize = ui?.settings.content_font_size ?? 13;
  const projectNames = compact
    ? sessions.map((session) => sessionHeadline(session, ui?.settings.show_project_name ?? true)).join("\0")
    : "";

  useLayoutEffect(() => {
    const shell = shellRef.current;
    if (!compact || !shell || typeof CanvasRenderingContext2D === "undefined") return;
    const context = document.createElement("canvas").getContext("2d");
    if (!context) return;

    let disposed = false;
    const measure = () => {
      if (disposed) return;
      const family = getComputedStyle(shell).fontFamily;
      context.font = `700 ${fontSize}px ${family}`;
      const projectWidth = Math.max(...projectNames.split("\0").map((label) => context.measureText(label).width));
      const cap = Math.min(160 * fontSize / 11, 180);
      context.font = `700 ${Math.max(8.5, fontSize - 2)}px ${family}`;
      const statusWidth = Math.max(...CARD_STATUSES.map((status) => context.measureText(t(statusLabelKey(status))).width));
      shell.style.setProperty("--compact-project-width", `${Math.min(cap, Math.ceil(projectWidth + 2))}px`);
      shell.style.setProperty("--compact-status-width", `${Math.ceil(statusWidth + 2)}px`);
    };
    measure();
    void document.fonts?.ready.then(measure);
    return () => {
      disposed = true;
    };
  }, [compact, fontSize, projectNames, t]);
  const shellStyle = {
    "--content-font-size": `${fontSize}px`,
    "--ai-message-lines": ui?.settings.ai_message_lines === 0 ? "none" : String(ui?.settings.ai_message_lines ?? 2),
    "--island-session-budget": maxVisible === UNLIMITED_VISIBLE_SESSIONS ? "100vh" : `${maxVisible * 90}px`,
  } as CSSProperties;

  // The order is held while the pointer is over the list so a card never
  // moves out from under the cursor when its status changes.
  const orderedSessions = useMemo(() => {
    if (!frozen) return sessions;
    const byId = new Map(sessions.map((session) => [session.session_id, session]));
    const kept = frozen.ids
      .map((id) => byId.get(id))
      .filter((session): session is SessionSnapshot => Boolean(session));
    const keptIds = new Set(kept.map((session) => session.session_id));
    return [...kept, ...sessions.filter((session) => !keptIds.has(session.session_id))];
  }, [sessions, frozen]);

  const freezeOrder = () => {
    if (thawTimer.current !== null) window.clearTimeout(thawTimer.current);
    setFrozen((previous) => previous ?? {
      ids: sessions.map((session) => session.session_id),
      sections: new Map(sessions.map((session) => [session.session_id, statusSection(session)])),
    });
  };
  const thawOrderSoon = () => {
    if (thawTimer.current !== null) window.clearTimeout(thawTimer.current);
    thawTimer.current = window.setTimeout(() => {
      thawTimer.current = null;
      setFrozen(null);
    }, 250);
  };

  const sessionIds = new Set(orderedSessions.map((session) => session.session_id));
  // Hiding dismisses the floating card. The permission queue stays
  // available through its session list row, including the hidden head.
  const permissions = requests.filter((request) => request.kind === "permission");
  const questions = requests.filter((request) => request.kind === "question");
  const orphanRequests = pending.filter((request) => !sessionIds.has(request.session_id));

  useEffect(() => () => {
    if (thawTimer.current !== null) window.clearTimeout(thawTimer.current);
  }, []);

  const groups = useMemo<SessionGroup[]>(() => {
    if (tab === "all") return [{ key: "all", sessions: orderedSessions }];
    if (tab === "status") {
      return STATUS_SECTIONS.flatMap((key) => {
        const members = orderedSessions.filter(
          (session) => (frozen?.sections.get(session.session_id) ?? statusSection(session)) === key,
        );
        return members.length ? [{ key, label: t(`groups.${key}`), sessions: members }] : [];
      });
    }

    const known = new Set<string>(CLI_SECTIONS.map(([key]) => key));
    const result: SessionGroup[] = CLI_SECTIONS.flatMap(([key, label]) => {
      const members = orderedSessions.filter((session) => session.agent === key);
      return members.length ? [{ key, label, source: key, sessions: members }] : [];
    });
    const other = orderedSessions.filter((session) => !known.has(session.agent));
    if (other.length) result.push({ key: "other", label: t("request.other"), sessions: other });
    return result;
  }, [tab, orderedSessions, frozen, t]);

  const handleDrop = (targetId: string) => {
    const source = dragId;
    setDragId(null);
    setDragOverId(null);
    if (!source || source === targetId) return;
    void moveSession(source, targetId);
  };

  const openQuestion = (request: PendingRequest) => {
    void openPendingQuestion(request.event_id);
  };

  const questionRow = (question: PendingRequest) => (
    <div className="island-question-row">
      <span>{t("request.questionWaiting")}</span>
      <PixelButton variant="primary" onClick={() => openQuestion(question)}>{t("request.answer")}</PixelButton>
    </div>
  );

  const renderSession = (session: SessionSnapshot) => {
    const permission = permissions.find((request) => request.session_id === session.session_id);
    const sessionQuestions = questions.filter((request) => request.session_id === session.session_id);
    const question = sessionQuestions.find((request) => !request.hidden) ?? sessionQuestions[0];
    return (
      <SessionCard
        key={session.session_id}
        session={session}
        now={now}
        markedForRemoval={markedIdleIds.has(session.session_id)}
        dragging={dragId === session.session_id}
        dragOver={dragOverId === session.session_id && dragId !== session.session_id}
        onDragStart={() => setDragId(session.session_id)}
        onDragEnd={() => {
          setDragId(null);
          setDragOverId(null);
        }}
        onDragOver={() => {
          if (dragId) setDragOverId(session.session_id);
        }}
        onDrop={() => handleDrop(session.session_id)}
        onRemove={() => void removeSession(session.session_id)}
      >
        {permission ? (
          <RequestCard
            key={permission.event_id}
            request={permission}
            standalone={false}
            position={permissions.indexOf(permission) + 1}
            total={permissions.length}
          />
        ) : question ? questionRow(question) : undefined}
      </SessionCard>
    );
  };

  const shownRequest =
    surface.kind === "approval_card" || surface.kind === "question_card"
      ? pending.find((request) => request.event_id === surface.event_id)
      : undefined;
  const shownCompletion =
    surface.kind === "completion_card"
      ? sessions.find((session) => session.session_id === surface.session_id)
      : undefined;
  const [lastRequest, setLastRequest] = useState<PendingRequest | undefined>(shownRequest);
  const [lastCompletion, setLastCompletion] = useState<SessionSnapshot | undefined>(shownCompletion);
  useEffect(() => {
    if (active && shownRequest) setLastRequest(shownRequest);
    if (active && shownCompletion) setLastCompletion(shownCompletion);
  }, [active, shownRequest, shownCompletion]);
  const request = active ? shownRequest : (shownRequest ?? lastRequest);
  const completion = active ? shownCompletion : (shownCompletion ?? lastCompletion);

  const requestContainer = useRef<HTMLDivElement>(null);
  const revision = ui?.surface_revision;
  useEffect(() => {
    const element = requestContainer.current;
    if (!active || !shownRequest || revision === undefined || !element) return;
    const acknowledge = () => void ackShortcutCard(shownRequest.event_id, revision);
    if (typeof IntersectionObserver === "undefined") {
      const timer = window.setTimeout(acknowledge, 420);
      return () => window.clearTimeout(timer);
    }
    const observer = new IntersectionObserver(([entry]) => {
      if (!entry?.isIntersecting || entry.intersectionRatio < 0.5) return;
      observer.disconnect();
      acknowledge();
    }, { threshold: 0.5 });
    observer.observe(element);
    return () => observer.disconnect();
  }, [active, revision, shownRequest?.event_id]);

  const expandTimer = useRef<number | null>(null);
  const cancelExpand = () => {
    if (expandTimer.current === null) return;
    window.clearTimeout(expandTimer.current);
    expandTimer.current = null;
  };
  useEffect(() => cancelExpand, [active, surface.kind]);
  const showAllSessions = () => {
    cancelExpand();
    void setIslandSurface({ kind: "session_list" });
  };

  // The macOS header draws these in its 5x7 Latin-only pixel font, so the
  // words stay ALL / STATUS / AGENT in every language (tooltips translate).
  const tabs: Array<{ id: Tab; label: string; titleKey: string }> = [
    { id: "all", label: "ALL", titleKey: "tabs.all" },
    { id: "status", label: "STATUS", titleKey: "tabs.status" },
    { id: "agent", label: "AGENT", titleKey: "tabs.agent" },
  ];

  const selectTab = (next: Tab) => {
    if (ui) {
      void updateSettings({ group_by: next === "agent" ? "cli" : next });
    }
  };

  return (
    <section
      ref={shellRef}
      className="island-shell"
      aria-label={t("a11y.islandLabel")}
      style={shellStyle}
      onKeyDown={(event) => {
        if (event.key === "Escape" && markedIdleIds.size) {
          event.preventDefault();
          setRequestedIdleGroups(null);
        }
      }}
    >
      <header className="island-header">
        <div className="island-header-start">
          <AppLogo />
          {sessions.length > 1 && (
            <nav className="island-tabs" role="tablist" aria-label={t("tabs.label")}>
              {tabs.map((entry) => (
                <button
                  key={entry.id}
                  type="button"
                  role="tab"
                  aria-selected={tab === entry.id}
                  data-active={tab === entry.id}
                  className="island-tab"
                  aria-label={t(entry.titleKey)}
                  title={t(entry.titleKey)}
                  onClick={() => selectTab(entry.id)}
                >
                  <span>{entry.label}</span>
                </button>
              ))}
            </nav>
          )}
        </div>
        <span className="footer-spacer" />
        <IslandHeaderControls
          active={active}
          idleRemoval={idleGroups?.length ? {
            confirming: markedIdleIds.size > 0,
            busy: removingIdleSessions,
            toggle: () => setRequestedIdleGroups(markedIdleIds.size ? null : idleGroups),
          } : undefined}
        />
      </header>
      <div className="island-panel-body" data-surface={surface.kind}>
        <div className="island-divider" />
        {surface.kind === "session_list" && (
          <>
            <div className="island-session-list">
              <IdleSessionRemovalBanner
                count={markedIdleIds.size}
                onCancel={() => setRequestedIdleGroups(null)}
                onRemove={confirmIdleRemoval}
              />
              <div
                className="island-scroll"
                onMouseEnter={freezeOrder}
                onMouseLeave={thawOrderSoon}
              >
                {orphanRequests.map((item) => item.kind === "question"
                  ? <div key={item.event_id} className="request-card">{questionRow(item)}</div>
                  : <RequestCard
                    key={item.event_id}
                    request={item}
                    standalone={false}
                    position={permissions.indexOf(item) + 1}
                    total={permissions.length}
                  />,
                )}
                {orderedSessions.length === 0 && orphanRequests.length === 0 && (
                  <div className="px-4 py-8 text-center">
                    <div className="text-[12px] font-bold text-muted">
                      {t("session.noSessions")}
                    </div>
                    <div className="mt-1 text-[11px] text-dim">
                      {t("session.noSessionsHint")}
                    </div>
                  </div>
                )}
                {groups.map((group) => (
                  <div key={group.key}>
                    {group.label && (
                      <div className="list-section">
                        {group.source && <CliIcon source={group.source} />}
                        <span>{group.label} ({group.sessions.length})</span>
                      </div>
                    )}
                    {group.sessions.map(renderSession)}
                  </div>
                ))}
              </div>
            </div>
            {quota && !(ui?.settings.show_claude_quota && planQuota?.snapshot) && (
              <ClaudeUsageFooter usage={quota} />
            )}
            {ui?.settings.show_claude_quota && planQuota && (
              <ClaudeQuotaFooter
                state={planQuota}
                usage={quota}
              />
            )}
          </>
        )}
        {(surface.kind === "approval_card" || surface.kind === "question_card") && request && (
          <div ref={requestContainer} className="island-focused-request">
            <RequestCard
              key={request.event_id}
              request={request}
              position={Math.max(0, pending.filter((item) => item.kind === request.kind)
                .findIndex((item) => item.event_id === request.event_id)) + 1}
              total={pending.filter((item) => item.kind === request.kind).length}
            />
          </div>
        )}
        {surface.kind === "completion_card" && completion && (
          <>
            <div className="island-focused-completion">
              <CompletionCard
                session={completion}
                now={now}
                onOpenWorkspace={completion.session_id.startsWith("ssh:")
                  ? undefined
                  : () => void openWorkspace(completion.session_id)}
                onRemove={() => void removeSession(completion.session_id)}
              />
            </div>
            {sessions.length > 1 && (
              <button
                type="button"
                className="island-show-sessions"
                onClick={showAllSessions}
                onMouseEnter={() => {
                  cancelExpand();
                  expandTimer.current = window.setTimeout(showAllSessions, 600);
                }}
                onMouseLeave={cancelExpand}
              >
                {t("footer.sessions", { count: sessions.length })}
              </button>
            )}
          </>
        )}
      </div>
    </section>
  );
}
