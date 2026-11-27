use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use crate::approval::{ApprovalAlwaysScope, agent_keeps_always_rule, command_for_event, commands_match};
use crate::ordering;
use crate::session_metadata::{TranscriptMetadata, apply_checklist, hook_checklist};
use crate::settings::DEFAULT_MAX_HISTORY;
use crate::types::*;

const SEEN_EVENT_CAPACITY: usize = 512;
const IDLE_REMOVAL_CAPACITY: usize = 1024;

#[derive(Debug)]
struct RemovedIdleGroup {
    at: DateTime<Utc>,
    members: Vec<String>,
}

pub struct RequestResolution {
    pub decision: Decision,
    pub approval_to_save: Option<PendingRequest>,
    pub covered: Vec<Decision>,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions: IndexMap<String, SessionSnapshot>,
    pending: IndexMap<String, PendingRequest>,
    removed_idle: IndexMap<String, Arc<RemovedIdleGroup>>,
    tool_uses: HashMap<String, (Option<String>, Option<String>)>,
    manual_order: Vec<String>,
    seen_events: VecDeque<String>,
    seen_set: HashSet<String>,
    last_seq: HashMap<String, u64>,
    _max_visible: usize,
    history_limit: usize,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new(8, DEFAULT_MAX_HISTORY)
    }
}

