//! The shared runtime: owns the session store, settings, attention decisions, blocking
//! waiters, persistence and the background services started by [`Runtime::start`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::{io, path::PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use codeisland_domain::{
    attention, AgentEvent, AgentEventType, AgentKind, AppSettings, ApplyOutcome, AttentionContext,
    AttentionDecision, AttentionIntents, AttentionSubject, CompletionPresentation, Decision,
    FollowUpIntent, PendingRequest,
    PersistedState, PushKind, RequestKind, SessionSnapshot, SessionStatus, SessionStore,
    StoreSnapshot, UserAction, CODEX_DESKTOP_SESSION_PREFIX, codex_desktop_session_id,
};
use codeisland_domain::store::RequestResolution;
use codeisland_domain::follow_up::{Delivery, Kind as ReminderKind, Origin as ReminderOrigin, Reminder};
use codeisland_integrations::push::{PushChannel, PushGate, PushNotifier};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, Notify, broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::follow_up::{FollowUps, reminder_request, still_pending};
use crate::push_format::{push_content, push_title, push_words};
use crate::island::{CardShortcut, CardShortcutOutcome, IslandSurface, visible_requests};
use crate::persistence;
use crate::{ipc_handler, process, tailer};

const BROADCAST_CAPACITY: usize = 256;
const PERSIST_DEBOUNCE: Duration = Duration::from_millis(250);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
const COMPLETION_VISIBLE: Duration = Duration::from_secs(5);
const GLANCE_VISIBLE: Duration = Duration::from_secs(600);
static SETTINGS_WRITES: Mutex<()> = Mutex::new(());

type TerminalVisibilityDetector = dyn Fn(&SessionSnapshot) -> bool + Send + Sync;

/// Everything the webview needs to render the island.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiState {
    pub store: StoreSnapshot,
    pub settings: AppSettings,
    pub version: String,
    pub surface: IslandSurface,
    pub surface_revision: u32,
    pub glance_completion_active: bool,
    pub follow_up_hint_active: bool,
    pub follow_up_hint_pulse: u32,
    #[serde(default)]
    pub display_asleep: bool,
    pub idle_session_removal_groups: Vec<Vec<String>>,
}

/// Updates broadcast to subscribers (the Tauri layer maps these onto `island://` events).
#[derive(Clone, Debug)]
pub enum RuntimeUpdate {
    State(StoreSnapshot),
    Attention {
        session_id: String,
        decision: AttentionDecision,
    },
    Settings(AppSettings),
    LocalClaudeStop,
    AwayChanged,
    Toast {
        kind: String,
        message: String,
    },
}

/// Shared, `Arc`-friendly runtime state.
///
/// All locks are taken for short synchronous sections only; no lock is held across an
/// `.await`, so blocking decisions can wait while the UI resolves requests.
pub struct Runtime {
    /// Weak self-reference so `&self` methods can spawn debounced persistence tasks.
    self_ref: Weak<Runtime>,
    store: Mutex<SessionStore>,
    island: RwLock<IslandPresentation>,
    settings_path: PathBuf,
    updates: broadcast::Sender<RuntimeUpdate>,
    waiters: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
    session_in_front: AtomicBool,
    shortcut_recording: AtomicBool,
    terminal_visibility: RwLock<Option<Arc<TerminalVisibilityDetector>>>,
    terminal_tab_visibility: RwLock<Option<Arc<TerminalVisibilityDetector>>>,
    push: AsyncMutex<PushDelivery>,
    held_pushes: Mutex<HashMap<String, (AgentEvent, PushKind)>>,
    away: AtomicBool,
    display_asleep: AtomicBool,
    idle_seconds: AtomicU64,
    follow_up_wake: Notify,
    codex_desktop_running: AtomicBool,
    closed_codex_desktop: Mutex<HashMap<String, DateTime<Utc>>>,
    discovered_codex_rollouts: Mutex<HashMap<String, String>>,
    dirty: AtomicBool,
    persist_scheduled: AtomicBool,
}

#[derive(Default)]
struct PushDelivery {
    gate: PushGate,
    last_completion: HashMap<String, DateTime<Utc>>,
    global: VecDeque<Instant>,
    channel_sends: HashMap<PushChannel, VecDeque<Instant>>,
}

struct PushMessage {
    id: String,
    session_id: String,
    kind: PushKind,
    title: String,
    headline: String,
    body: String,
}

struct IslandPresentation {
    settings: AppSettings,
    surface: IslandSurface,
    revision: u32,
    card_rendered: bool,
    completion_queue: VecDeque<String>,
    completion_entered: bool,
    pointer_over_island: bool,
    follow_ups: FollowUps,
    completion_timeout: Option<u32>,
    glance_completion_active: bool,
    glance_revision: u32,
    glance_timeout: Option<u32>,
}

impl IslandPresentation {
    fn present(&mut self, surface: IslandSurface) {
        self.settings.collapsed = surface.is_collapsed();
        if self.surface != surface {
            self.follow_ups.presented(&surface);
        }
        self.surface = surface;
        self.revision = self.revision.wrapping_add(1);
        self.card_rendered = false;
        self.completion_entered = false;
        self.completion_timeout = match self.surface {
            IslandSurface::CompletionCard { .. } => Some(self.revision),
            _ => None,
        };
        if !self.settings.collapsed {
            self.glance_completion_active = false;
            self.glance_timeout = None;
        }
    }

    fn is_looking_at(&self, reminder: &Reminder) -> bool {
        if !self.pointer_over_island {
            return false;
        }
        match &self.surface {
            IslandSurface::SessionList => true,
            IslandSurface::ApprovalCard { session_id, .. } => {
                reminder.key.kind == ReminderKind::Approval && &reminder.key.session_id == session_id
            }
            IslandSurface::QuestionCard { session_id, .. } => {
                reminder.key.kind == ReminderKind::Question && &reminder.key.session_id == session_id
            }
            IslandSurface::CompletionCard { session_id } => {
                reminder.key.kind == ReminderKind::Completion && &reminder.key.session_id == session_id
            }
            IslandSurface::Collapsed => false,
        }
    }

    fn flash_completion(&mut self) {
        if self.surface.is_collapsed() {
            self.glance_completion_active = true;
            self.glance_revision = self.glance_revision.wrapping_add(1);
            self.glance_timeout = Some(self.glance_revision);
        }
    }
}