impl SessionStore {
    pub fn new(max_visible: usize, history_limit: usize) -> Self {
        Self {
            sessions: IndexMap::new(),
            pending: IndexMap::new(),
            removed_idle: IndexMap::new(),
            tool_uses: HashMap::new(),
            manual_order: Vec::new(),
            seen_events: VecDeque::new(),
            seen_set: HashSet::new(),
            last_seq: HashMap::new(),
            _max_visible: max_visible.max(1),
            history_limit: history_limit.max(1),
        }
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn sessions(&self) -> &IndexMap<String, SessionSnapshot> {
        &self.sessions
    }

    /// Apply process evidence to the matching WSL transcript without
    /// assigning a Linux PID to the Windows process monitor.
    pub fn reconcile_wsl_process(
        &mut self,
        id: &str,
        transcript: Option<&str>,
        process: Option<WslProcess>,
        status: Option<(SessionStatus, DateTime<Utc>)>,
        exited: bool,
    ) -> bool {
        let pending = self.pending.values().any(|request| request.session_id == id);
        let Some(session) = self.sessions.get_mut(id).filter(|session| {
            session.transcript_path.as_deref() == transcript
        }) else { return false };
        let mut changed = session.wsl_process != process;
        if process.is_some() && session.pid.take().is_some() {
            session.discovered_pid_started_at = None;
            changed = true;
        }
        session.wsl_process = process;
        if pending || session.status.needs_you() {
            return changed;
        }
        let next = if exited && session.status.is_active() {
            Some((SessionStatus::Idle, session.last_activity))
        } else {
            status.filter(|(next, timestamp)| {
                *timestamp >= session.last_activity
                    && (*next != SessionStatus::Idle || session.status.is_active())
            })
        };
        if let Some((next, timestamp)) = next {
            changed |= session.status != next || session.last_activity != timestamp;
            session.status = next;
            session.last_activity = timestamp;
            if !next.is_active() {
                session.active_tool = None;
                session.active_tool_detail = None;
                session.live_codex_output = None;
                session.tool_use_id = None;
                session.is_executing_tool = false;
            }
        }
        changed
    }

    pub fn pending(&self) -> &IndexMap<String, PendingRequest> {
        &self.pending
    }

    pub fn manual_order(&self) -> &[String] {
        &self.manual_order
    }

    pub fn get(&self, session_id: &str) -> Option<&SessionSnapshot> {
        self.sessions.get(session_id)
    }

    pub fn get_pending(&self, event_id: &str) -> Option<&PendingRequest> {
        self.pending.get(event_id)
    }

    pub fn pending_for(&self, session_id: &str) -> Vec<&PendingRequest> {
        self.pending
            .values()
            .filter(|request| request.session_id == session_id)
            .collect()
    }

    /// Fills a Codex relationship discovered in a rollout header without
    /// replaying activity or overwriting a more recent transcript association.
    pub fn link_codex_rollout_parent(&mut self, child_id: &str, parent_id: &str, path: &str, title: Option<&str>) -> bool {
        if child_id == parent_id {
            return false;
        }
        let Some(child) = self.sessions.get_mut(child_id).filter(|child| {
            child.agent == AgentKind::Codex
                && child.parent_session_id.as_deref().is_none_or(|parent| parent == parent_id)
                && child.transcript_path.as_deref().is_none_or(|stored| {
                    crate::discovery::same_transcript_path(stored, path)
                })
        }) else {
            return false;
        };
        let mut changed = child.parent_session_id.is_none();
        child.parent_session_id.get_or_insert_with(|| parent_id.to_owned());
        changed |= replace_discovered(&mut child.title, title, false);
        self.attach_child(parent_id, child_id) || changed
    }

    /// Reconciles a freshly attached Codex Desktop rollout's latest turn
    /// marker without synthesizing a prompt, completion or hook.
    pub fn restore_codex_rollout_turn(
        &mut self,
        session_id: &str,
        path: &str,
        modified_at: DateTime<Utc>,
        running: bool,
    ) -> bool {
        if self.pending.values().any(|request| request.session_id == session_id) {
            return false;
        }
        let Some(session) = self.sessions.get_mut(session_id).filter(|session| {
            session.agent == AgentKind::Codex
                && session.last_activity <= modified_at
                && session.transcript_path.as_deref().is_some_and(|current| {
                    crate::discovery::same_transcript_path(current, path)
                })
        }) else {
            return false;
        };
        session.status = match (running, session.status) {
            (true, SessionStatus::Idle) => SessionStatus::Running,
            (false, status) if status.is_active() => SessionStatus::Idle,
            _ => return false,
        };
        session.last_activity = modified_at;
        session.active_tool = None;
        session.active_tool_detail = None;
        session.tool_use_id = None;
        session.is_executing_tool = false;
        session.live_codex_output = None;
        if running {
            session.interrupted = false;
            session.last_turn_failed = false;
            session.error = None;
        }
        true
    }

    /// Applies transcript presentation data without changing activity or phase.
    pub fn apply_transcript_metadata(&mut self, event: &AgentEvent) -> bool {
        let Some(session) = self.sessions.get_mut(&event.session_id).filter(|session| {
            session.agent == event.agent
                && session.transcript_path.as_deref().zip(event.transcript_path.as_deref())
                    .is_none_or(|(current, incoming)| crate::discovery::same_transcript_path(current, incoming))
        }) else {
            return false;
        };
        apply_transcript_metadata(session, event)
    }

    /// Merge recovery without changing a newer hook/app-server state or
    /// producing attention. The caller supplies the platform liveness check.
    pub fn merge_discovered(
        &mut self,
        info: &crate::discovery::DiscoveredSession,
        pid_alive: impl Fn(u32, Option<DateTime<Utc>>) -> bool,
    ) -> bool {
        use crate::discovery::same_transcript_path;

        let id = &info.session_id;
        let desktop = id.starts_with(crate::CODEX_DESKTOP_SESSION_PREFIX);
        let matches_provider = |session: &SessionSnapshot| {
            session.agent == info.agent
                && !session.session_id.starts_with("ssh:")
                && !session.session_id.starts_with(crate::cowork::PREFIX)
                && desktop == session.session_id.starts_with(crate::CODEX_DESKTOP_SESSION_PREFIX)
                && session.provider_session_id.as_ref().is_none_or(|provider_id| provider_id == &info.provider_session_id)
        };
        let key = if self.sessions.contains_key(id) {
            id.clone()
        } else {
            self.sessions.values().find(|session| {
                matches_provider(session)
                    && session.transcript_path.as_deref().is_some_and(|path| same_transcript_path(path, &info.transcript_path))
            }).map_or_else(|| id.clone(), |session| session.session_id.clone())
        };
        if self.sessions.get(&key).is_some_and(|session| !matches_provider(session)) {
            return false;
        }
        let has_pending = self.pending.values().any(|request| request.session_id == key);
        if info.finished_subagent {
            return !has_pending
                && self.sessions.get(&key).is_some_and(|session| {
                    info.modified_at >= session.last_activity && !session.status.needs_you()
                })
                && self.remove_session(&key);
        }
        let parent = info.parent_session_id.as_deref()
            .filter(|parent| self.sessions.contains_key(*parent) && *parent != key)
            .map(ToOwned::to_owned);
        if info.parent_session_id.is_some() && parent.is_none() && !self.sessions.contains_key(&key) {
            return false;
        }
        let mut evidence = AgentEvent::new(
            &key,
            info.agent,
            if info.status.is_active() { AgentEventType::UserPrompt } else { AgentEventType::Heartbeat },
        );
        evidence.timestamp = info.modified_at;
        if !self.accept_removed_idle_event(&evidence) {
            return false;
        }

        let mut changed = !self.sessions.contains_key(&key);
        if desktop && id == &key {
            let raw_id = &info.provider_session_id;
            let raw_is_duplicate = self.sessions.get(raw_id).is_some_and(|raw| {
                raw.agent == info.agent && raw.pid.is_none() && raw.provider_session_id.is_none()
                    && raw.terminal_kind.as_deref().is_none_or(|kind| kind == "codex")
                    && raw.transcript_path.as_deref().is_some_and(|path| same_transcript_path(path, &info.transcript_path))
                    && !self.pending.values().any(|request| request.session_id == *raw_id)
            });
            if raw_is_duplicate {
                changed |= self.remove_session(raw_id);
            }
        }


        let session = self.sessions.entry(key.clone()).or_insert_with(|| {
            evidence.timestamp = info.started_at.unwrap_or(info.modified_at);
            SessionSnapshot::new(&evidence)
        });
        let freshest = info.modified_at >= session.last_activity;
        if info.modified_at > session.last_activity {
            session.last_activity = info.modified_at;
            changed = true;
        }
        let preserves_completion = info.status == SessionStatus::Idle
            && matches!(session.status, SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled);
        if freshest && !has_pending && !session.status.needs_you() && !preserves_completion && session.status != info.status {
            session.status = info.status;
            if !info.status.is_active() {
                session.active_tool = None;
                session.active_tool_detail = None;
                session.is_executing_tool = false;
                if !info.status.needs_you() {
                    session.live_codex_output = None;
                }
            }
            changed = true;
        }
        if let Some(pid) = info.pid
            && session.pid.is_none_or(|current| {
                current == pid || !pid_alive(current, session.discovered_pid_started_at)
            })
            && (session.pid != Some(pid) || session.discovered_pid_started_at != info.started_at)
        {
            session.pid = Some(pid);
            session.discovered_pid_started_at = info.started_at;
            changed = true;
        }
        changed |= replace_discovered(&mut session.provider_session_id, Some(&info.provider_session_id), false);
        changed |= replace_discovered(
            &mut session.claude_desktop_session_id, info.claude_desktop_session_id.as_deref(), freshest,
        );
        changed |= replace_discovered(&mut session.title, info.title.as_deref(), freshest);
        changed |= replace_discovered(&mut session.cwd, info.cwd.as_deref(), freshest);
        let project = project_name(session.cwd.as_deref());
        changed |= replace_discovered(&mut session.project, project.as_deref(), freshest);
        changed |= replace_discovered(&mut session.git_branch, info.git_branch.as_deref(), freshest);
        changed |= replace_discovered(&mut session.model, info.model.as_deref(), freshest);
        changed |= replace_discovered(&mut session.reasoning, info.reasoning.as_deref(), freshest);
        changed |= replace_discovered(&mut session.last_prompt, info.last_prompt.as_deref(), freshest);
        changed |= replace_discovered(&mut session.last_message, info.last_message.as_deref(), freshest);
        changed |= replace_discovered(&mut session.transcript_path, Some(&info.transcript_path), freshest);
        if desktop {
            changed |= replace_discovered(&mut session.terminal_kind, Some("codex"), true);
        } else if session.claude_desktop_session_id.is_some() {
            changed |= replace_discovered(&mut session.terminal_kind, Some("claude"), true);
        }
        if let Some(parent) = parent.as_ref() {
            changed |= replace_discovered(&mut session.parent_session_id, Some(parent), false);
            if let Some(parent) = self.sessions.get_mut(parent)
                && !parent.child_session_ids.contains(&key)
            {
                parent.child_session_ids.push(key);
                changed = true;
            }
        }
        changed
    }

    /// Updates an app-server thread without synthesizing a prompt or hook.
    pub fn observe_codex_desktop_thread(
        &mut self,
        event: &AgentEvent,
        status: Option<SessionStatus>,
        preview: Option<&str>,
    ) -> bool {
        let starts_thread = event.event_type == AgentEventType::SessionStart;
        if starts_thread && !self.accept_removed_idle_event(event) {
            return false;
        }
        let session = if starts_thread {
            self.sessions.entry(event.session_id.clone()).or_insert_with(|| SessionSnapshot::new(event))
        } else if let Some(session) = self.sessions.get_mut(&event.session_id) {
            session
        } else {
            return false;
        };
        session.last_activity = session.last_activity.max(event.timestamp);
        if let Some(cwd) = event.cwd.as_deref().filter(|value| !value.is_empty()) {
            session.cwd = Some(cwd.to_owned());
            session.project = project_name(Some(cwd));
        }
        if let Some(title) = event.title.as_deref().filter(|value| !value.is_empty()) {
            session.title = Some(title.to_owned());
        }
        if let Some(path) = event.transcript_path.as_deref().filter(|value| !value.is_empty()) {
            session.transcript_path = Some(path.to_owned());
        }
        if let Some(preview) = preview.filter(|value| !value.is_empty()) {
            session.last_prompt = Some(preview.to_owned());
        }
        if let Some(status) = status
            && !self.pending.values().any(|request| request.session_id == event.session_id)
        {
            session.status = status;
            if !status.is_active() && !status.needs_you() {
                session.live_codex_output = None;
            }
            if matches!(status, SessionStatus::Idle | SessionStatus::Running) {
                session.active_tool = None;
                session.active_tool_detail = None;
                session.is_executing_tool = false;
            }
        }
        true
    }

    pub fn observe_cowork(&mut self, id: &str, update: &crate::cowork::Update) -> Option<SessionStatus> {
        use crate::cowork::Phase;

        let metadata = &update.metadata;
        let state = &update.audit;
        let is_new = !self.sessions.contains_key(id);
        if is_new && !(update.restore || update.live) {
            return None;
        }
        let active = !update.settle && state.phase != Phase::Idle;
        let mut evidence = AgentEvent::new(
            id, AgentKind::Claude,
            if active && update.live { AgentEventType::UserPrompt } else { AgentEventType::Heartbeat },
        );
        evidence.timestamp = update.last_activity;
        if !self.accept_removed_idle_event(&evidence) {
            return None;
        }
        let session = self.sessions.entry(id.to_owned()).or_insert_with(|| {
            evidence.timestamp = metadata.created_at.unwrap_or(update.last_activity);
            SessionSnapshot::new(&evidence)
        });
        session.terminal_kind = Some("claude".to_owned());
        session.provider_session_id = metadata.cli_id.clone();
        session.last_activity = session.last_activity.max(update.last_activity);
        session.title = metadata.title.clone();
        session.cwd = metadata.host_cwd.clone();
        session.project = project_name(metadata.host_cwd.as_deref());
        session.transcript_path = update.transcript_path.clone();
        if let Some(model) = &update.conversation.model {
            session.model = Some(model.clone());
        } else if session.model.is_none() {
            session.model = metadata.model.clone();
        }
        if let Some(prompt) = update.conversation.prompt.as_ref()
            .or(state.prompt.as_ref()).or(metadata.initial_prompt.as_ref())
        {
            session.last_prompt = Some(prompt.clone());
        }
        if let Some(reply) = update.conversation.response.as_ref().or(state.result.as_ref()) {
            session.last_message = Some(reply.clone());
        }

        if update.settle {
            session.status = SessionStatus::Idle;
            session.active_tool = None;
            session.active_tool_detail = None;
            session.is_executing_tool = false;
        } else if update.live || update.restore {
            session.status = match state.phase {
                Phase::Idle if state.interrupted => SessionStatus::Cancelled,
                Phase::Idle if state.failed => SessionStatus::Failed,
                Phase::Idle if state.end_count > 0 => SessionStatus::Completed,
                Phase::Idle => SessionStatus::Idle,
                Phase::Running => SessionStatus::Running,
                Phase::WaitingApproval => SessionStatus::WaitingPermission,
                Phase::WaitingQuestion => SessionStatus::WaitingAnswer,
            };
            let tool = state.active_permission().or(state.current_tool.as_ref());
            session.active_tool = tool.map(|tool| tool.name.clone());
            session.active_tool_detail = tool.and_then(|tool| tool.detail.clone());
            session.is_executing_tool = state.phase == Phase::Running && state.current_tool.is_some();
            session.interrupted = state.phase == Phase::Idle && state.interrupted;
            session.last_turn_failed = state.phase == Phase::Idle && state.failed;
            session.error = session.last_turn_failed.then(|| state.result.clone()).flatten();
        }
        Some(session.status)
    }

    pub fn remove_cowork_shadowed_by(&mut self, cli_id: &str) -> bool {
        let shadowed: Vec<_> = self.sessions.values()
            .filter(|session| session.session_id.starts_with(crate::cowork::PREFIX)
                && session.provider_session_id.as_deref() == Some(cli_id))
            .map(|session| session.session_id.clone()).collect();
        let removed = !shadowed.is_empty();
        for id in shadowed {
            self.remove_session(&id);
        }
        removed
    }

    pub fn apply(&mut self, event: &AgentEvent) -> ApplyOutcome {
        self.apply_with_approval_scope(event, ApprovalAlwaysScope::Session, None)
    }

    pub fn apply_with_approval_scope(
        &mut self,
        event: &AgentEvent,
        always_scope: ApprovalAlwaysScope,
        approval_config: Option<PathBuf>,
    ) -> ApplyOutcome {
        let session_id = event.session_id.clone();
        if !self.mark_seen(&event.id) || !self.accept_removed_idle_event(event) {
            return ApplyOutcome {
                applied: false,
                session_id,
                is_new_session: false,
                ended_turn: false,
                turn_failed: false,
                request: None,
            };
        }
        if let Some(seq) = event.seq {
            let last = self.last_seq.get(&event.session_id).copied().unwrap_or(0);
            if seq <= last {
                return ApplyOutcome {
                    applied: false,
                    session_id,
                    is_new_session: false,
                    ended_turn: false,
                    turn_failed: false,
                    request: None,
                };
            }
            self.last_seq.insert(event.session_id.clone(), seq);
        }

        if event.event_type == AgentEventType::Heartbeat && event.transcript_metadata.is_some() {
            return ApplyOutcome {
                applied: self.apply_transcript_metadata(event),
                session_id,
                is_new_session: false,
                ended_turn: false,
                turn_failed: false,
                request: None,
            };
        }

        // A CLI that exits takes its card and queued requests with it; it is
        // not a completed turn. remove_session also detaches it from its parent.
        if event.event_type == AgentEventType::SessionEnd {
            self.remove_session(&event.session_id);
            return ApplyOutcome {
                applied: true,
                session_id,
                is_new_session: false,
                ended_turn: false,
                turn_failed: false,
                request: None,
            };
        }

        // Subagent lifecycle events belong to the child, never the parent. A
        // subagent event with no parent (or one that names its own session) is
        // a heartbeat at best: it must not mint a card or reset one.
        if matches!(
            event.event_type,
            AgentEventType::SubagentStart | AgentEventType::SubagentEnd
        ) {
            match event.parent_session_id.as_deref() {
                Some(parent) if parent != event.session_id.as_str() => {}
                _ => {
                    return ApplyOutcome {
                        applied: true,
                        session_id,
                        is_new_session: false,
                        ended_turn: false,
                        turn_failed: false,
                        request: None,
                    };
                }
            }
        }

        let is_new_session = !self.sessions.contains_key(&event.session_id);
        let mut request = None;
        let mut ended_turn = false;
        let mut turn_failed = false;

        if is_new_session {
            let snapshot = SessionSnapshot::new(event);
            self.sessions.insert(event.session_id.clone(), snapshot);
        }

        let snapshot = self
            .sessions
            .get_mut(&event.session_id)
            .expect("session inserted above");
        if event.timestamp > snapshot.last_activity {
            snapshot.last_activity = event.timestamp;
        }
        if snapshot.started_at > event.timestamp {
            snapshot.started_at = event.timestamp;
        }
        if let Some(cwd) = event.cwd.as_deref() {
            snapshot.cwd = Some(cwd.to_owned());
            snapshot.project = project_name(Some(cwd));
        }
        if let Some(branch) = event.branch.as_deref().filter(|value| !value.is_empty()) {
            snapshot.git_branch = Some(branch.to_owned());
        }
        if let Some(title) = event.title.as_deref().filter(|value| !value.is_empty()) {
            snapshot.title = Some(title.to_owned());
        }
        if let Some(model) = event.model.as_deref().filter(|value| !value.is_empty()) {
            snapshot.model = Some(model.to_owned());
        }
        if let Some(reasoning) = event.reasoning.as_deref().filter(|value| !value.is_empty()) {
            snapshot.reasoning = Some(reasoning.to_owned());
        }
        if let Some(pid) = event.pid {
            if snapshot.pid != Some(pid) {
                snapshot.discovered_pid_started_at = None;
            }
            snapshot.pid = Some(pid);
        }
        if let Some(terminal) = event.terminal_kind.as_deref() {
            snapshot.terminal_kind = Some(terminal.to_owned());
        }
        if event.agent == AgentKind::Claude && !event.session_id.starts_with("ssh:")
            && let Some(host_id) = event.payload.as_ref()
                .and_then(|payload| payload["_claude_desktop_session"].as_str())
                .filter(|id| crate::claude_desktop_code::valid_id(id))
        {
            snapshot.claude_desktop_session_id = Some(host_id.to_owned());
            snapshot.terminal_kind = Some("claude".to_owned());
        }
        if let Some(pane) = event.wezterm_pane {
            snapshot.wezterm_pane = Some(pane);
        }
        if let Some(path) = event.transcript_path.as_deref() {
            snapshot.transcript_path = Some(path.to_owned());
        }
        if let Some(parent) = event.parent_session_id.as_deref() {
            snapshot.parent_session_id = Some(parent.to_owned());
        }
        if let Some(agent_id) = event.agent_id.as_deref().filter(|value| !value.is_empty()) {
            snapshot.agent_id = Some(agent_id.to_owned());
        }
        snapshot.agent = event.agent;
        snapshot.history.push(HistoryEntry {
            at: event.timestamp,
            kind: format!("{:?}", event.event_type).to_ascii_lowercase(),
            text: event.text.clone().or_else(|| event.tool.clone()),
        });
        trim_history(&mut snapshot.history, self.history_limit);
        apply_transcript_metadata(snapshot, event);
        apply_checklist(snapshot, hook_checklist(event), event.timestamp);

        let tool_use_id = event_tool_use_id(event);
        let tool_input = event.payload.as_ref().and_then(|payload| {
            payload.get("tool_input").or_else(|| payload.get("toolInput"))
        });
        let same_agent = |request: &PendingRequest| {
            request.session_id == event.session_id && request.agent_id == event.agent_id
        };
        let had_wait = snapshot.status.needs_you();
        // A normal step by this agent takes down its old question. A step by
        // another agent sharing the same session must not answer it.
        if had_wait
            && !matches!(
                event.event_type,
                AgentEventType::Notification
                    | AgentEventType::SessionStart
                    | AgentEventType::SessionEnd
                    | AgentEventType::Heartbeat
                    | AgentEventType::Question
                    | AgentEventType::PermissionRequest
            )
        {
            self.pending.retain(|_, request| {
                !(same_agent(request) && request.kind == RequestKind::Question)
            });
        }
        if matches!(
            event.event_type,
            AgentEventType::ToolStart
                | AgentEventType::ToolEnd
                | AgentEventType::UserPrompt
                | AgentEventType::TurnEnd
        ) {
            self.pending.retain(|_, request| {
                let different_tool = event
                    .tool
                    .as_ref()
                    .zip(request.tool.as_ref())
                    .is_some_and(|(step, asked)| step != asked);
                let different_input = tool_input
                    .zip(request.tool_input.as_ref())
                    .is_some_and(|(step, asked)| step != asked);
                !(request.kind == RequestKind::Permission
                    && request.tool_use_id.is_none()
                    && same_agent(request)
                    && !different_tool
                    && !different_input)
            });
        }

        match event.event_type {
            AgentEventType::SessionStart => {
                // A rebuilt card starts over: nothing from the previous
                // conversation survives except its checklist.
                reset_for_session_start(snapshot);
                snapshot.status = SessionStatus::Idle;
            }
            AgentEventType::SubagentStart => {
                // The event was routed to a child card above; only that child
                // is touched, so the parent's tool, tools and tasks survive.
                snapshot.status = SessionStatus::Running;
                snapshot.active_tool = event
                    .tool
                    .clone()
                    .or_else(|| Some("Agent".to_owned()));
                snapshot.active_tool_detail = event.payload.as_ref()
                    .and_then(|payload| payload["agent_type"].as_str())
                    .map(ToOwned::to_owned);
                snapshot.is_executing_tool = true;
            }
            AgentEventType::SubagentEnd => {
                snapshot.status = SessionStatus::Completed;
                snapshot.active_tool = None;
                snapshot.tool_use_id = None;
                snapshot.is_executing_tool = false;
                if let Some(text) = event.text.as_deref().filter(|value| !value.is_empty()) {
                    snapshot.last_message = Some(text.to_owned());
                }
            }
            AgentEventType::UserPrompt => {
                snapshot.status = SessionStatus::Running;
                snapshot.last_prompt_at = Some(snapshot.last_prompt_at.map_or(event.timestamp, |last| last.max(event.timestamp)));
                if snapshot.recap_created_at.is_none_or(|created| created <= event.timestamp) {
                    snapshot.recap = None;
                    snapshot.recap_created_at = None;
                }
                snapshot.last_prompt = event.text.clone();
                snapshot.last_message = None;
                snapshot.live_codex_output = None;
                snapshot.error = None;
                snapshot.interrupted = false;
                snapshot.last_turn_failed = false;
                snapshot.is_executing_tool = false;
                snapshot.active_tool = None;
                snapshot.tool_use_id = None;
            }
            AgentEventType::ToolStart => {
                crate::hook_tool::apply(snapshot, event, tool_use_id.as_deref());
                // A permission gate often arrives after the tool started with
                // less detail; remember what we saw so the approval card can
                // still name the tool and show its command.
                if let Some(id) = tool_use_id.as_deref() {
                    self.tool_uses.insert(
                        tool_use_key(&event.session_id, id),
                        (event.tool.clone(), event.text.clone()),
                    );
                }
            }
            AgentEventType::ToolEnd => {
                crate::hook_tool::apply(snapshot, event, tool_use_id.as_deref());
                if let Some(id) = tool_use_id.as_deref() {
                    self.tool_uses
                        .remove(&tool_use_key(&event.session_id, id));
                    // Trae can finish the tool hook before its own approval
                    // is answered; its PermissionDenied is still definitive.
                    let keep_permission = event.agent == AgentKind::Trae
                        && event.raw_event_name.as_deref() != Some("PermissionDenied");
                    if !keep_permission
                        && let Some(index) = self.pending.values().position(|request| {
                            same_agent(request)
                                && request.kind == RequestKind::Permission
                                && request.tool_use_id.as_deref() == Some(id)
                        })
                    {
                        self.pending.shift_remove_index(index);
                    }
                }
            }
            AgentEventType::PermissionRequest => {
                snapshot.status = SessionStatus::WaitingPermission;
                self.pending.retain(|_, request| {
                    !(same_agent(request) && request.kind == RequestKind::Question)
                });
                let cached = tool_use_id
                    .as_deref()
                    .and_then(|id| self.tool_uses.get(&tool_use_key(&event.session_id, id)))
                    .cloned();
                let tool = event
                    .tool
                    .clone()
                    .or_else(|| cached.as_ref().and_then(|(tool, _)| tool.clone()));
                let body = event
                    .text
                    .clone()
                    .or_else(|| cached.as_ref().and_then(|(_, text)| text.clone()));
                // Identical tool requests keep their queue position and
                // dismissal. Parallel calls can share an id and differ in input.
                let replay = tool_use_id.as_deref().and_then(|id| {
                    self.pending.values().enumerate().find_map(|(index, request)| {
                        (same_agent(request)
                            && request.kind == RequestKind::Permission
                            && request.tool_use_id.as_deref() == Some(id)
                            && request.tool_input.as_ref() == tool_input)
                            .then(|| (index, request.hidden, request.always_scope.clone(), request.approval_config.clone()))
                    })
                });
                if replay.is_none() {
                    for queued in self.pending.values_mut().filter(|request| {
                        request.session_id == event.session_id && request.kind == RequestKind::Permission
                    }) {
                        queued.hidden = false;
                    }
                }
                let (always_scope, approval_config) = replay.as_ref()
                    .map(|(_, _, scope, config)| (scope.clone(), config.clone()))
                    .unwrap_or((always_scope, approval_config));
                let pending = PendingRequest {
                    event_id: event.id.clone(),
                    session_id: event.session_id.clone(),
                    agent: event.agent,
                    kind: RequestKind::Permission,
                    tool: tool.clone(),
                    title: event.title.clone().or_else(|| tool.clone()),
                    body,
                    options: Vec::new(),
                    questions: Vec::new(),
                    tool_use_id: tool_use_id.clone(),
                    created_at: event.timestamp,
                    hidden: replay.as_ref().is_some_and(|(_, hidden, _, _)| *hidden),
                    agent_id: event.agent_id.clone(),
                    tool_input: tool_input.cloned(),
                    always_scope,
                    approval_config,
                    native_ask_racing: false,
                };
                if let Some((index, ..)) = replay {
                    self.pending.shift_remove_index(index);
                    self.pending.shift_insert(index, pending.event_id.clone(), pending.clone());
                } else {
                    self.pending.insert(pending.event_id.clone(), pending.clone());
                }
                request = Some(pending);
            }
            AgentEventType::Question => {
                snapshot.status = SessionStatus::WaitingAnswer;
                let ask_user_question = event.tool.as_deref() == Some("AskUserQuestion");
                self.pending.retain(|_, request| {
                    !same_agent(request) || (request.kind == RequestKind::Question && !ask_user_question)
                });
                let (options, questions) = question_options(event);
                let pending = PendingRequest {
                    event_id: event.id.clone(),
                    session_id: event.session_id.clone(),
                    agent: event.agent,
                    kind: RequestKind::Question,
                    tool: event.tool.clone(),
                    title: event.title.clone(),
                    body: event.text.clone(),
                    options,
                    questions,
                    tool_use_id: tool_use_id.clone(),
                    created_at: event.timestamp,
                    hidden: false,
                    agent_id: event.agent_id.clone(),
                    tool_input: tool_input.cloned(),
                    always_scope: ApprovalAlwaysScope::Session,
                    approval_config: None,
                    native_ask_racing: event
                        .payload
                        .as_ref()
                        .and_then(|payload| payload.get("_codeisland_native_ask_racing"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                };
                self.pending.insert(pending.event_id.clone(), pending.clone());
                request = Some(pending);
            }
            AgentEventType::Message => {
                if snapshot.status != SessionStatus::Cancelled {
                    // The Codex rollout's task_started record changes the
                    // phase; an assistant message can arrive after Stop.
                    if !snapshot.status.needs_you() && event.codex_transcript_output.is_none() {
                        snapshot.status = SessionStatus::Running;
                    }
                    snapshot.last_message = event.text.clone();
                    snapshot.is_executing_tool = false;
                }
            }
            AgentEventType::Error => {
                snapshot.status = SessionStatus::Failed;
                snapshot.error = event.text.clone();
                snapshot.last_turn_failed = true;
                snapshot.is_executing_tool = false;
                turn_failed = true;
                ended_turn = true;
            }
            AgentEventType::TurnEnd => {
                if event.is_interrupted_turn() {
                    // Esc/Ctrl+C arrives as a Stop with a stop reason; the
                    // turn is cancelled, not completed.
                    snapshot.status = SessionStatus::Cancelled;
                    snapshot.interrupted = true;
                    snapshot.active_tool = None;
                    snapshot.tool_use_id = None;
                    snapshot.is_executing_tool = false;
                } else {
                    snapshot.status = SessionStatus::Completed;
                    snapshot.active_tool = None;
                    snapshot.tool_use_id = None;
                    snapshot.is_executing_tool = false;
                    if let Some(text) = event.text.as_deref().filter(|value| !value.is_empty()) {
                        snapshot.last_message = Some(text.to_owned());
                    }
                }
                ended_turn = true;
            }
            AgentEventType::TurnFailed => {
                if event.is_interrupted_turn() {
                    snapshot.status = SessionStatus::Cancelled;
                    snapshot.interrupted = true;
                    snapshot.active_tool = None;
                    snapshot.tool_use_id = None;
                    snapshot.is_executing_tool = false;
                } else {
                    snapshot.status = SessionStatus::Failed;
                    snapshot.last_turn_failed = true;
                    snapshot.is_executing_tool = false;
                    if let Some(text) = event.text.as_deref().filter(|value| !value.is_empty()) {
                        snapshot.error = Some(text.to_owned());
                    }
                    turn_failed = true;
                }
                ended_turn = true;
            }
            AgentEventType::Interrupt => {
                snapshot.status = SessionStatus::Cancelled;
                snapshot.interrupted = true;
                snapshot.is_executing_tool = false;
                snapshot.active_tool = None;
                snapshot.tool_use_id = None;
                ended_turn = true;
            }
            AgentEventType::Notification => {
                if let Some(text) = event.text.as_deref().filter(|value| !value.is_empty()) {
                    snapshot.last_message = Some(text.to_owned());
                }
            }
            AgentEventType::Heartbeat => {
                if snapshot.status != SessionStatus::Cancelled {
                    if let Some(text) = event.text.as_deref().filter(|value| !value.is_empty()) {
                        snapshot.last_message = Some(text.to_owned());
                        snapshot.is_executing_tool = false;
                    } else if event.tool.as_deref() == Some("background") {
                        snapshot.is_executing_tool = true;
                    }
                }
            }
            // Removed before the reducer runs; kept for exhaustiveness.
            AgentEventType::SessionEnd => {}
        }

        if snapshot.status.needs_you()
            && !matches!(event.event_type, AgentEventType::PermissionRequest | AgentEventType::Question)
        {
            snapshot.status = self
                .pending
                .values()
                .find(|request| request.session_id == event.session_id)
                .map(|request| match request.kind {
                    RequestKind::Permission => SessionStatus::WaitingPermission,
                    RequestKind::Question => SessionStatus::WaitingAnswer,
                })
                .unwrap_or(SessionStatus::Running);
        }

        if snapshot.active_tool.is_none() {
            snapshot.active_tool_detail = None;
        }

        if ended_turn {
            snapshot.live_codex_output = None;
        } else if snapshot.agent == AgentKind::Codex
            && !snapshot.interrupted
            && (snapshot.status.is_active() || snapshot.status.needs_you())
            && event.agent_id.is_none()
            && !matches!(event.event_type, AgentEventType::SessionStart | AgentEventType::UserPrompt)
            && let Some(output) = event.codex_transcript_output.as_deref()
                .or_else(|| event.payload.as_ref()?.get("last_assistant_message")?.as_str())
                .map(str::trim)
                .filter(|output| !output.is_empty())
            && snapshot.live_codex_output.as_deref() != Some(output)
        {
            snapshot.live_codex_output = Some(output.to_owned());
        }

        // Keep the parent's child list current. attach_child is idempotent and
        // a self-referential parent is never its own child.
        if let Some(parent) = event
            .parent_session_id
            .as_deref()
            .filter(|parent| *parent != event.session_id.as_str())
        {
            self.attach_child(parent, &event.session_id);
        }
        if event.agent == AgentKind::Codex
            && (is_new_session || event.event_type == AgentEventType::SessionStart)
        {
            let children = self.sessions.values()
                .filter(|child| child.parent_session_id.as_deref() == Some(event.session_id.as_str()))
                .map(|child| child.session_id.clone())
                .collect();
            self.sessions.get_mut(&event.session_id).expect("existing session").child_session_ids = children;
        }

        ApplyOutcome {
            applied: true,
            session_id,
            is_new_session,
            ended_turn,
            turn_failed,
            request,
        }
    }

    /// Resolve a pending request. Hide keeps the agent waiting and only hides the card.
    pub fn resolve(&mut self, event_id: &str, action: UserAction, text: Option<String>) -> Option<Decision> {
        let request = self.pending.get(event_id)?.clone();
        if !request.accepts(action) {
            return None;
        }
        if action == UserAction::Hide {
            if request.kind == RequestKind::Permission {
                for pending in self.pending.values_mut().filter(|other| {
                    other.kind == RequestKind::Permission && other.session_id == request.session_id
                }) {
                    pending.hidden = true;
                }
            } else if let Some(pending) = self.pending.get_mut(event_id) {
                pending.hidden = true;
            }
            return None;
        }
        self.pending.shift_remove(event_id);
        if let Some(session) = self.sessions.get_mut(&request.session_id) {
            if action == UserAction::AlwaysAllow && !agent_keeps_always_rule(request.agent) {
                if let ApprovalAlwaysScope::CodexRules { patterns } = &request.always_scope {
                    for pattern in patterns {
                        if !session.always_allowed_command_prefixes.contains(pattern) {
                            session.always_allowed_command_prefixes.push(pattern.clone());
                        }
                    }
                } else if let Some(tool) = request.tool.as_ref().filter(|tool| !tool.is_empty())
                    && !session.always_allowed_tools.contains(tool)
                {
                    session.always_allowed_tools.push(tool.clone());
                }
            }
            let remaining: Vec<&PendingRequest> = self
                .pending
                .values()
                .filter(|value| value.session_id == request.session_id)
                .collect();
            session.status = match remaining.first().map(|value| value.kind) {
                Some(RequestKind::Permission) => SessionStatus::WaitingPermission,
                Some(RequestKind::Question) => SessionStatus::WaitingAnswer,
                None => SessionStatus::Running,
            };
        }
        Some(Decision {
            event_id: request.event_id,
            session_id: request.session_id,
            agent: request.agent,
            action,
            text,
        })
    }

    /// Applies the same held permission rule to live requests and their queued siblings.
    pub fn session_always_allows(&self, event: &AgentEvent) -> bool {
        event.event_type == AgentEventType::PermissionRequest
            && self.is_held_approval(&event.session_id, event.agent, event.tool.as_deref(), command_for_event(event))
    }

    fn is_held_approval(
        &self,
        session_id: &str,
        agent: AgentKind,
        tool: Option<&str>,
        command: Option<&str>,
    ) -> bool {
        let Some(session) = self.get(session_id) else { return false };
        let Some(tool) = tool.filter(|tool| !tool.is_empty()) else { return false };
        !agent_keeps_always_rule(agent)
            && (session.always_allowed_tools.iter().any(|allowed| allowed == tool)
                || (agent == AgentKind::Codex && tool == "Bash"
                    && command.is_some_and(|command| commands_match(command, &session.always_allowed_command_prefixes))))
    }

    pub fn resolve_with_coverage(
        &mut self,
        event_id: &str,
        action: UserAction,
        text: Option<String>,
    ) -> Option<RequestResolution> {
        let approval_to_save = self.get_pending(event_id).filter(|request| {
            action == UserAction::AlwaysAllow && request.approval_config.is_some()
        }).cloned();
        let decision = self.resolve(event_id, action, text)?;
        let mut covered = Vec::new();
        if action == UserAction::AlwaysAllow {
            let covered_ids: Vec<String> = self.pending_for(&decision.session_id).into_iter().filter(|request| {
                request.kind == RequestKind::Permission && self.is_held_approval(
                    &request.session_id,
                    request.agent,
                    request.tool.as_deref(),
                    request.tool_input.as_ref().and_then(|input| input.get("command")).and_then(serde_json::Value::as_str),
                )
            }).map(|request| request.event_id.clone()).collect();
            covered.extend(covered_ids.into_iter().filter_map(|id| self.resolve(&id, UserAction::Approve, None)));
        }
        Some(RequestResolution { decision, approval_to_save, covered })
    }

    /// Discard a stale waiter with no decision; never manufacture an approval
    /// from an agent moving on or disconnecting.
    pub fn discard_pending(&mut self, event_id: &str) -> bool {
        let Some(request) = self.pending.shift_remove(event_id) else {
            return false;
        };
        if let Some(session) = self.sessions.get_mut(&request.session_id)
            && session.status.needs_you()
        {
            session.status = match self
                .pending
                .values()
                .find(|other| other.session_id == request.session_id)
                .map(|other| other.kind)
            {
                Some(RequestKind::Permission) => SessionStatus::WaitingPermission,
                Some(RequestKind::Question) => SessionStatus::WaitingAnswer,
                None => SessionStatus::Running,
            };
        }
        true
    }

    pub fn resolve_current(&mut self, action: UserAction, text: Option<String>) -> Option<Decision> {
        let event_id = self
            .pending
            .values()
            .find(|request| !request.hidden && request.accepts(action))
            .map(|request| request.event_id.clone())?;
        self.resolve(&event_id, action, text)
    }

    pub fn unhide(&mut self, event_id: &str) -> bool {
        if let Some(request) = self.pending.get_mut(event_id) {
            request.hidden = false;
            return true;
        }
        false
    }

    pub fn remove_session(&mut self, session_id: &str) -> bool {
        let Some(removed) = self.remove_session_entry(session_id) else {
            return false;
        };
        for child in removed.child_session_ids {
            self.remove_session(&child);
        }
        true
    }

    fn remove_session_entry(&mut self, session_id: &str) -> Option<SessionSnapshot> {
        let removed = self.sessions.shift_remove(session_id)?;
        self.pending.retain(|_, request| request.session_id != session_id);
        let tool_key = format!("{session_id}\u{1}");
        self.tool_uses.retain(|key, _| !key.starts_with(&tool_key));
        self.manual_order.retain(|value| value != session_id);
        if let Some(parent) = removed.parent_session_id.as_deref()
            && let Some(session) = self.sessions.get_mut(parent)
        {
            session.child_session_ids.retain(|value| value != session_id);
        }
        Some(removed)
    }

    /// The original idle removal policy evaluates connected families as a
    /// unit. A hidden request blocks removal just like a visible request.
    pub fn idle_session_removal_groups(&self) -> Vec<Vec<String>> {
        let mut links: HashMap<&str, Vec<&str>> = self.sessions.keys()
            .map(|id| (id.as_str(), Vec::new()))
            .collect();
        for (id, session) in &self.sessions {
            if let Some(parent) = session.parent_session_id.as_deref()
                && let Some(children) = links.get_mut(parent)
            {
                children.push(id);
                links.get_mut(id.as_str()).expect("session adjacency").push(parent);
            }
        }
        let blocked: HashSet<&str> = self.pending.values()
            .map(|request| request.session_id.as_str())
            .collect();
        let mut visited = HashSet::new();
        let mut groups = Vec::new();

        for root in self.sessions.keys().map(String::as_str) {
            if !visited.insert(root) {
                continue;
            }
            let mut group = Vec::new();
            let mut pending = vec![root];
            let mut removable = true;
            while let Some(id) = pending.pop() {
                group.push(id);
                let status = self.sessions[id].status;
                // Swift represents the resting completion/error display as idle.
                removable &= !status.is_active() && !status.needs_you() && !blocked.contains(id);
                pending.extend(links[id].iter().copied().filter(|neighbor| visited.insert(*neighbor)));
            }
            if removable {
                group.sort_unstable();
                groups.push(group.into_iter().map(str::to_owned).collect());
            }
        }
        groups.sort_unstable();
        groups
    }

    /// Revalidate the groups captured when the user opened confirmation.
    /// Delete only exact matches, so new children and newly idle cards stay.
    pub fn remove_idle_sessions(
        &mut self,
        requested: Vec<Vec<String>>,
        now: DateTime<Utc>,
    ) -> usize {
        let requested: HashSet<Vec<String>> = requested.into_iter().map(|mut group| {
            group.sort_unstable();
            group.dedup();
            group
        }).collect();
        let mut removed = 0;
        for group in self.idle_session_removal_groups() {
            if !requested.contains(&group) {
                continue;
            }
            let removal = Arc::new(RemovedIdleGroup { at: now, members: group });
            for id in &removal.members {
                self.removed_idle.insert(id.clone(), Arc::clone(&removal));
                removed += usize::from(self.remove_session_entry(id).is_some());
            }
        }
        while self.removed_idle.len() > IDLE_REMOVAL_CAPACITY {
            self.removed_idle.shift_remove_index(0);
        }
        removed
    }

    fn accept_removed_idle_event(&mut self, event: &AgentEvent) -> bool {
        let Some(removal) = self.removed_idle.get(&event.session_id).or_else(|| {
            self.removed_idle.get(event.parent_session_id.as_deref()?)
        }).cloned() else {
            return true;
        };
        let starts_work = matches!(
            event.event_type,
            AgentEventType::SessionStart
                | AgentEventType::UserPrompt
                | AgentEventType::ToolStart
                | AgentEventType::SubagentStart
        ) || event.raw_event_name.as_deref() == Some("PreCompact");
        if event.timestamp <= removal.at || !(starts_work || event.event_type.is_blocking()) {
            return false;
        }
        for id in &removal.members {
            if self.removed_idle.get(id).is_some_and(|group| Arc::ptr_eq(group, &removal)) {
                self.removed_idle.shift_remove(id);
            }
        }
        true
    }

    /// A local Claude turn with no process to observe may miss its Stop hook.
    /// Settle silent work in place, keeping the reply and avoiding completion cues.
    pub fn settle_unmonitored_claude(&mut self, now: DateTime<Utc>, session_id: Option<&str>) -> bool {
        let settle = |session: &mut SessionSnapshot| {
            settle_unmonitored_claude_turn(session, &self.pending, now)
        };
        if let Some(id) = session_id {
            return self.sessions.get_mut(id).is_some_and(settle);
        }
        self.sessions.values_mut().fold(false, |changed, session| settle(session) || changed)
    }

    pub fn remove_expired(&mut self, cutoff: DateTime<Utc>) -> usize {
        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                // A live turn is never expired, even after a long silent
                // build; only resting cards are swept.
                matches!(
                    session.status,
                    SessionStatus::Idle
                        | SessionStatus::Completed
                        | SessionStatus::Failed
                        | SessionStatus::Cancelled
                ) && session.last_activity < cutoff
                    && !self
                        .pending
                        .values()
                        .any(|request| request.session_id == session.session_id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let count = expired.len();
        for id in expired {
            self.remove_session(&id);
        }
        count
    }

    pub fn move_before(&mut self, session_id: &str, before_id: &str) -> bool {
        if session_id == before_id || !self.sessions.contains_key(session_id) {
            return false;
        }
        let mut order = self.ordered_ids();
        order.retain(|value| value != session_id);
        let Some(index) = order.iter().position(|value| value == before_id) else {
            return false;
        };
        order.insert(index, session_id.to_owned());
        self.manual_order = order;
        true
    }

    pub fn set_manual_order(&mut self, order: Vec<String>) {
        self.manual_order = order
            .into_iter()
            .filter(|value| self.sessions.contains_key(value))
            .collect();
    }

    pub fn ordered_ids(&self) -> Vec<String> {
        ordering::order_sessions(&self.sessions, &self.pending)
    }

    pub fn ordered_sessions(&self) -> Vec<SessionSnapshot> {
        self.ordered_ids()
            .into_iter()
            .filter_map(|id| self.sessions.get(&id).cloned())
            .map(|mut session| {
                session.model_label = session.model.as_deref().and_then(|model| {
                    codeisland_swift::call(c"modelLabel", serde_json::json!({ "model": model, "effort": session.reasoning }))
                });
                session
            })
            .collect()
    }

    pub fn current_session(&self) -> Option<&SessionSnapshot> {
        let ordered = self.ordered_ids();
        // A session waiting on the user outranks any manual preference.
        if let Some(session) = ordered.iter().find_map(|id| {
            let session = self.sessions.get(id)?;
            session.status.needs_you().then_some(session)
        }) {
            return Some(session);
        }
        // Among working sessions, a manual drag decides; otherwise the
        // tier ordering does.
        if let Some(session) = self.manual_order.iter().find_map(|id| {
            let session = self.sessions.get(id)?;
            session.status.is_active().then_some(session)
        }) {
            return Some(session);
        }
        ordered
            .iter()
            .find_map(|id| {
                let session = self.sessions.get(id)?;
                session.status.is_active().then_some(session)
            })
            .or_else(|| ordered.first().and_then(|id| self.sessions.get(id)))
    }

    pub fn snapshot(&self) -> StoreSnapshot {
        StoreSnapshot {
            sessions: self.ordered_sessions(),
            pending: self.pending.values().cloned().collect(),
            current_session_id: self.current_session().map(|session| session.session_id.clone()),
            total_sessions: self.sessions.len(),
        }
    }

    pub fn persisted(&self) -> PersistedState {
        let mut sessions = self.sessions.clone();
        for session in sessions.values_mut() {
            session.live_codex_output = None;
        }
        PersistedState {
            sessions,
            manual_order: self.manual_order.clone(),
            saved_at: Some(Utc::now()),
        }
    }

    pub fn restore(&mut self, state: PersistedState) {
        self.sessions = state.sessions;
        self.manual_order = state
            .manual_order
            .into_iter()
            .filter(|id| self.sessions.contains_key(id))
            .collect();
        // Pending requests cannot survive a restart: the hook process is gone.
        self.pending.clear();
        self.removed_idle.clear();
        self.tool_uses.clear();
        self.seen_events.clear();
        self.seen_set.clear();
        self.last_seq.clear();
        // A restart restores cards, never turns: every session lands Idle with
        // no tool in flight. The conversation text and checklist survive.
        for session in self.sessions.values_mut() {
            session.status = SessionStatus::Idle;
            session.wsl_process = None;
            session.active_tool = None;
            session.active_tool_detail = None;
            session.live_codex_output = None;
            session.tool_use_id = None;
            session.is_executing_tool = false;
            session.always_allowed_tools.clear();
            session.always_allowed_command_prefixes.clear();
            session.child_session_ids.clear();
            session.error = None;
            session.interrupted = false;
            session.last_turn_failed = false;
            if session.last_prompt_at.is_none() {
                session.last_prompt_at = session.history.iter()
                    .filter(|event| event.kind == "userprompt")
                    .map(|event| event.at)
                    .max();
            }
            trim_history(&mut session.history, self.history_limit);
        }
        let codex_links: Vec<_> = self.sessions.values()
            .filter(|session| session.agent == AgentKind::Codex)
            .filter_map(|session| Some((session.parent_session_id.clone()?, session.session_id.clone())))
            .collect();
        for (parent, child) in codex_links {
            self.attach_child(&parent, &child);
        }
    }

    pub fn apply_history_limit(&mut self, limit: usize) {
        self.history_limit = limit.max(1);
        for session in self.sessions.values_mut() {
            trim_history(&mut session.history, self.history_limit);
        }
    }

    /// The visible limit sizes the rendered list; it never drops sessions.
    pub fn set_max_visible(&mut self, max_visible: usize) {
        self._max_visible = max_visible.max(1);
    }

    fn mark_seen(&mut self, event_id: &str) -> bool {
        if event_id.is_empty() {
            return true;
        }
        if self.seen_set.contains(event_id) {
            return false;
        }
        self.seen_set.insert(event_id.to_owned());
        self.seen_events.push_back(event_id.to_owned());
        while self.seen_events.len() > SEEN_EVENT_CAPACITY {
            if let Some(old) = self.seen_events.pop_front() {
                self.seen_set.remove(&old);
            }
        }
        true
    }

    fn attach_child(&mut self, parent_id: &str, child_id: &str) -> bool {
        if parent_id != child_id
            && let Some(parent) = self.sessions.get_mut(parent_id)
            && !parent.child_session_ids.iter().any(|value| value == child_id)
        {
            parent.child_session_ids.push(child_id.to_owned());
            return true;
        }
        false
    }

}

fn trim_history(history: &mut Vec<HistoryEntry>, limit: usize) {
    if history.len() > limit {
        history.drain(..history.len() - limit);
    }
}

fn apply_transcript_metadata(session: &mut SessionSnapshot, event: &AgentEvent) -> bool {
    match event.transcript_metadata.as_ref() {
        Some(TranscriptMetadata::Recap(recap))
            if session.agent == AgentKind::Claude
                && session.last_prompt_at.is_none_or(|prompt| prompt < event.timestamp)
                && session.recap_created_at.is_none_or(|previous| previous <= event.timestamp) =>
        {
            let changed = session.recap.as_ref() != Some(recap) || session.recap_created_at != Some(event.timestamp);
            session.recap = Some(recap.clone());
            session.recap_created_at = Some(event.timestamp);
            changed
        }
        Some(TranscriptMetadata::Checklist(updates)) => {
            apply_checklist(session, updates.clone(), event.timestamp)
        }
        _ => false,
    }
}

fn reset_for_session_start(session: &mut SessionSnapshot) {
    session.last_prompt = None;
    session.last_message = None;
    session.live_codex_output = None;
    session.active_tool = None;
    session.active_tool_detail = None;
    session.tool_use_id = None;
    session.error = None;
    session.interrupted = false;
    session.last_turn_failed = false;
    session.is_executing_tool = false;
    session.always_allowed_tools.clear();
    session.always_allowed_command_prefixes.clear();
    session.child_session_ids.clear();
    session.history.clear();
    // Tasks survive: a resumed or compacted conversation is still working
    // through the same checklist. Only /clear (a new session) drops them.
}

fn settle_unmonitored_claude_turn(
    session: &mut SessionSnapshot,
    pending: &IndexMap<String, PendingRequest>,
    now: DateTime<Utc>,
) -> bool {
    if session.agent != AgentKind::Claude
        || !session.status.is_active()
        || session.pid.is_some()
        || session.wsl_process.is_some()
        || session.session_id.starts_with("ssh:")
        || session.session_id.starts_with(crate::cowork::PREFIX)
    {
        return false;
    }
    let timeout = if session.active_tool.is_some() { 180 } else { 300 };
    if now - session.last_activity <= chrono::TimeDelta::seconds(timeout)
        || pending.values().any(|request| request.session_id == session.session_id)
    {
        return false;
    }
    session.status = SessionStatus::Idle;
    session.active_tool = None;
    session.active_tool_detail = None;
    session.tool_use_id = None;
    session.is_executing_tool = false;
    true
}

fn replace_discovered(target: &mut Option<String>, value: Option<&str>, newest: bool) -> bool {
    let Some(value) = value.filter(|value| !value.is_empty()) else { return false };
    if target.as_deref() == Some(value) || (!newest && target.is_some()) {
        return false;
    }
    *target = Some(value.to_owned());
    true
}

fn event_tool_use_id(event: &AgentEvent) -> Option<String> {
    event.payload.as_ref().and_then(|payload| {
        payload
            .get("tool_use_id")
            .or_else(|| payload.get("toolUseId"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn tool_use_key(session_id: &str, tool_use_id: &str) -> String {
    format!("{session_id}\u{1}{tool_use_id}")
}

fn question_options(event: &AgentEvent) -> (Vec<String>, Vec<QuestionItem>) {
    let Some(payload) = event.payload.as_ref() else {
        return (Vec::new(), Vec::new());
    };
    if let Some(questions) = payload.get("questions").and_then(|value| value.as_array()) {
        let items: Vec<QuestionItem> = questions
            .iter()
            .filter_map(|question| {
                let text = question
                    .get("question")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned();
                if text.is_empty() {
                    return None;
                }
                let (options, mut option_descriptions): (Vec<_>, Vec<_>) = question
                    .get("options")
                    .and_then(|value| value.as_array())
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(|option| {
                                option
                                    .get("label")
                                    .and_then(|label| label.as_str())
                                    .or_else(|| option.as_str())
                                    .map(|label| (
                                        label.to_owned(),
                                        option.get("description").and_then(|value| value.as_str())
                                            .unwrap_or_default().to_owned(),
                                    ))
                            })
                            .unzip()
                    })
                    .unwrap_or_default();
                if option_descriptions.iter().all(String::is_empty) {
                    option_descriptions.clear();
                }
                Some(QuestionItem {
                    question: text,
                    header: question
                        .get("header")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned),
                    options,
                    multi_select: question
                        .get("multiSelect")
                        .or_else(|| question.get("multi_select"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                    answer_key: question.get("answer_key").and_then(|value| value.as_str())
                        .map(ToOwned::to_owned),
                    option_descriptions,
                    is_secret: question.get("isSecret").and_then(|value| value.as_bool()).unwrap_or(false),
                })
            })
            .collect();
        let first = items.first().map(|item| item.options.clone()).unwrap_or_default();
        return (first, items);
    }
    let options = payload
        .get("options")
        .and_then(|value| value.as_array())
        .map(|options| {
            options
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    (options, Vec::new())
}