impl Runtime {
    /// Creates the runtime, restoring any persisted sessions and validating settings.
    pub fn new(settings: AppSettings) -> Arc<Self> {
        Self::build(settings, persistence::load_state())
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(settings: AppSettings) -> Arc<Self> {
        Self::build(settings, PersistedState::default())
    }

    pub(crate) fn build(settings: AppSettings, persisted: PersistedState) -> Arc<Self> {
        Self::build_with_settings_path(settings, persisted, persistence::settings_path())
    }

    fn build_with_settings_path(settings: AppSettings, persisted: PersistedState, settings_path: PathBuf) -> Arc<Self> {
        let settings = settings.validate();
        let mut store = SessionStore::new(settings.max_visible_sessions, settings.max_history);
        if !persisted.sessions.is_empty() {
            store.restore(persisted);
            store.set_max_visible(settings.max_visible_sessions);
            store.apply_history_limit(settings.max_history);
            // A restart is not a replay: drop cards that were already stale
            // before the app came back (0 means never expire).
            if settings.session_cleanup_minutes > 0 {
                let cutoff = Utc::now()
                    - chrono::Duration::minutes(i64::from(settings.session_cleanup_minutes));
                store.remove_expired(cutoff);
            }
        }

        let (updates, _receiver) = broadcast::channel(BROADCAST_CAPACITY);
        Arc::new_cyclic(|weak| Self {
            self_ref: weak.clone(),
            store: Mutex::new(store),
            settings_path,
            island: RwLock::new(IslandPresentation {
                surface: if settings.collapsed {
                    IslandSurface::Collapsed
                } else {
                    IslandSurface::SessionList
                },
                settings,
                revision: 0,
                card_rendered: false,
                completion_queue: VecDeque::new(),
                completion_entered: false,
                pointer_over_island: false,
                follow_ups: FollowUps::default(),
                completion_timeout: None,
                glance_completion_active: false,
                glance_revision: 0,
                glance_timeout: None,
            }),
            updates,
            waiters: Mutex::new(HashMap::new()),
            session_in_front: AtomicBool::new(false),
            shortcut_recording: AtomicBool::new(false),
            terminal_visibility: RwLock::new(None),
            terminal_tab_visibility: RwLock::new(None),
            push: AsyncMutex::new(PushDelivery::default()),
            held_pushes: Mutex::new(HashMap::new()),
            away: AtomicBool::new(false),
            display_asleep: AtomicBool::new(false),
            idle_seconds: AtomicU64::new(0),
            follow_up_wake: Notify::new(),
            codex_desktop_running: AtomicBool::new(false),
            closed_codex_desktop: Mutex::new(HashMap::new()),
            discovered_codex_rollouts: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
            persist_scheduled: AtomicBool::new(false),
        })
    }

    /// Current UI state.
    pub fn state(&self) -> UiState {
        let store = lock(&self.store);
        let island = read_lock(&self.island);
        let mut snapshot = store.snapshot();
        if let Some(session_id) = island.surface.session_id()
            && store.get(session_id).is_some()
        {
            snapshot.current_session_id = Some(session_id.to_owned());
        }
        UiState {
            store: snapshot,
            settings: island.settings.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            surface: island.surface.clone(),
            surface_revision: island.revision,
            glance_completion_active: island.glance_completion_active,
            follow_up_hint_active: island.follow_ups.hint_active(),
            follow_up_hint_pulse: island.follow_ups.hint_pulse,
            display_asleep: self.is_display_asleep(),
            idle_session_removal_groups: store.idle_session_removal_groups(),
        }
    }

    /// Current settings.
    pub fn settings(&self) -> AppSettings {
        read_lock(&self.island).settings.clone()
    }

    pub fn surface(&self) -> IslandSurface {
        read_lock(&self.island).surface.clone()
    }

    pub fn current_session_in_front(&self) -> bool {
        let store = lock(&self.store);
        store.current_session().is_some_and(|session| {
            self.terminal_is_front(&store, &session.session_id)
        })
    }

    pub fn has_sessions(&self) -> bool {
        lock(&self.store).session_count() != 0
    }

    pub fn has_active_sessions(&self) -> bool {
        lock(&self.store)
            .sessions()
            .values()
            .any(|session| session.status.is_active() || session.status.needs_you())
    }

    /// Presents a live target and invalidates any acknowledgment for an earlier
    /// presentation, including a previously hidden instance of the same card.
    pub fn set_surface(&self, surface: IslandSurface) -> bool {
        {
            let store = lock(&self.store);
            if !surface.is_available(&store) {
                return false;
            }
            let mut island = write_lock(&self.island);
            if matches!(surface, IslandSurface::Collapsed | IslandSurface::SessionList) {
                island.completion_queue.clear();
            }
            island.present(surface);
        }
        self.emit_state();
        true
    }

    /// Publishes a folded or list surface without saving preferences.
    pub fn set_collapsed(&self, collapsed: bool) {
        self.set_surface(if collapsed {
            IslandSurface::Collapsed
        } else {
            IslandSurface::SessionList
        });
    }

    /// An explicit click can bring back a dismissed question. With no
    /// selection, prefer the first visible question, as on the compact bar.
    pub fn open_pending_question(&self, event_id: Option<&str>) -> bool {
        {
            let mut store = lock(&self.store);
            let request = if let Some(event_id) = event_id {
                store
                    .get_pending(event_id)
                    .filter(|request| request.kind == RequestKind::Question)
            } else {
                visible_requests(&store, RequestKind::Question)
                    .next()
                    .or_else(|| {
                        store
                            .pending()
                            .values()
                            .find(|request| request.kind == RequestKind::Question)
                    })
            };
            let Some(request) = request else {
                return false;
            };
            let surface = IslandSurface::for_request(request);
            let event_id = request.event_id.clone();
            store.unhide(&event_id);
            write_lock(&self.island).present(surface);
        }
        self.emit_state();
        true
    }

    pub fn set_pointer_over_island(&self, inside: bool) {
        let saw_completion = {
            let mut island = write_lock(&self.island);
            island.pointer_over_island = inside;
            if inside && let IslandSurface::CompletionCard { session_id } = &island.surface {
                let session_id = session_id.clone();
                island.completion_entered = true;
                island.follow_ups.completion_seen(&session_id);
                true
            } else {
                false
            }
        };
        if saw_completion {
            self.emit_state();
        }
    }

    pub fn user_jumped_to_session(&self, session_id: &str) {
        write_lock(&self.island).follow_ups.user_jumped(session_id);
        self.emit_state();
    }

    /// Supplies the platform's lock/sleep/screen-saver state.
    pub fn set_away(&self, away: bool) {
        if self.away.swap(away, Ordering::Relaxed) != away {
            self.follow_up_wake.notify_one();
            let _ = self.updates.send(RuntimeUpdate::AwayChanged);
            if away {
                self.release_held_pushes();
            }
        }
    }

    pub fn is_away(&self) -> bool {
        self.away.load(Ordering::Relaxed)
    }

    pub fn set_display_asleep(&self, asleep: bool) {
        if self.display_asleep.swap(asleep, Ordering::Relaxed) != asleep {
            let _ = self.updates.send(RuntimeUpdate::AwayChanged);
            if asleep {
                self.release_held_pushes();
            }
        }
    }

    pub fn is_display_asleep(&self) -> bool {
        self.display_asleep.load(Ordering::Relaxed)
    }

    pub fn set_idle_seconds(&self, seconds: u64) {
        if self.idle_seconds.swap(seconds, Ordering::Relaxed) != seconds {
            self.release_held_pushes();
        }
    }

    fn is_away_for_push(&self, settings: &AppSettings) -> bool {
        self.is_away()
            || self.is_display_asleep()
            || self.idle_seconds.load(Ordering::Relaxed) >= u64::from(settings.push_away_idle_minutes) * 60
    }

    /// The desktop supplies its platform-specific terminal visibility check.
    /// Smart Suppress always checks the session that owns the request.
    pub fn set_terminal_visibility_detector(
        &self,
        detector: impl Fn(&SessionSnapshot) -> bool + Send + Sync + 'static,
    ) {
        *write_lock(&self.terminal_visibility) = Some(Arc::new(detector));
    }

    pub fn is_shortcut_recording(&self) -> bool {
        self.shortcut_recording.load(Ordering::Relaxed)
    }

    pub fn set_shortcut_recording(&self, recording: bool) -> bool {
        self.shortcut_recording.swap(recording, Ordering::Relaxed) != recording
    }

    /// Accept the webview's acknowledgment only for the current live card.
    pub fn acknowledge_surface(&self, event_id: &str, revision: u32) -> bool {
        let store = lock(&self.store);
        let mut island = write_lock(&self.island);
        if island.revision != revision
            || island
                .surface
                .current_request(&store)
                .is_none_or(|request| request.event_id != event_id)
        {
            return false;
        }
        island.card_rendered = true;
        true
    }

    /// Act only on the exact matching card the user can see. The desktop
    /// adapter reveals the returned surface without resolving its request.
    pub fn perform_card_shortcut(
        &self,
        shortcut: CardShortcut,
        island_on_screen: bool,
    ) -> CardShortcutOutcome {
        let mut store = lock(&self.store);
        let island = read_lock(&self.island);
        let kind = shortcut.request_kind();
        if let Some(request) = island
            .surface
            .current_request(&store)
            .filter(|request| request.kind == kind)
        {
            if !island_on_screen || !island.card_rendered {
                return CardShortcutOutcome::Opened(island.surface.clone());
            }
            let event_id = request.event_id.clone();
            let decision = store.resolve_with_coverage(&event_id, shortcut.decision(), None);
            drop(island);
            drop(store);
            if let Some(decision) = decision {
                self.complete_resolution(decision);
                return CardShortcutOutcome::Acted;
            }
            return CardShortcutOutcome::Ignored;
        }
        let outcome = visible_requests(&store, kind)
            .next()
            .map(|request| CardShortcutOutcome::Opened(IslandSurface::for_request(request)))
            .unwrap_or(CardShortcutOutcome::Ignored);
        outcome
    }

    /// Validates, applies and persists new settings.
    pub fn update_settings(&self, settings: AppSettings) -> AppSettings {
        self.try_update_settings(|_| Ok(settings)).unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to persist settings");
            self.settings()
        })
    }

    /// Reads, saves and applies a preference change as one serialized update.
    pub fn try_update_settings(
        &self,
        update: impl FnOnce(&AppSettings) -> io::Result<AppSettings>,
    ) -> io::Result<AppSettings> {
        self.commit_settings(|current| {
            let mut next = update(current)?;
            // The native file picker owns this setting independently of a form.
            next.custom_sound_paths.clone_from(&current.custom_sound_paths);
            Ok(next)
        })
    }

    pub fn set_custom_sound_path(&self, cue: &str, path: Option<String>) -> io::Result<()> {
        if !codeisland_domain::settings::SOUND_FILE_NAMES.contains(&cue) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid sound cue"));
        }
        self.commit_settings(|current| {
            let mut next = current.clone();
            if let Some(path) = path {
                next.custom_sound_paths.insert(cue.to_owned(), path);
            } else {
                next.custom_sound_paths.remove(cue);
            }
            Ok(next)
        }).map(|_| ())
    }

    fn commit_settings(&self, update: impl FnOnce(&AppSettings) -> io::Result<AppSettings>) -> io::Result<AppSettings> {
        let next = {
            let _write = lock(&SETTINGS_WRITES);
            let current = self.settings();
            let mut next = update(&current)?.validate();
            next.collapsed = current.collapsed;
            persistence::save_settings_to(&self.settings_path, &next)?;

            let mut store = lock(&self.store);
            let mut island = write_lock(&self.island);
            next.collapsed = island.surface.is_collapsed();
            if current.max_visible_sessions != next.max_visible_sessions {
                store.set_max_visible(next.max_visible_sessions);
            }
            if current.max_history != next.max_history {
                store.apply_history_limit(next.max_history);
            }
            island.settings = next.clone();
            let _ = self.updates.send(RuntimeUpdate::Settings(next.clone()));
            next
        };
        self.emit_state();
        Ok(next)
    }

    /// Applies an event, computes attention, broadcasts updates and schedules persistence.
    pub fn handle_event(self: &Arc<Self>, mut event: AgentEvent) -> ApplyOutcome {
        if event.agent == AgentKind::Codex
            && self.codex_desktop_running.load(Ordering::Relaxed)
            && !event.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX)
            && !event.session_id.starts_with("ssh:")
            && event.transcript_path.as_deref().is_some_and(|path| {
                self.is_discovered_codex_rollout(&event.session_id, path)
                    || tailer::is_codex_desktop_transcript(path)
            })
        {
            event.session_id = codex_desktop_session_id(&event.session_id);
            event.parent_session_id = event.parent_session_id.as_deref().map(codex_desktop_session_id);
            event.terminal_kind = Some("codex".to_owned());
        }
        self.apply_event(event, false)
    }

    pub(crate) fn handle_transcript_event(self: &Arc<Self>, mut event: AgentEvent) -> ApplyOutcome {
        if event.agent == AgentKind::Codex
            && !event.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX)
            && event.transcript_path.as_deref().is_some_and(|path| {
                self.is_discovered_codex_rollout(&event.session_id, path)
            })
        {
            event.session_id = codex_desktop_session_id(&event.session_id);
            event.parent_session_id = event.parent_session_id.as_deref().map(codex_desktop_session_id);
            event.terminal_kind = Some("codex".to_owned());
        }
        self.apply_event(event, true)
    }

    pub(crate) fn backfill_transcript_metadata(&self, event: &AgentEvent) {
        let changed = {
            let mut store = lock(&self.store);
            self.admit_codex_event(&store, event, true) && store.apply_transcript_metadata(event)
        };
        if changed {
            self.emit_state();
            self.schedule_persist();
        }
    }

    fn is_discovered_codex_rollout(&self, raw_id: &str, path: &str) -> bool {
        lock(&self.discovered_codex_rollouts).get(raw_id).is_some_and(|recorded| {
            codeisland_domain::discovery::same_transcript_path(recorded, path)
        })
    }

    pub(crate) fn reconcile_codex_rollout_parent(&self, child: &str, parent: &str, path: &str, title: Option<&str>) {
        let desktop_alias = !child.starts_with(CODEX_DESKTOP_SESSION_PREFIX)
            && self.is_discovered_codex_rollout(child, path);
        let (desktop_child, desktop_parent);
        let (child, parent) = if desktop_alias {
            desktop_child = codex_desktop_session_id(child);
            desktop_parent = codex_desktop_session_id(parent);
            (desktop_child.as_str(), desktop_parent.as_str())
        } else {
            (child, parent)
        };
        let changed = self.can_tail_codex_desktop(child)
            && lock(&self.store).link_codex_rollout_parent(child, parent, path, title);
        if changed {
            self.emit_state();
            self.schedule_persist();
        }
    }

    pub(crate) fn restore_codex_rollout_turn(
        &self,
        session_id: &str,
        path: &str,
        modified_at: DateTime<Utc>,
        turn: tailer::RecoveredTurn,
    ) {
        let changed = session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX)
            && self.can_tail_codex_desktop(session_id)
            && lock(&self.store).restore_codex_rollout_turn(
                session_id,
                path,
                modified_at,
                turn == tailer::RecoveredTurn::Running,
            );
        if changed {
            self.emit_state();
            self.schedule_persist();
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn is_codex_desktop_running(&self) -> bool {
        self.codex_desktop_running.load(Ordering::Relaxed)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn merge_provider_discovery(&self, mut records: Vec<codeisland_domain::discovery::DiscoveredSession>) {
        records.sort_by_key(|record| (record.parent_session_id.is_some(), record.session_id.clone()));
        let changed = {
            let mut store = lock(&self.store);
            let mut changed = false;
            for record in records {
                if record.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX) {
                    if !self.is_codex_desktop_running() {
                        continue;
                    }
                    let mut closed = lock(&self.closed_codex_desktop);
                    if let Some(closed_at) = closed.get(&record.session_id) {
                        if record.modified_at <= *closed_at || !record.status.is_active() {
                            continue;
                        }
                        closed.remove(&record.session_id);
                    }
                    lock(&self.discovered_codex_rollouts).insert(
                        record.provider_session_id.clone(), record.transcript_path.clone(),
                    );
                }
                if record.agent == AgentKind::Claude {
                    changed |= store.remove_cowork_shadowed_by(&record.provider_session_id);
                }
                changed |= store.merge_discovered(&record, process::is_session_process_alive);
            }
            changed
        };
        if changed {
            self.emit_state();
            self.schedule_persist();
        }
    }

    #[cfg(windows)]
    pub(crate) fn wsl_sessions(&self) -> Vec<SessionSnapshot> {
        lock(&self.store).sessions().values().filter(|session| {
            matches!(session.agent, AgentKind::Codex | AgentKind::Claude)
                && session.transcript_path.as_deref().is_some_and(|path| {
                    crate::wsl_process::is_wsl_transcript(std::path::Path::new(path))
                })
        }).cloned().collect()
    }

    #[cfg(windows)]
    pub(crate) fn merge_wsl_processes(&self, observations: Vec<(SessionSnapshot, crate::wsl_process::Observation)>) {
        let changed = {
            let mut store = lock(&self.store);
            let mut changed = false;
            for (session, observation) in observations {
                changed |= store.reconcile_wsl_process(
                    &session.session_id, session.transcript_path.as_deref(), observation.process,
                    observation.status, observation.settle,
                );
            }
            changed
        };
        if changed {
            self.emit_state();
            self.schedule_persist();
        }
    }

    pub(crate) fn can_tail_codex_desktop(&self, session_id: &str) -> bool {
        !session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX)
            || (self.codex_desktop_running.load(Ordering::Relaxed)
                && !lock(&self.closed_codex_desktop).contains_key(session_id))
    }

    #[cfg(any(windows, test))]
    pub(crate) fn set_codex_desktop_running(&self, running: bool) {
        self.codex_desktop_running.store(running, Ordering::Relaxed);
        if !running {
            let desktop_sessions: Vec<_> = lock(&self.store).sessions().keys()
                .filter(|id| id.starts_with(CODEX_DESKTOP_SESSION_PREFIX))
                .cloned()
                .collect();
            for id in desktop_sessions {
                self.remove_session(&id);
            }
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn observe_codex_desktop_thread(
        &self,
        event: &AgentEvent,
        status: Option<SessionStatus>,
        preview: Option<&str>,
    ) {
        let applied = {
            let mut store = lock(&self.store);
            if event.event_type == AgentEventType::SessionStart {
                lock(&self.closed_codex_desktop).remove(&event.session_id);
            }
            store.observe_codex_desktop_thread(event, status, preview)
        };
        if applied {
            self.emit_state();
            self.schedule_persist();
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn close_codex_desktop_thread(&self, session_id: &str) {
        lock(&self.closed_codex_desktop).insert(session_id.to_owned(), Utc::now());
        self.remove_session(session_id);
    }

    #[cfg(any(windows, test))]
    pub(crate) fn queue_codex_desktop_question(
        self: &Arc<Self>,
        event: AgentEvent,
    ) -> Option<oneshot::Receiver<Decision>> {
        let event_id = event.id.clone();
        let (sender, receiver) = oneshot::channel();
        lock(&self.waiters).insert(event_id.clone(), sender);
        lock(&self.closed_codex_desktop).remove(&event.session_id);
        if !self.handle_event(event).applied {
            lock(&self.waiters).remove(&event_id);
            return None;
        }
        Some(receiver)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn cancel_codex_desktop_questions<'a>(
        &self, event_ids: impl Iterator<Item = &'a str>,
    ) {
        let mut discarded = false;
        {
            let mut store = lock(&self.store);
            let mut waiters = lock(&self.waiters);
            for event_id in event_ids {
                discarded |= store.discard_pending(event_id);
                waiters.remove(event_id);
            }
        }
        if discarded {
            self.emit_state();
            self.schedule_persist();
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn reconcile_cowork(&self, updates: &[codeisland_domain::cowork::Update]) {
        use codeisland_domain::cowork;

        let keep: HashSet<_> = updates.iter().map(|update| {
            format!("{}{}", cowork::PREFIX, update.metadata.id)
        }).collect();
        let stale: Vec<_> = lock(&self.store).sessions().keys()
            .filter(|id| id.starts_with(cowork::PREFIX) && !keep.contains(*id))
            .cloned().collect();
        for id in stale {
            self.remove_session(&id);
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn observe_cowork(&self, update: codeisland_domain::cowork::Update) {
        use codeisland_domain::{PushIntent, cowork};

        let settings = self.settings();
        let session_id = format!("{}{}", cowork::PREFIX, update.metadata.id);
        let (previous, status, in_front) = {
            let mut store = lock(&self.store);
            if !settings.track_claude_desktop_cowork || !update.metadata.trackable()
                || update.metadata.cli_id.as_deref().is_some_and(|id| store.get(id).is_some())
            {
                drop(store);
                self.remove_session(&session_id);
                return;
            }
            let previous = store.get(&session_id).map(|session| session.status);
            let Some(status) = store.observe_cowork(&session_id, &update) else { return };
            let in_front = settings.smart_suppress && self.terminal_is_front(&store, &session_id);
            (previous, status, in_front)
        };
        if !update.live || update.settle {
            self.emit_state();
            self.schedule_persist();
            return;
        }

        let context = AttentionContext {
            sounds_enabled: settings.sound_enabled,
            audible_cues: settings.audible_cues(),
            quiet_hours: settings.is_quiet_now(),
            mute_when_away: settings.mute_when_away,
            away: self.is_away(),
            completion_style: settings.completion_style,
            push_enabled: settings.push_enabled,
            session_in_front: in_front,
        };
        let mut event = AgentEvent::new(&session_id, AgentKind::Claude, AgentEventType::Heartbeat);
        let decision = if update.turns_completed && !status.needs_you() && !status.is_active() {
            if update.audit.interrupted {
                AttentionDecision::default()
            } else {
                event.event_type = if update.audit.failed { AgentEventType::TurnFailed } else { AgentEventType::TurnEnd };
                event.text = update.audit.result.clone().or(update.conversation.response.clone());
                let outcome = ApplyOutcome {
                    applied: true,
                    session_id: session_id.clone(),
                    is_new_session: previous.is_none(),
                    ended_turn: true,
                    turn_failed: update.audit.failed,
                    request: None,
                };
                let decision = attention::decide(&event, &outcome, AttentionSubject::default(), &context);
                self.apply_island_attention(&event, &outcome, &decision);
                decision
            }
        } else if update.permissions_requested && status.needs_you() {
            let (push_kind, reminder_kind) = if status == SessionStatus::WaitingAnswer {
                (PushKind::Question, ReminderKind::Question)
            } else {
                (PushKind::Approval, ReminderKind::Approval)
            };
            if let Some(tool) = update.audit.active_permission() {
                event.tool = Some(tool.name.clone());
                event.text = tool.detail.clone();
            }
            if previous == Some(status) {
                write_lock(&self.island).follow_ups.restart_display_only(reminder_kind, &session_id, Utc::now());
            }
            attention::settle(AttentionIntents {
                sound: Some(attention::CUE_REQUEST.into()),
                push: (previous != Some(status)).then_some(PushIntent { kind: push_kind, smart_suppressible: true }),
                ..AttentionIntents::default()
            }, &context)
        } else if update.prompts_started {
            let fresh = previous.is_none()
                && update.metadata.created_at.is_some_and(|created| Utc::now() - created < TimeDelta::minutes(2));
            attention::settle(AttentionIntents {
                sound: Some(if fresh { attention::CUE_SESSION_START } else { attention::CUE_PROMPT }.into()),
                ..AttentionIntents::default()
            }, &context)
        } else {
            AttentionDecision::default()
        };
        self.emit_state();
        if let Some(push) = decision.push.as_ref()
            && (!push.smart_suppressed || self.is_away_for_push(&settings))
        {
            self.send_push(&event, push.kind, push.smart_suppressed, &settings);
        }
        if decision != AttentionDecision::default() {
            let _ = self.updates.send(RuntimeUpdate::Attention { session_id, decision });
        }
        self.schedule_persist();
    }

    fn admit_codex_event(&self, store: &SessionStore, event: &AgentEvent, from_transcript: bool) -> bool {
        if !event.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX) {
            return true;
        }
        if from_transcript {
            // A rollout backfill must not replace the newer waiting state from
            // the live app-server connection.
            return self.can_tail_codex_desktop(&event.session_id)
                && !store.pending().values().any(|request| {
                    request.session_id == event.session_id
                        && request.agent_id.as_deref() == Some("codex-app-server")
                        && request.created_at >= event.timestamp
                });
        }
        let mut closed = lock(&self.closed_codex_desktop);
        if matches!(event.event_type, AgentEventType::SessionStart | AgentEventType::UserPrompt) {
            closed.remove(&event.session_id);
        }
        !closed.contains_key(&event.session_id)
    }

    fn apply_event(self: &Arc<Self>, mut event: AgentEvent, from_transcript: bool) -> ApplyOutcome {
        if event.branch.is_none() {
            if let Some(cwd) = event.cwd.as_deref() {
                event.branch = crate::process::git_branch(cwd);
            }
        }
        let (scope, approval_config) = if event.event_type == AgentEventType::PermissionRequest {
            codeisland_agents::codex_rules::stage_approval(&event, &self.settings())
        } else {
            (codeisland_domain::ApprovalAlwaysScope::Session, None)
        };
        let (outcome, closed_requests) = {
            let mut store = lock(&self.store);
            if !self.admit_codex_event(&store, &event, from_transcript) {
                return ApplyOutcome {
                    applied: false,
                    session_id: event.session_id,
                    is_new_session: false,
                    ended_turn: false,
                    turn_failed: false,
                    request: None,
                };
            }
            // Check the held rule under the same lock that would queue the
            // request. An Always click can race another hook's arrival.
            if store.session_always_allows(&event) {
                drop(store);
                if let Some(sender) = lock(&self.waiters).remove(&event.id) {
                    let _ = sender.send(Decision {
                        event_id: event.id,
                        session_id: event.session_id.clone(),
                        agent: event.agent,
                        action: UserAction::Approve,
                        text: None,
                    });
                }
                return ApplyOutcome {
                    applied: false,
                    session_id: event.session_id,
                    is_new_session: false,
                    ended_turn: false,
                    turn_failed: false,
                    request: None,
                };
            }
            let before: Vec<_> = store.pending().values()
                .map(|request| (request.event_id.clone(), request.kind))
                .collect();
            let outcome = store.apply_with_approval_scope(&event, scope, approval_config);
            if from_transcript && outcome.applied && event.agent == AgentKind::Claude {
                store.settle_unmonitored_claude(Utc::now(), Some(&outcome.session_id));
            }
            if outcome.applied && !from_transcript && event.agent == AgentKind::Claude
                && !event.session_id.starts_with("ssh:")
                && !event.session_id.starts_with(codeisland_domain::cowork::PREFIX)
                && event.event_type != AgentEventType::SessionEnd
            {
                store.remove_cowork_shadowed_by(&event.session_id);
            }
            let closed = before
                .into_iter()
                .filter(|(id, _)| store.get_pending(id).is_none())
                .collect::<Vec<_>>();
            (outcome, closed)
        };
        if !closed_requests.is_empty() {
            let mut waiters = lock(&self.waiters);
            for (event_id, _) in &closed_requests {
                // Dropping the sender makes the hook fail open immediately.
                waiters.remove(event_id);
            }
        }
        if event.event_type == AgentEventType::PermissionRequest
            && let Some((old_event_id, _)) = closed_requests
                .iter()
                .find(|(_, kind)| *kind == RequestKind::Permission)
        {
            let store = lock(&self.store);
            let mut island = write_lock(&self.island);
            if matches!(&island.surface, IslandSurface::ApprovalCard { event_id, .. } if event_id == old_event_id)
                && let Some(new_request) = store.get_pending(&event.id).filter(|request| !request.hidden)
            {
                // The display stays on the same logical request, but the
                // replacement event must receive a fresh render acknowledgment.
                island.present(IslandSurface::for_request(new_request));
            }
            drop(island);
            drop(store);
            self.emit_state();
            self.schedule_persist();
            return outcome;
        }

        let subject = {
            let store = lock(&self.store);
            match store.get(&event.session_id) {
                Some(session) => AttentionSubject {
                    is_child: session.parent_session_id.is_some(),
                    interrupted: session.interrupted,
                },
                None => AttentionSubject {
                    is_child: event.parent_session_id.is_some(),
                    interrupted: false,
                },
            }
        };

        // Transcript deltas recover state; only hook events ring or push.
        if from_transcript {
            self.emit_state();
            if outcome.applied && (event.transcript_metadata.is_some()
                || (event.agent == AgentKind::Claude && event.event_type == AgentEventType::UserPrompt))
            {
                self.schedule_persist();
            }
            return outcome;
        }

        let settings = self.settings();
        if outcome.applied
            && settings.show_claude_quota
            && event.agent == AgentKind::Claude
            && event.raw_event_name.as_deref() == Some("Stop")
            && !event.session_id.starts_with("ssh:")
        {
            let _ = self.updates.send(RuntimeUpdate::LocalClaudeStop);
        }
        let in_front = {
            let store = lock(&self.store);
            settings.smart_suppress
                && event.raw_event_name.as_deref() != Some("CodexAppServerQuestion")
                && self.terminal_is_front(&store, &event.session_id)
        };
        let context = AttentionContext {
            sounds_enabled: settings.sound_enabled,
            audible_cues: settings.audible_cues(),
            quiet_hours: settings.is_quiet_now(),
            mute_when_away: settings.mute_when_away,
            away: self.away.load(Ordering::Relaxed),
            completion_style: settings.completion_style,
            push_enabled: settings.push_enabled,
            session_in_front: in_front,
        };
        let first = match event.event_type {
            AgentEventType::PermissionRequest | AgentEventType::Question => {
                let kind = if event.event_type == AgentEventType::Question { RequestKind::Question } else { RequestKind::Permission };
                let store = lock(&self.store);
                visible_requests(&store, kind).next().is_some_and(|head| head.event_id == event.id)
            }
            _ => true,
        };
        let decision = attention::decide_for_burst(&event, &outcome, subject, &context, first);

        if outcome.applied {
            self.apply_island_attention(&event, &outcome, &decision);
        }
        self.emit_state();
        // Duplicate/replayed events are ignored by the store; do not re-alert for them.
        if outcome.applied && decision != AttentionDecision::default() {
            if let Some(push) = decision.push.clone() {
                self.send_push(&event, push.kind, push.smart_suppressed, &settings);
            }
            let _ = self.updates.send(RuntimeUpdate::Attention {
                session_id: event.session_id.clone(),
                decision,
            });
        }
        self.schedule_persist();
        outcome
    }

    /// The frontend tells the runtime whether the active session's terminal is
    /// in front, so Smart Suppress can keep pushes and hover expansion quiet.
    pub fn set_session_in_front(&self, in_front: bool) {
        self.session_in_front.store(in_front, Ordering::Relaxed);
    }

    fn send_push(&self, event: &AgentEvent, kind: PushKind, smart_suppressed: bool, settings: &AppSettings) {
        if !self.is_away_for_push(settings) && (smart_suppressed || settings.push_only_when_away) {
            if matches!(kind, PushKind::Approval | PushKind::Question) {
                let store = lock(&self.store);
                if store.get_pending(&event.id).is_some() {
                    let mut held = lock(&self.held_pushes);
                    held.retain(|id, _| store.get_pending(id).is_some());
                    held.insert(event.id.clone(), (event.clone(), kind));
                }
                drop(store);
                self.release_held_pushes();
            }
            return;
        }
        let notifiers = self.push_notifiers(settings);
        if notifiers.is_empty() { return; }
        let message = {
            let store = lock(&self.store);
            let session = store.get(&event.session_id);
            let request = store.get_pending(&event.id);
            let words = push_words(&settings.language);
            let (headline, body) = push_content(kind, request, session, Some(event), words, settings.push_summary_length);
            PushMessage {
                id: event.id.clone(),
                session_id: event.session_id.clone(),
                kind,
                title: push_title(kind, event.agent, session.and_then(|session| session.project.as_deref()), false),
                headline,
                body,
            }
        };
        let weak = self.self_ref.clone();
        spawn_handle().spawn(async move {
            if let Some(runtime) = weak.upgrade() {
                runtime.deliver_push(&notifiers, &message, None).await;
            }
        });
    }

    fn release_held_pushes(&self) {
        if lock(&self.held_pushes).is_empty() {
            return;
        }
        let settings = self.settings();
        if !self.is_away_for_push(&settings) {
            return;
        }
        let queued = std::mem::take(&mut *lock(&self.held_pushes));
        for (_, (event, kind)) in queued {
            if lock(&self.store).get_pending(&event.id).is_some() {
                self.send_push(&event, kind, false, &settings);
            }
        }
    }

    async fn deliver_push(
        &self,
        notifiers: &[PushNotifier],
        message: &PushMessage,
        reminder: Option<&Reminder>,
    ) -> bool {
        let must_deliver = reminder.is_none() && matches!(message.kind, PushKind::Approval | PushKind::Question);
        {
            let mut push = self.push.lock().await;
            if let Some(reminder) = reminder {
                let store = lock(&self.store);
                let island = read_lock(&self.island);
                if !island.follow_ups.scheduler.is_current(reminder)
                    || !still_pending(&store, &reminder.key, reminder.request_id.as_deref())
                    || (reminder.key.kind == ReminderKind::Completion
                        && push.last_completion.get(&message.session_id).is_some_and(|sent_at| {
                            *sent_at >= reminder.waiting_since - TimeDelta::seconds(5)
                        }))
                {
                    return false;
                }
            }
            if !notifiers.iter().any(|notifier| notifier.accepts(message.kind, reminder.is_some())) {
                return false;
            }
            let now = Instant::now();
            push.global.retain(|sent| now.saturating_duration_since(*sent) < Duration::from_secs(60));
            if !must_deliver && push.global.len() >= 12 {
                return false;
            }
            let request = match message.kind {
                PushKind::Approval | PushKind::Question | PushKind::DisplayOnlyWait => &message.id,
                _ => &message.session_id,
            };
            if !push.gate.should_send(&message.id, &format!("{:?}|{request}", message.kind)) {
                return false;
            }
            if !must_deliver {
                push.global.push_back(now);
            }
        }
        let mut sent = false;
        let mut deliveries = tokio::task::JoinSet::new();
        for notifier in notifiers.iter().filter(|notifier| notifier.accepts(message.kind, reminder.is_some())) {
            let notifier = notifier.clone();
            let title = message.title.clone();
            let headline = message.headline.clone();
            let body = message.body.clone();
            let kind = message.kind;
            let reminder = reminder.cloned();
            let event_id = message.id.clone();
            let runtime = self.self_ref.clone();
            deliveries.spawn(async move {
                let Some(runtime) = runtime.upgrade() else { return false; };
                if !runtime.acquire_push_slot(notifier.channel, must_deliver).await {
                    return false;
                }
                if let Some(reminder) = &reminder {
                    let store = lock(&runtime.store);
                    if !still_pending(&store, &reminder.key, reminder.request_id.as_deref()) {
                        return false;
                    }
                } else if must_deliver && lock(&runtime.store).get_pending(&event_id).is_none() {
                    return false;
                }
                notifier.send_for(kind, &title, &headline, &body, reminder.is_some()).await.is_ok()
            });
        }
        while let Some(result) = deliveries.join_next().await {
            sent |= result.unwrap_or(false);
        }
        if sent && reminder.is_none() && matches!(message.kind, PushKind::Completion | PushKind::Error) {
            let mut push = self.push.lock().await;
            let now = Utc::now();
            push.last_completion.retain(|_, sent_at| now - *sent_at < TimeDelta::hours(1));
            push.last_completion.insert(message.session_id.clone(), now);
        }
        sent
    }

    async fn acquire_push_slot(&self, channel: PushChannel, must_deliver: bool) -> bool {
        let limits = channel.rate_limits();
        let Some(maximum) = limits.iter().map(|(_, seconds)| *seconds).max() else {
            return true;
        };
        let grace = Instant::now() + Duration::from_secs(5);
        loop {
            let next = {
                let now = Instant::now();
                let mut push = self.push.lock().await;
                let sends = push.channel_sends.entry(channel).or_default();
                sends.retain(|sent| now.saturating_duration_since(*sent) < Duration::from_secs(maximum));
                let next = limits.iter().filter_map(|(count, seconds)| {
                    let window = Duration::from_secs(*seconds);
                    let index = sends.len().checked_sub(*count)?;
                    let slot = sends[index] + window;
                    (slot > now).then_some(slot)
                }).max();
                let Some(next) = next else {
                    sends.push_back(now);
                    return true;
                };
                next
            };
            if !must_deliver && next > grace {
                return false;
            }
            tokio::time::sleep_until(next.into()).await;
        }
    }

    fn push_notifiers(&self, settings: &AppSettings) -> Vec<PushNotifier> {
        if !settings.push_enabled || std::env::var("CODEISLAND_ISOLATED").as_deref() == Ok("1") {
            return Vec::new();
        }
        if settings.push_only_when_away && !self.is_away_for_push(settings) {
            return Vec::new();
        }
        if settings.push_channels.is_empty() {
            return push_notifier(settings).into_iter().collect();
        }
        let mut seen = HashSet::new();
        settings.push_channels.iter().filter(|channel| channel.enabled)
            .filter_map(|channel| PushNotifier::configured(channel.clone(), settings.push_summary_length))
            .filter(|notifier| seen.insert(notifier.channel.name()))
            .collect()
    }

    /// Applies a blocking event and waits for the user's decision.
    ///
    /// The waiter is registered *before* the event is applied so a fast resolve cannot race
    /// the hook. On timeout (or cancellation) the waiter is removed and `None` is returned,
    /// letting the agent hook fail open.
    pub async fn handle_blocking(
        self: &Arc<Self>,
        event: AgentEvent,
        timeout: Duration,
    ) -> Option<Decision> {
        let event_id = event.id.clone();
        // A replayed hook must fail open instead of replacing the live waiter.
        let (sender, mut receiver) = oneshot::channel();
        {
            let mut waiters = lock(&self.waiters);
            if waiters.contains_key(&event_id) {
                return None;
            }
            waiters.insert(event_id.clone(), sender);
        }

        let mut guard = WaiterGuard {
            runtime: Arc::clone(self),
            event_id: event_id.clone(),
            armed: true,
        };

        if !self.handle_event(event).applied {
            guard.armed = false;
            return receiver.try_recv().ok().or_else(|| {
                lock(&self.waiters).remove(&event_id);
                None
            });
        }

        let decision = match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(decision)) => Some(decision),
            // Sender dropped (waiter replaced) or timeout: fail open.
            Ok(Err(_)) | Err(_) => None,
        };

        if decision.is_some() {
            guard.armed = false;
        }

        decision
    }

    /// Resolves a pending request by id, waking the hook waiter if one exists.
    pub fn resolve(
        &self,
        event_id: &str,
        action: UserAction,
        text: Option<String>,
    ) -> Option<Decision> {
        let (decision, hidden_only) = {
            let mut store = lock(&self.store);
            let existed = store.get_pending(event_id).is_some();
            let decision = store.resolve_with_coverage(event_id, action, text);
            let hidden_only = existed && action == UserAction::Hide && decision.is_none();
            (decision, hidden_only)
        };

        match decision {
            Some(decision) => Some(self.complete_resolution(decision)),
            None => {
                // `Hide` mutates the store but yields no decision; the UI still needs the update.
                if hidden_only {
                    self.emit_state();
                }
                None
            }
        }
    }

    /// Resolves the first visible pending request.
    pub fn resolve_current(&self, action: UserAction, text: Option<String>) -> Option<Decision> {
        let event_id = {
            let store = lock(&self.store);
            store
                .pending()
                .values()
                .find(|request| !request.hidden && request.accepts(action))
                .map(|request| request.event_id.clone())
        };
        event_id.and_then(|event_id| self.resolve(&event_id, action, text))
    }

    /// Hides a pending request without unblocking the agent.
    pub fn hide(&self, event_id: &str) -> bool {
        let hidden = {
            let mut store = lock(&self.store);
            if store.get_pending(event_id).is_some() {
                let _ = store.resolve(event_id, UserAction::Hide, None);
                true
            } else {
                false
            }
        };
        if hidden {
            self.emit_state();
        }
        hidden
    }

    /// Shows a previously hidden request again.
    pub fn unhide(&self, event_id: &str) -> bool {
        let unhidden = { lock(&self.store).unhide(event_id) };
        if unhidden {
            self.emit_state();
        }
        unhidden
    }

    /// Removes a session (and its children/pending requests). Hooks waiting on that
    /// session's requests fail open immediately.
    pub fn remove_session(&self, session_id: &str) -> bool {
        let (removed, waiter_ids) = {
            let mut store = lock(&self.store);
            let before: Vec<String> = store.pending().keys().cloned().collect();
            let removed = store.remove_session(session_id);
            let waiter_ids: Vec<String> = before
                .into_iter()
                .filter(|id| store.get_pending(id).is_none())
                .collect();
            (removed, waiter_ids)
        };
        if removed {
            if !waiter_ids.is_empty() {
                let mut waiters = lock(&self.waiters);
                for event_id in waiter_ids {
                    waiters.remove(&event_id);
                }
            }
            self.emit_state();
            self.schedule_persist();
        }
        removed
    }

    pub fn remove_idle_sessions(&self, groups: Vec<Vec<String>>) -> usize {
        let removed = {
            let mut store = lock(&self.store);
            if read_lock(&self.island).surface != IslandSurface::SessionList {
                return 0;
            }
            store.remove_idle_sessions(groups, Utc::now())
        };
        if removed > 0 {
            self.emit_state();
            self.schedule_persist();
        }
        removed
    }

    /// Reorders a session before another one in the manual order.
    pub fn move_session(&self, session_id: &str, before_id: &str) -> bool {
        let moved = { lock(&self.store).move_before(session_id, before_id) };
        if moved {
            self.emit_state();
            self.schedule_persist();
        }
        moved
    }

    /// Removes every completed/failed/cancelled session that is not waiting for the user.
    pub fn dismiss_completed(&self) -> usize {
        let ids: Vec<String> = {
            let store = lock(&self.store);
            store
                .sessions()
                .iter()
                .filter(|(_, session)| {
                    matches!(
                        session.status,
                        SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled
                    )
                })
                .filter(|(_, session)| store.pending_for(&session.session_id).is_empty())
                .map(|(id, _)| id.clone())
                .collect()
        };
        if ids.is_empty() {
            return 0;
        }

        let removed = {
            let mut store = lock(&self.store);
            let before = store.session_count();
            for id in &ids {
                store.remove_session(id.as_str());
            }
            before - store.session_count()
        };
        if removed > 0 {
            self.emit_state();
            self.schedule_persist();
        }
        removed
    }

    /// Subscribes to runtime updates.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeUpdate> {
        self.updates.subscribe()
    }

    /// Starts the IPC server, the cleanup task and the Codex tailer.
    ///
    /// Works both inside an async context (Tauri commands, tests) and from a
    /// synchronous caller such as Tauri's `setup`, in which case a small
    /// process-wide fallback runtime is used.
    pub fn start(self: &Arc<Self>) -> RuntimeHandle {
        let handle = spawn_handle();
        let running = Arc::new(AtomicBool::new(true));
        let mut tasks = Vec::with_capacity(5);

        tasks.push(ipc_handler::spawn(Arc::clone(self), handle.clone()));

        #[cfg(windows)]
        if std::env::var("CODEISLAND_ISOLATED").as_deref() != Ok("1") {
            tasks.push(handle.spawn(crate::codex_app_server::watch(Arc::clone(self))));
            tasks.push(handle.spawn(crate::cowork_watcher::watch(Arc::clone(self))));
            tasks.push(handle.spawn(crate::provider_discovery::watch(Arc::clone(self))));
        }

        let reminder_runtime = Arc::clone(self);
        tasks.push(handle.spawn(async move { reminder_runtime.run_follow_ups().await }));

        let cleanup_runtime = Arc::clone(self);
        let cleanup_running = Arc::clone(&running);
        tasks.push(handle.spawn(async move {
            while cleanup_running.load(Ordering::SeqCst) {
                tokio::time::sleep(CLEANUP_INTERVAL).await;
                if !cleanup_running.load(Ordering::SeqCst) {
                    break;
                }
                cleanup_runtime.run_cleanup();
            }
        }));

        if std::env::var("CODEISLAND_ISOLATED").as_deref() != Ok("1") {
            tasks.push(tailer::spawn(
                Arc::clone(self),
                Arc::clone(&running),
                handle,
            ));
        }

        RuntimeHandle { running, tasks }
    }

    fn complete_resolution(&self, resolution: RequestResolution) -> Decision {
        if let Some(request) = resolution.approval_to_save.as_ref() {
            codeisland_agents::codex_rules::persist_approved_scope(request);
        }
        {
            let mut waiters = lock(&self.waiters);
            for decision in std::iter::once(&resolution.decision).chain(resolution.covered.iter()) {
                if let Some(sender) = waiters.remove(&decision.event_id) {
                    let _ = sender.send(decision.clone());
                }
            }
        }
        self.emit_state();
        self.schedule_persist();
        resolution.decision
    }

    fn terminal_is_front(&self, store: &SessionStore, session_id: &str) -> bool {
        let detector = read_lock(&self.terminal_visibility).clone();
        match detector {
            Some(detector) => store.get(session_id).is_some_and(|session| detector(session)),
            None => self.session_in_front.load(Ordering::Relaxed),
        }
    }

    pub fn set_terminal_tab_visibility_detector(
        &self,
        detector: impl Fn(&SessionSnapshot) -> bool + Send + Sync + 'static,
    ) {
        *write_lock(&self.terminal_tab_visibility) = Some(Arc::new(detector));
    }

    fn terminal_tab_is_visible(&self, store: &SessionStore, session_id: &str) -> bool {
        self.terminal_is_front(store, session_id)
            && read_lock(&self.terminal_tab_visibility).as_ref().is_none_or(|detector| {
                store.get(session_id).is_some_and(|session| detector(session))
            })
    }

    fn may_open_request(
        &self,
        store: &SessionStore,
        settings: &AppSettings,
        request: &PendingRequest,
    ) -> bool {
        let allowed = match request.kind {
            RequestKind::Permission => settings.auto_expand_on_permission,
            RequestKind::Question => settings.auto_expand_on_question,
        };
        allowed
            && (!settings.smart_suppress
                || (request.kind == RequestKind::Question
                    && request.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX)
                    && request.tool.as_deref() == Some("request_user_input"))
                || (request.kind == RequestKind::Question
                    && request.tool.as_deref() == Some("AskUserQuestion")
                    && (request.agent != AgentKind::Pi || !request.native_ask_racing))
                || !self.terminal_is_front(store, &request.session_id))
    }

    /// Keep the exact visible card in place, otherwise consider the oldest
    /// eligible request. Permissions precede questions when advancing the queue.
    fn show_next_pending(&self, store: &SessionStore, island: &mut IslandPresentation) -> bool {
        let shown = island.surface.current_request(store);
        if shown.is_some_and(|request| request.kind == RequestKind::Permission) {
            return true;
        }
        if let Some(permission) = visible_requests(store, RequestKind::Permission).next() {
            if island.surface == IslandSurface::SessionList {
                return true;
            }
            if self.may_open_request(store, &island.settings, permission) {
                island.present(IslandSurface::for_request(permission));
                return true;
            }
            return shown.is_some();
        }
        if let Some(question) = visible_requests(store, RequestKind::Question).next() {
            if shown.is_some() {
                return true;
            }
            if self.may_open_request(store, &island.settings, question) {
                island.present(IslandSurface::for_request(question));
                return true;
            }
            return island.surface == IslandSurface::SessionList;
        }
        false
    }

    fn show_next_completion(&self, store: &SessionStore, island: &mut IslandPresentation) -> bool {
        while let Some(session_id) = island.completion_queue.pop_front() {
            if store.get(&session_id).is_some()
                && (!island.settings.smart_suppress || !self.terminal_tab_is_visible(store, &session_id))
            {
                island.present(IslandSurface::CompletionCard { session_id });
                return true;
            }
        }
        false
    }

    fn apply_island_attention(
        &self,
        event: &AgentEvent,
        outcome: &ApplyOutcome,
        decision: &AttentionDecision,
    ) {
        let store = lock(&self.store);
        let mut island = write_lock(&self.island);
        if let Some(request) = &outcome.request {
            match request.kind {
                RequestKind::Permission => {
                    self.show_next_pending(&store, &mut island);
                }
                RequestKind::Question => {
                    let first = visible_requests(&store, RequestKind::Question).next();
                    if let Some(head) = first
                        && head.event_id == request.event_id
                        && self.may_open_request(&store, &island.settings, head)
                    {
                        island.present(IslandSurface::for_request(head));
                    }
                }
            }
        }
        if let Some(FollowUpIntent::TrackCompletion { turn_failed }) = decision.follow_up {
            let now = Utc::now();
            let minutes = island.settings.follow_up_reminder_minutes;
            island.follow_ups.sync(&store, minutes, now);
            island.follow_ups.scheduler.track_completion(&event.session_id, now, turn_failed);
        }
        let Some(completion) = &decision.completion else {
            return;
        };
        match completion.presentation {
            CompletionPresentation::StatusOnly => {}
            CompletionPresentation::Glance => island.flash_completion(),
            CompletionPresentation::Card => {
                if island.surface == (IslandSurface::CompletionCard { session_id: event.session_id.clone() })
                    || island.completion_queue.contains(&event.session_id)
                {
                    return;
                }
                if matches!(island.surface, IslandSurface::CompletionCard { .. })
                    || island.surface.current_request(&store).is_some()
                {
                    island.completion_queue.push_back(event.session_id.clone());
                } else if store.get(&event.session_id).is_some()
                    && (!island.settings.smart_suppress || !self.terminal_tab_is_visible(&store, &event.session_id))
                {
                    island.present(IslandSurface::CompletionCard {
                        session_id: event.session_id.clone(),
                    });
                }
            }
        }
    }

    async fn run_follow_ups(&self) {
        loop {
            self.tick_follow_ups(Utc::now());
            let now = Utc::now();
            let delay = {
                let island = read_lock(&self.island);
                let scheduler = &island.follow_ups.scheduler;
                let wake = scheduler.next_wake_date();
                let recheck = scheduler.has_owed().then(|| now + TimeDelta::minutes(1));
                wake.into_iter().chain(recheck).min()
                    .map(|next| (next - now).to_std().unwrap_or_default())
            };
            if let Some(delay) = delay {
                tokio::select! {
                    () = self.follow_up_wake.notified() => {}
                    () = tokio::time::sleep(delay) => {}
                }
            } else {
                self.follow_up_wake.notified().await;
            }
        }
    }

    fn tick_follow_ups(&self, now: DateTime<Utc>) {
        let (context, notifiers, local, remote) = {
            let store = lock(&self.store);
            let mut island = write_lock(&self.island);
            let settings = island.settings.clone();
            if settings.follow_up_reminder_minutes == 0 {
                return;
            }
            let away = self.is_away();
            let context = AttentionContext {
                sounds_enabled: settings.sound_enabled,
                audible_cues: settings.audible_cues(),
                quiet_hours: settings.is_quiet_now(),
                mute_when_away: settings.mute_when_away,
                away,
                ..AttentionContext::default()
            };
            let notifiers = self.push_notifiers(&settings);
            let minutes = settings.follow_up_reminder_minutes;
            island.follow_ups.sync(&store, minutes, now);
            let due = island.follow_ups.scheduler.collect_due(now, context.holds_sounds());
            let mut local = Vec::new();
            let mut remote = Vec::new();
            for reminder in due {
                if !still_pending(&store, &reminder.key, reminder.request_id.as_deref()) {
                    continue;
                }
                let suppressed = reminder.delivery == Delivery::OnTime
                    && (island.is_looking_at(&reminder)
                        || (island.settings.smart_suppress
                            && self.terminal_tab_is_visible(&store, &reminder.key.session_id)));
                let should_push = !notifiers.is_empty() && reminder.delivery != Delivery::CatchUp
                    && (!suppressed || self.is_away_for_push(&settings));
                let push = should_push.then(|| reminder_push(&store, &reminder, now, &settings)).flatten();
                if suppressed && push.is_none() {
                    island.follow_ups.scheduler.postpone(&reminder, now);
                    continue;
                }
                if let Some(push) = push {
                    remote.push((reminder.clone(), push, suppressed));
                }
                if reminder.delivery != Delivery::Deferred && !suppressed {
                    local.push(reminder);
                }
            }

            let reopened = local.iter().find(|reminder| {
                reminder.key.kind != ReminderKind::Completion
                    && reminder.origin == ReminderOrigin::Island
            }).filter(|_| island.surface.is_collapsed()).and_then(|reminder| {
                let auto_expand = match reminder.key.kind {
                    ReminderKind::Approval => island.settings.auto_expand_on_permission,
                    ReminderKind::Question => island.settings.auto_expand_on_question,
                    ReminderKind::Completion => false,
                };
                let request = reminder_request(&store, reminder).filter(|_| auto_expand)?;
                island.present(IslandSurface::for_request(request));
                Some(&reminder.key)
            });
            island.follow_ups.hint(local.iter().filter(|reminder| Some(&reminder.key) != reopened).cloned());
            (context, notifiers, local, remote)
        };

        if !local.is_empty() {
            self.emit_state();
        }
        let mut played = HashSet::new();
        for reminder in local {
            let cue = reminder.key.kind.sound(reminder.turn_failed);
            if !played.insert(cue) {
                continue;
            }
            let decision = attention::settle(AttentionIntents {
                sound: Some(cue.to_owned()),
                ..AttentionIntents::default()
            }, &context);
            if decision.sound.is_some() {
                let _ = self.updates.send(RuntimeUpdate::Attention {
                    session_id: reminder.key.session_id,
                    decision,
                });
            }
        }
        if !remote.is_empty() {
            let weak = self.self_ref.clone();
            spawn_handle().spawn(async move {
                for (reminder, message, suppressed) in remote {
                    let Some(runtime) = weak.upgrade() else { return };
                    let sent = runtime.deliver_push(&notifiers, &message, Some(&reminder)).await;
                    if suppressed && !sent {
                        write_lock(&runtime.island).follow_ups.scheduler.postpone(&reminder, now);
                        runtime.follow_up_wake.notify_one();
                    }
                }
            });
        }
    }

    fn expire_completion(&self, revision: u32) {
        {
            let store = lock(&self.store);
            let mut island = write_lock(&self.island);
            if island.revision != revision
                || island.completion_entered
                || !matches!(island.surface, IslandSurface::CompletionCard { .. })
            {
                return;
            }
            if !self.show_next_pending(&store, &mut island)
                && !self.show_next_completion(&store, &mut island)
            {
                island.present(IslandSurface::Collapsed);
            }
        }
        self.emit_state();
    }

    fn expire_glance(&self, revision: u32) {
        {
            let mut island = write_lock(&self.island);
            if island.glance_revision != revision || !island.glance_completion_active {
                return;
            }
            island.glance_completion_active = false;
        }
        self.emit_state();
    }

    fn emit_state(&self) {
        let (snapshot, completion_timeout, glance_timeout) = {
            let store = lock(&self.store);
            let mut island = write_lock(&self.island);
            if !island.surface.is_available(&store)
                && !self.show_next_pending(&store, &mut island)
                && !self.show_next_completion(&store, &mut island)
            {
                island.present(IslandSurface::Collapsed);
            }
            let minutes = island.settings.follow_up_reminder_minutes;
            island.follow_ups.sync(&store, minutes, Utc::now());
            (
                store.snapshot(),
                island.completion_timeout.take(),
                island.glance_timeout.take(),
            )
        };
        let _ = self.updates.send(RuntimeUpdate::State(snapshot));
        self.follow_up_wake.notify_one();
        if let Some(revision) = completion_timeout {
            let weak = self.self_ref.clone();
            spawn_handle().spawn(async move {
                tokio::time::sleep(COMPLETION_VISIBLE).await;
                if let Some(runtime) = weak.upgrade() {
                    runtime.expire_completion(revision);
                }
            });
        }
        if let Some(revision) = glance_timeout {
            let weak = self.self_ref.clone();
            spawn_handle().spawn(async move {
                tokio::time::sleep(GLANCE_VISIBLE).await;
                if let Some(runtime) = weak.upgrade() {
                    runtime.expire_glance(revision);
                }
            });
        }
    }

    /// Settles abandoned local turns and removes expired or exited sessions.
    fn run_cleanup(&self) -> usize {
        let settings = self.settings();
        let now = Utc::now();
        let cutoff = (settings.session_cleanup_minutes > 0).then(|| {
            now - chrono::Duration::minutes(i64::from(settings.session_cleanup_minutes))
        });

        let candidates: Vec<_> = {
            let store = lock(&self.store);
            store
                .sessions()
                .iter()
                .filter(|(id, session)| {
                    !session.status.needs_you() && !id.starts_with(CODEX_DESKTOP_SESSION_PREFIX)
                        && session.wsl_process.is_none()
                })
                .filter(|(_, session)| store.pending_for(&session.session_id).is_empty())
                .filter_map(|(id, session)| {
                    session.pid.map(|pid| (id.clone(), pid, session.discovered_pid_started_at))
                })
                .collect()
        };
        let dead: Vec<_> = candidates
            .into_iter()
            .filter(|(_, pid, started)| !process::is_session_process_alive(*pid, *started))
            .collect();

        let (removed, settled) = {
            let mut store = lock(&self.store);
            let settled = store.settle_unmonitored_claude(now, None);
            let mut removed = cutoff.map(|cutoff| store.remove_expired(cutoff)).unwrap_or(0);
            for (id, pid, started) in dead {
                // A concurrent hook or scan may have attached a different process.
                let still_removable = store.get(&id).is_some_and(|session| {
                    !session.status.needs_you()
                        && session.pid == Some(pid)
                        && session.discovered_pid_started_at == started
                }) && store.pending_for(&id).is_empty();
                if still_removable && store.remove_session(&id) {
                    removed += 1;
                }
            }
            (removed, settled)
        };

        if removed > 0 || settled {
            self.emit_state();
            self.schedule_persist();
        }
        removed
    }

    /// Marks the store dirty and coalesces writes into one 250 ms debounced task.
    fn schedule_persist(&self) {
        self.dirty.store(true, Ordering::SeqCst);
        if self.persist_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }

        let Some(runtime) = self.self_ref.upgrade() else {
            self.persist_scheduled.store(false, Ordering::SeqCst);
            self.persist_if_dirty();
            return;
        };

        let task = async move {
            tokio::time::sleep(PERSIST_DEBOUNCE).await;
            runtime.persist_scheduled.store(false, Ordering::SeqCst);
            runtime.persist_if_dirty();
        };
        spawn_handle().spawn(task);
    }

    fn persist_if_dirty(&self) {
        if !self.dirty.swap(false, Ordering::SeqCst) {
            return;
        }
        self.persist_now();
    }

    fn persist_now(&self) {
        let state = { lock(&self.store).persisted() };
        if let Err(error) = persistence::save_state(&state) {
            tracing::warn!(%error, "failed to persist session state");
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // Best-effort final flush; the debounced task may not get a chance to run on shutdown.
        if self.dirty.swap(false, Ordering::SeqCst) {
            let state = lock(&self.store).persisted();
            if let Err(error) = persistence::save_state(&state) {
                tracing::warn!(%error, "failed to flush session state on shutdown");
            }
        }
    }
}

/// Keeps the runtime alive while a hook is blocked and removes the waiter if the future is
/// cancelled or times out.
struct WaiterGuard {
    runtime: Arc<Runtime>,
    event_id: String,
    armed: bool,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        if self.armed && lock(&self.runtime.waiters).remove(&self.event_id).is_some() {
            let discarded = { lock(&self.runtime.store).discard_pending(&self.event_id) };
            if discarded {
                self.runtime.emit_state();
                self.runtime.schedule_persist();
            }
        }
    }
}

/// Shutdown handle for the background services. Dropping it stops the tasks.
pub struct RuntimeHandle {
    running: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
}

impl RuntimeHandle {
    /// Whether the handle's tasks are still expected to be running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }
}

fn push_notifier(settings: &AppSettings) -> Option<PushNotifier> {
    if !settings.push_enabled || settings.push_target.trim().is_empty() {
        return None;
    }
    let channel = PushChannel::from_name(&settings.push_channel)?;
    Some(PushNotifier::new(channel, settings.push_target.clone()))
}

fn reminder_push(store: &SessionStore, reminder: &Reminder, now: DateTime<Utc>, settings: &AppSettings) -> Option<PushMessage> {
    let session = store.get(&reminder.key.session_id)?;
    let request = reminder_request(store, reminder);
    let kind = match reminder.key.kind {
        ReminderKind::Approval => PushKind::Approval,
        ReminderKind::Question => PushKind::Question,
        ReminderKind::Completion if reminder.turn_failed => return None,
        ReminderKind::Completion => PushKind::Completion,
    };
    let words = push_words(&settings.language);
    let (pending, details) = push_content(kind, request, Some(session), None, words, settings.push_summary_length);
    let minutes = (now - reminder.waiting_since).num_minutes();
    let headline = if minutes > 0 {
        format!("{} · {} · {pending}", words.reminder, words.minutes.replace("%d", &minutes.to_string()))
    } else {
        format!("{} · {pending}", words.reminder)
    };
    Some(PushMessage {
        id: format!(
            "follow-up:{:?}:{}:{}:{}",
            reminder.key.kind,
            reminder.request_id.as_deref().unwrap_or(&reminder.key.session_id),
            reminder.waiting_since.timestamp_millis(),
            reminder.attempt,
        ),
        session_id: session.session_id.clone(),
        kind,
        title: push_title(kind, session.agent, session.project.as_deref(), true),
        headline,
        body: details,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The tokio handle used for background services.
///
/// Tauri's `setup` hook runs on the main thread without an entered tokio
/// context, so fall back to a lazily created process-wide runtime there.
fn spawn_handle() -> tokio::runtime::Handle {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => fallback_runtime().handle().clone(),
    }
}

fn fallback_runtime() -> &'static tokio::runtime::Runtime {
    static FALLBACK: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    FALLBACK.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("codeisland-runtime")
            .build()
            .expect("failed to build the CodeIsland background runtime")
    })
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use codeisland_domain::{AgentEvent, AgentEventType, AgentKind};

    fn use_temp_state_dir() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let dir = std::env::temp_dir()
                .join(format!("codeisland-runtime-tests-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp state dir");
            // SAFETY: set once, before any runtime is created in this test binary.
            unsafe {
                std::env::set_var("CODEISLAND_STATE_DIR", &dir);
            }
        });
    }

    pub(crate) fn runtime_with(settings: AppSettings) -> Arc<Runtime> {
        use_temp_state_dir();
        Runtime::new_for_test(settings)
    }

    async fn wait_for_pending(runtime: &Arc<Runtime>, event_id: &str) {
        for _ in 0..200 {
            if runtime
                .state()
                .store
                .pending
                .iter()
                .any(|request| request.event_id == event_id)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("pending request {event_id} never appeared");
    }

    #[test]
    fn runtime_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        fn assert_send<T: Send>() {}
        assert_send_sync::<Arc<Runtime>>();
        assert_send::<RuntimeHandle>();
    }

    #[test]
    fn custom_sound_selection_survives_a_stale_settings_form_and_can_be_reset() {
        let runtime = runtime_with(AppSettings::default());
        let mut stale = runtime.settings();
        runtime.set_custom_sound_path("8bit_start", Some("C:/sounds/quiet.wav".into())).unwrap();

        stale.sound_volume = 15;
        let saved = runtime.update_settings(stale);
        assert_eq!(saved.sound_volume, 15);
        assert_eq!(
            saved.custom_sound_paths.get("8bit_start").map(String::as_str),
            Some("C:/sounds/quiet.wav"),
        );

        runtime.set_custom_sound_path("8bit_start", None).unwrap();
        assert!(runtime.settings().custom_sound_paths.is_empty());
    }

    #[test]
    fn settings_write_failure_preserves_live_values_and_emits_no_success() {
        use_temp_state_dir();
        let path = persistence::state_dir().join("transaction-settings.json");
        std::fs::create_dir(&path).unwrap();
        let runtime = Runtime::build_with_settings_path(
            AppSettings::default(), PersistedState::default(), path.clone(),
        );
        let mut updates = runtime.subscribe();
        let previous = runtime.settings();
        let change = |settings: &AppSettings| Ok(AppSettings {
            sound_enabled: !previous.sound_enabled,
            ..settings.clone()
        });

        assert!(runtime.try_update_settings(change).is_err());
        assert_eq!(runtime.settings(), previous);
        assert!(matches!(updates.try_recv(), Err(broadcast::error::TryRecvError::Empty)));

        std::fs::remove_dir(&path).unwrap();
        let saved = runtime.try_update_settings(change).unwrap();
        assert_ne!(saved.sound_enabled, previous.sound_enabled);
        assert_eq!(persistence::load_settings_from(&path), Some(saved.clone()));
        assert!(matches!(updates.try_recv(), Ok(RuntimeUpdate::Settings(settings)) if settings == saved));
    }

    #[test]
    fn collapsing_emits_state_without_a_preferences_update() {
        let mut settings = AppSettings::default();
        let runtime = runtime_with(settings.clone());
        let mut updates = runtime.subscribe();
        settings.collapsed = !settings.collapsed;

        runtime.set_collapsed(settings.collapsed);

        assert_eq!(runtime.settings(), settings);
        assert!(matches!(updates.try_recv(), Ok(RuntimeUpdate::State(_))));
        assert!(matches!(
            updates.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn shortcut_reveals_exact_permission_before_resolving_and_leaves_question_waiting() {
        let runtime = runtime_with(AppSettings {
            collapsed: true,
            auto_expand_on_permission: false,
            auto_expand_on_question: false,
            ..AppSettings::default()
        });
        let question = AgentEvent::new("question", AgentKind::Claude, AgentEventType::Question);
        let approval = AgentEvent::new(
            "permission",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        );
        runtime.handle_event(question.clone());
        runtime.handle_event(approval.clone());

        let target = IslandSurface::ApprovalCard {
            event_id: approval.id.clone(),
            session_id: approval.session_id,
        };
        let reveal = CardShortcutOutcome::Opened(target.clone());
        assert_eq!(runtime.perform_card_shortcut(CardShortcut::Approve, true), reveal);
        assert_eq!(runtime.state().store.pending.len(), 2);
        assert!(runtime.set_surface(target.clone()));

        let revision = runtime.state().surface_revision;
        assert!(!runtime.acknowledge_surface(&question.id, revision));
        assert_eq!(runtime.perform_card_shortcut(CardShortcut::Approve, true), reveal);
        assert!(runtime.acknowledge_surface(&approval.id, revision));
        assert_eq!(runtime.perform_card_shortcut(CardShortcut::Approve, false), reveal);
        assert!(runtime.set_surface(target));
        assert!(!runtime.acknowledge_surface(&approval.id, revision));
        let revision = runtime.state().surface_revision;
        assert!(runtime.acknowledge_surface(&approval.id, revision));
        assert_eq!(
            runtime.perform_card_shortcut(CardShortcut::Approve, true),
            CardShortcutOutcome::Acted
        );
        let remaining = runtime.state();
        assert_eq!(remaining.surface, IslandSurface::Collapsed);
        assert_eq!(remaining.store.pending.len(), 1);
        assert_eq!(remaining.store.pending[0].event_id, question.id);
    }

    #[test]
    fn dismissed_question_head_cannot_be_skipped_unseen() {
        let runtime = runtime_with(AppSettings {
            collapsed: true,
            auto_expand_on_question: false,
            ..AppSettings::default()
        });
        let head = AgentEvent::new("question", AgentKind::Claude, AgentEventType::Question);
        let next = AgentEvent::new("question", AgentKind::Claude, AgentEventType::Question);
        runtime.handle_event(head.clone());
        runtime.handle_event(next.clone());
        assert!(runtime.hide(&head.id));
        assert_eq!(
            runtime.perform_card_shortcut(CardShortcut::SkipQuestion, true),
            CardShortcutOutcome::Ignored
        );
        assert!(runtime.open_pending_question(None));
        let ui = runtime.state();
        assert!(runtime.acknowledge_surface(&head.id, ui.surface_revision));
        assert_eq!(
            runtime.perform_card_shortcut(CardShortcut::SkipQuestion, true),
            CardShortcutOutcome::Acted
        );
        assert_eq!(runtime.state().store.pending[0].event_id, next.id);
    }

    #[test]
    fn permission_keeps_its_surface_and_then_advances_completion_queue() {
        let runtime = runtime_with(AppSettings {
            collapsed: true,
            smart_suppress: false,
            ..AppSettings::default()
        });
        let permission = AgentEvent::new(
            "permission",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        );
        runtime.handle_event(permission.clone());
        for session in ["first", "second"] {
            runtime.handle_event(AgentEvent::new(session, AgentKind::Claude, AgentEventType::UserPrompt));
            runtime.handle_event(AgentEvent::new(session, AgentKind::Claude, AgentEventType::TurnEnd));
        }
        assert!(matches!(runtime.surface(), IslandSurface::ApprovalCard { .. }));
        runtime.resolve(&permission.id, UserAction::Approve, None);
        assert_eq!(
            runtime.surface(),
            IslandSurface::CompletionCard { session_id: "first".into() }
        );
        let revision = runtime.state().surface_revision;
        runtime.expire_completion(revision);
        assert_eq!(
            runtime.surface(),
            IslandSurface::CompletionCard { session_id: "second".into() }
        );
        let revision = runtime.state().surface_revision;
        runtime.set_pointer_over_island(true);
        runtime.expire_completion(revision);
        assert_eq!(
            runtime.surface(),
            IslandSurface::CompletionCard { session_id: "second".into() }
        );
        runtime.set_collapsed(true);
        runtime.expire_completion(revision);
        assert_eq!(runtime.surface(), IslandSurface::Collapsed);
    }

    #[test]
    fn smart_suppress_uses_request_session_and_keeps_blocking_question_available() {
        let runtime = runtime_with(AppSettings {
            collapsed: true,
            smart_suppress: true,
            ..AppSettings::default()
        });
        runtime.set_terminal_visibility_detector(|session| session.session_id == "frontmost");
        runtime.handle_event(AgentEvent::new(
            "frontmost",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        ));
        assert_eq!(runtime.surface(), IslandSurface::Collapsed);
        let mut question = AgentEvent::new("frontmost", AgentKind::Claude, AgentEventType::Question);
        question.tool = Some("AskUserQuestion".into());
        runtime.handle_event(question.clone());
        assert_eq!(
            runtime.surface(),
            IslandSurface::QuestionCard {
                session_id: "frontmost".into(),
                event_id: question.id,
            }
        );
    }

    #[tokio::test]
    async fn follow_up_defers_while_away_then_reopens_only_the_waiting_request() {
        let runtime = runtime_with(AppSettings {
            collapsed: true,
            follow_up_reminder_minutes: 1,
            ..AppSettings::default()
        });
        let question = AgentEvent::new("question", AgentKind::Claude, AgentEventType::Question);
        runtime.handle_event(question.clone());
        runtime.set_collapsed(true);
        let now = Utc::now();

        runtime.set_away(true);
        runtime.tick_follow_ups(now + TimeDelta::seconds(61));
        assert!(runtime.surface().is_collapsed());
        assert!(!runtime.state().follow_up_hint_active);

        runtime.set_away(false);
        runtime.tick_follow_ups(now + TimeDelta::seconds(62));
        assert_eq!(runtime.surface(), IslandSurface::QuestionCard {
            session_id: question.session_id.clone(),
            event_id: question.id.clone(),
        });
        runtime.resolve(&question.id, UserAction::Skip, None);
        runtime.tick_follow_ups(now + TimeDelta::minutes(5));
        assert!(runtime.surface().is_collapsed());
        assert!(!runtime.state().follow_up_hint_active);
    }

    #[tokio::test]
    async fn follow_up_respects_hidden_and_replaced_requests() {
        let runtime = runtime_with(AppSettings {
            collapsed: true,
            auto_expand_on_question: false,
            follow_up_reminder_minutes: 1,
            ..AppSettings::default()
        });
        let first = AgentEvent::new("session", AgentKind::Claude, AgentEventType::Question);
        let mut next = AgentEvent::new("session", AgentKind::Claude, AgentEventType::Question);
        next.tool = Some("AskUserQuestion".into());
        runtime.handle_event(first.clone());
        let now = Utc::now();
        runtime.tick_follow_ups(now + TimeDelta::seconds(61));
        assert!(runtime.state().follow_up_hint_active);
        assert_eq!(runtime.state().follow_up_hint_pulse, 1);

        runtime.handle_event(next.clone());
        assert!(!runtime.state().follow_up_hint_active);
        runtime.resolve(&next.id, UserAction::Hide, None);
        runtime.tick_follow_ups(now + TimeDelta::minutes(5));
        assert!(!runtime.state().follow_up_hint_active);
        assert_eq!(runtime.state().follow_up_hint_pulse, 1);
    }

    #[tokio::test]
    async fn follow_up_completion_stops_when_seen_or_jumped_to() {
        let runtime = runtime_with(AppSettings {
            collapsed: true,
            follow_up_reminder_minutes: 1,
            ..AppSettings::default()
        });
        let end = |id| AgentEvent::new(id, AgentKind::Claude, AgentEventType::TurnEnd);
        runtime.handle_event(end("shown"));
        runtime.set_pointer_over_island(true);
        runtime.set_pointer_over_island(false);
        runtime.set_collapsed(true);

        runtime.handle_event(end("in-list"));
        runtime.set_collapsed(false);
        runtime.set_collapsed(true);

        runtime.handle_event(end("jumped"));
        runtime.user_jumped_to_session("jumped");
        runtime.set_collapsed(true);

        runtime.handle_event(end("unseen"));
        runtime.set_collapsed(true);
        let now = Utc::now();
        runtime.tick_follow_ups(now + TimeDelta::seconds(61));
        assert!(runtime.state().follow_up_hint_active);
        assert_eq!(runtime.state().follow_up_hint_pulse, 1);
        assert!(runtime.surface().is_collapsed());

        runtime.remove_session("unseen");
        assert!(!runtime.state().follow_up_hint_active);
        runtime.tick_follow_ups(now + TimeDelta::minutes(4));
        assert_eq!(runtime.state().follow_up_hint_pulse, 1);
    }

    #[test]
    fn hook_with_transcript_path_still_opens_and_transcript_replay_stays_silent() {
        let settings = AppSettings { collapsed: true, ..AppSettings::default() };
        let hook = runtime_with(settings.clone());
        let transcript = runtime_with(settings);
        let mut event = AgentEvent::new("claude", AgentKind::Claude, AgentEventType::TurnEnd);
        event.transcript_path = Some("projects/session.jsonl".into());

        assert!(hook.handle_event(event.clone()).applied);
        assert_eq!(hook.surface(), IslandSurface::CompletionCard { session_id: "claude".into() });

        assert!(transcript.handle_transcript_event(event).applied);
        assert_eq!(transcript.surface(), IslandSurface::Collapsed);
        assert_eq!(transcript.state().store.sessions[0].status, SessionStatus::Completed);
    }

    #[tokio::test]
    async fn blocking_decision_round_trip() {
        let runtime = runtime_with(AppSettings::default());
        let mut event = AgentEvent::new(
            "blocking-session",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        );
        event.tool = Some("Bash".to_owned());
        let event_id = event.id.clone();

        let waiter_runtime = Arc::clone(&runtime);
        let waiter = tokio::spawn(async move {
            waiter_runtime
                .handle_blocking(event, Duration::from_secs(5))
                .await
        });

        wait_for_pending(&runtime, &event_id).await;

        let resolved = runtime.resolve(&event_id, UserAction::Approve, None);
        assert_eq!(
            resolved.as_ref().map(|decision| decision.action),
            Some(UserAction::Approve)
        );

        let decision = waiter
            .await
            .expect("waiter task")
            .expect("blocking decision");
        assert_eq!(decision.event_id, event_id);
        assert_eq!(decision.action, UserAction::Approve);
        assert!(runtime.state().store.pending.is_empty());
    }

    #[tokio::test]
    async fn blocking_timeout_returns_none() {
        let runtime = runtime_with(AppSettings::default());
        let event = AgentEvent::new(
            "timeout-session",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        );

        let decision = runtime
            .handle_blocking(event, Duration::from_millis(20))
            .await;
        assert!(decision.is_none());
        assert!(runtime.state().store.pending.is_empty());
    }

    #[tokio::test]
    async fn ending_parent_releases_child_waiters_without_a_decision() {
        let runtime = runtime_with(AppSettings::default());
        runtime.handle_event(AgentEvent::new(
            "parent",
            AgentKind::Claude,
            AgentEventType::SessionStart,
        ));
        let mut request = AgentEvent::new(
            "child",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        );
        request.parent_session_id = Some("parent".into());
        let event_id = request.id.clone();
        let waiter_runtime = Arc::clone(&runtime);
        let waiter = tokio::spawn(async move {
            waiter_runtime.handle_blocking(request, Duration::from_secs(10)).await
        });
        wait_for_pending(&runtime, &event_id).await;
        runtime.handle_event(AgentEvent::new(
            "parent",
            AgentKind::Claude,
            AgentEventType::SessionEnd,
        ));
        assert!(tokio::time::timeout(Duration::from_secs(1), waiter)
            .await.unwrap().unwrap().is_none());
        assert!(runtime.state().store.pending.is_empty());
    }

    #[tokio::test]
    async fn cleanup_removes_expired_sessions() {
        let runtime = runtime_with(AppSettings {
            session_cleanup_minutes: 1,
            ..AppSettings::default()
        });

        let mut event = AgentEvent::new(
            "expired-session",
            AgentKind::Claude,
            AgentEventType::SessionStart,
        );
        event.timestamp = Utc::now() - chrono::Duration::minutes(10);
        runtime.handle_event(event);
        assert_eq!(runtime.state().store.sessions.len(), 1);

        let removed = runtime.run_cleanup();
        assert_eq!(removed, 1);
        assert!(runtime.state().store.sessions.is_empty());
    }

    #[test]
    fn cleanup_settles_only_stale_unmonitored_local_claude_turns() {
        let runtime = runtime_with(AppSettings::default());
        let now = Utc::now();
        let stale = now - TimeDelta::minutes(12);
        for (id, timestamp, pid) in [
            ("missing-stop", stale, None),
            ("still-thinking", now - TimeDelta::minutes(4), None),
            ("monitored", stale, Some(std::process::id())),
            ("ssh:remote", stale, None),
            ("cowork:owned", stale, None),
        ] {
            let mut prompt = AgentEvent::new(id, AgentKind::Claude, AgentEventType::UserPrompt);
            prompt.timestamp = timestamp;
            prompt.pid = pid;
            runtime.handle_event(prompt);
            let mut reply = AgentEvent::new(id, AgentKind::Claude, AgentEventType::Message);
            reply.timestamp = timestamp + TimeDelta::seconds(1);
            reply.text = Some("Reply already received".to_owned());
            runtime.handle_event(reply);
        }
        assert!(runtime.state().store.sessions.iter().all(|session| session.status == SessionStatus::Running));

        assert_eq!(runtime.run_cleanup(), 0);
        let state = runtime.state();
        for session in state.store.sessions {
            if session.session_id == "missing-stop" {
                assert_eq!(session.status, SessionStatus::Idle);
                assert_eq!(session.last_message.as_deref(), Some("Reply already received"));
                assert_eq!(session.last_activity, stale + TimeDelta::seconds(1));
            } else {
                assert_eq!(session.status, SessionStatus::Running, "{}", session.session_id);
            }
        }
        assert!(state.store.pending.is_empty());
        assert!(!state.glance_completion_active);
    }

    #[test]
    fn old_claude_transcript_recovery_stays_idle() {
        let runtime = runtime_with(AppSettings::default());
        let stale = Utc::now() - TimeDelta::minutes(12);
        let mut prompt = AgentEvent::new("reattached", AgentKind::Claude, AgentEventType::UserPrompt);
        prompt.timestamp = stale;
        assert!(runtime.handle_transcript_event(prompt).applied);
        assert_eq!(runtime.state().store.sessions[0].status, SessionStatus::Idle);

        let mut reply = AgentEvent::new("reattached", AgentKind::Claude, AgentEventType::Message);
        reply.timestamp = stale + TimeDelta::seconds(1);
        reply.text = Some("Recovered reply".to_owned());
        assert!(runtime.handle_transcript_event(reply).applied);
        let snapshot = &runtime.state().store.sessions[0];
        assert_eq!(snapshot.status, SessionStatus::Idle);
        assert_eq!(snapshot.last_message.as_deref(), Some("Recovered reply"));
    }

    #[test]
    fn legacy_push_channel_honors_only_when_away() {
        let settings = AppSettings {
            push_enabled: true,
            push_channel: "ntfy".into(),
            push_target: "unused-offline-fixture".into(),
            push_only_when_away: true,
            ..AppSettings::default()
        };
        let runtime = runtime_with(settings.clone());
        let approval = AgentEvent::new("pending", AgentKind::Claude, AgentEventType::PermissionRequest);
        assert!(runtime.handle_event(approval.clone()).applied);

        assert!(runtime.push_notifiers(&settings).is_empty());
        assert!(lock(&runtime.held_pushes).contains_key(&approval.id));
        // Check the away gate without delivering the held request.
        runtime.away.store(true, Ordering::Relaxed);
        assert_eq!(runtime.push_notifiers(&settings).len(), 1);
    }

    #[tokio::test]
    async fn dismiss_completed_skips_waiting_sessions() {
        let runtime = runtime_with(AppSettings::default());

        let mut done = AgentEvent::new(
            "done-session",
            AgentKind::Claude,
            AgentEventType::TurnEnd,
        );
        done.timestamp = Utc::now() - chrono::Duration::seconds(1);
        runtime.handle_event(done);

        let waiting = AgentEvent::new(
            "waiting-session",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        );
        runtime.handle_event(waiting);

        let removed = runtime.dismiss_completed();
        assert_eq!(removed, 1);
        let state = runtime.state();
        assert!(state
            .store
            .sessions
            .iter()
            .any(|session| session.session_id == "waiting-session"));
    }
}
