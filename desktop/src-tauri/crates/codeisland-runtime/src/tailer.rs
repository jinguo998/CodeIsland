//! Polling tailers for agent JSONL transcripts.
//!
//! Codex writes rollout transcripts under `~/.codex/sessions/<yyyy>/<mm>/<dd>/*.jsonl`
//! and Claude Code writes session transcripts under `~/.claude/projects/**/*.jsonl`.
//! `CODEISLAND_CODEX_DIR` / `CODEISLAND_CLAUDE_DIR` override those roots; Cursor,
//! Qoder and CodeBuddy flat transcripts are tailed from `~/.cursor`, `~/.qoder`
//! and `~/.codebuddy` when present (each also has a `CODEISLAND_*_DIR` override).
//! Windows also tails Codex and Claude transcripts in running WSL distributions.
//!
//! The tailers poll every 500 ms and track a byte offset per file. Offsets are
//! persisted so a restart resumes at the tail. On first sight of a *live*
//! transcript (modified in the last 30 minutes) the last 64 KB are scanned and
//! only state-bearing events are replayed, so the restored card recovers its
//! last prompt, reply, model and turn status without replaying history and
//! without a bare `SessionStart` that would clear restored approvals/tasks.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use codeisland_domain::{
    AgentEvent, AgentEventType, AgentKind, CODEX_DESKTOP_SESSION_PREFIX, codex_desktop_session_id,
};
use codeisland_domain::session_metadata::{claude_transcript_metadata, codex_transcript_checklist};
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::Runtime;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_SCAN_DEPTH: usize = 8;
/// A pathological file without newlines must not grow memory without bound.
const MAX_PARTIAL_BYTES: usize = 4 * 1024 * 1024;
/// Offsets survive restarts so a launch resumes at the tail instead of
/// replaying every old transcript.
const OFFSETS_FILE: &str = "tailer-offsets.json";
#[cfg(windows)]
const WSL_OFFSETS_FILE: &str = "tailer-wsl-offsets.json";
#[cfg(windows)]
const WSL_POLL_INTERVAL: Duration = Duration::from_secs(2);
#[cfg(windows)]
const WSL_DISCOVERY_INTERVAL: Duration = Duration::from_secs(15);
const OFFSETS_SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// A transcript written this recently is a live session: recover its card
/// from the tail of the file, but never replay its history.
const LIVE_FILE_WINDOW: Duration = Duration::from_secs(30 * 60);
const CODEX_DESKTOP_RECOVERY_WINDOW: Duration = Duration::from_secs(5 * 60);
/// How much of a live transcript's tail is scanned on first sight.
const TAIL_SCAN_BYTES: u64 = 64 * 1024;

/// Which agent produced a transcript, and therefore how to parse its lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptSource {
    Codex,
    Claude,
    Cursor,
    Qoder,
    CodeBuddy,
}

impl TranscriptSource {
    fn agent(self) -> AgentKind {
        match self {
            Self::Codex => AgentKind::Codex,
            Self::Claude => AgentKind::Claude,
            Self::Cursor => AgentKind::Cursor,
            Self::Qoder => AgentKind::Qoder,
            Self::CodeBuddy => AgentKind::CodeBuddy,
        }
    }

    fn raw_prefix(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Qoder => "qoder",
            Self::CodeBuddy => "codebuddy",
        }
    }

    fn parse(self, line: &str, meta: &mut TranscriptMeta) -> Option<AgentEvent> {
        match self {
            Self::Codex => parse_codex_line(line, meta),
            Self::Claude => parse_claude_line(self, line, meta),
            Self::Cursor | Self::Qoder | Self::CodeBuddy => parse_flat_line(self, line, meta),
        }
    }
}

pub(crate) fn spawn(
    runtime: Arc<Runtime>,
    running: Arc<AtomicBool>,
    handle: tokio::runtime::Handle,
) -> JoinHandle<()> {
    handle.spawn(async move {
        #[cfg(windows)]
        tokio::join!(tail_configured(&runtime, &running), tail_wsl(&runtime, &running));
        #[cfg(not(windows))]
        tail_configured(&runtime, &running).await;
    })
}

struct TranscriptTailer {
    cursors: HashMap<PathBuf, FileCursor>,
    saved: HashMap<String, u64>,
    checkpoint: PathBuf,
    last_save: Instant,
    check_file_exists: bool,
}

impl TranscriptTailer {
    fn new(offsets_file: &str, check_file_exists: bool) -> Self {
        let checkpoint = crate::persistence::state_dir().join(offsets_file);
        Self {
            cursors: HashMap::new(),
            saved: load_offsets(&checkpoint, check_file_exists),
            checkpoint,
            last_save: Instant::now(),
            check_file_exists,
        }
    }

    fn poll(
        &mut self,
        runtime: &Arc<Runtime>,
        running: &AtomicBool,
        roots: impl IntoIterator<Item = (TranscriptSource, PathBuf)>,
        include: impl Fn(&Path) -> bool,
    ) {
        // A file is tailed once even if two roots happen to overlap.
        let mut seen = HashSet::new();
        for (source, root) in roots {
            if !running.load(Ordering::SeqCst) {
                break;
            }
            let mut files = Vec::new();
            collect_jsonl(&root, &mut files, 0);
            files.sort();
            for file in files {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                if include(&file) && seen.insert(file.clone()) {
                    tail_file(runtime, source, &file, &mut self.cursors, &self.saved);
                }
            }
        }
        if self.last_save.elapsed() >= OFFSETS_SAVE_INTERVAL {
            self.save();
            self.last_save = Instant::now();
        }
    }

    fn save(&self) {
        save_offsets(&self.checkpoint, &self.cursors, self.check_file_exists);
    }
}

async fn tail_configured(runtime: &Arc<Runtime>, running: &AtomicBool) {
    let mut tailer = TranscriptTailer::new(OFFSETS_FILE, true);
    while running.load(Ordering::SeqCst) {
        tailer.poll(runtime, running, transcript_roots(&runtime.settings()), |_| true);
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    tailer.save();
}

#[cfg(windows)]
async fn tail_wsl(runtime: &Arc<Runtime>, running: &Arc<AtomicBool>) {
    use crate::wsl::{WslTranscriptSource, discover_distributions};
    use crate::wsl_process::{DistributionProcesses, Observation};

    let mut tailer: Option<TranscriptTailer> = None;
    let mut discovery = None;
    let mut refresh_at = Instant::now();
    while running.load(Ordering::SeqCst) {
        if Instant::now() >= refresh_at {
            discovery = discover_distributions().await;
            refresh_at = Instant::now() + WSL_DISCOVERY_INTERVAL;
        }

        if let Some(discovery) = &discovery
            && running.load(Ordering::SeqCst)
        {
            let runtime = Arc::clone(runtime);
            let running = Arc::clone(running);
            let distributions = discovery.distributions.clone();
            let running_distros = discovery.running.clone();
            let previous = tailer.take();
            // UNC access can block if a distribution exits during the scan.
            // Keep it away from the configured roots and the async executor.
            let next = tokio::task::spawn_blocking(move || {
                // Old checkpoints may name a distribution that has since
                // stopped. Only the active-root scan should touch UNC files.
                let mut tailer = previous.unwrap_or_else(|| TranscriptTailer::new(WSL_OFFSETS_FILE, false));
                let configured: Vec<_> = transcript_roots(&runtime.settings()).into_iter()
                    .filter_map(|(_, path)| windows_path_key(&path))
                    .collect();
                let include = |path: &Path| !covered_windows_path(path, &configured);
                let processes: Vec<_> = distributions.iter()
                    .filter_map(|distro| DistributionProcesses::scan(distro).ok())
                    .collect();
                let mut distinct = HashSet::new();
                let roots = distributions.iter().flat_map(|distro| distro.transcript_roots())
                    .chain(processes.iter().flat_map(DistributionProcesses::transcript_roots))
                    .filter(|(_, root)| include(root) && distinct.insert(root.clone()))
                    .map(|(source, path)| {
                        let source = match source {
                            WslTranscriptSource::Codex => TranscriptSource::Codex,
                            WslTranscriptSource::Claude => TranscriptSource::Claude,
                        };
                        (source, path)
                    });
                tailer.poll(&runtime, &running, roots, include);
                let observations = runtime.wsl_sessions().into_iter().filter_map(|session| {
                    let stopped = session.wsl_process.as_ref().is_some_and(|process| {
                        !running_distros.iter().any(|name| name.eq_ignore_ascii_case(&process.distro))
                    });
                    let observation = if stopped {
                        Observation { process: None, status: None, settle: true }
                    } else {
                        processes.iter().find_map(|processes| processes.observe(&session))?
                    };
                    Some((session, observation))
                }).collect::<Vec<_>>();
                runtime.merge_wsl_processes(observations);
                tailer
            }).await;
            let Ok(next) = next else { break };
            tailer = Some(next);
        }
        tokio::time::sleep(WSL_POLL_INTERVAL).await;
    }
    if let Some(tailer) = tailer {
        let _ = tokio::task::spawn_blocking(move || tailer.save()).await;
    }
}

#[cfg(windows)]
fn windows_path_key(path: &Path) -> Option<PathBuf> {
    use std::path::{Component, Prefix};

    if !path.is_absolute() {
        return None;
    }
    let mut components = path.components();
    let Component::Prefix(prefix) = components.next()? else { return None };
    let (mut key, case_sensitive) = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            (PathBuf::from(format!("{}:\\", drive.to_ascii_lowercase() as char)), false)
        }
        Prefix::UNC(server, distro) | Prefix::VerbatimUNC(server, distro)
            if server.to_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("wsl.localhost") || host.eq_ignore_ascii_case("wsl$")
            }) =>
        {
            // Unify Windows' two WSL share aliases without folding the case of
            // paths inside the Linux filesystem.
            (PathBuf::from(distro.to_str()?.to_ascii_lowercase()), true)
        }
        _ => return None,
    };
    let root_depth = key.components().count();
    for component in components {
        match component {
            Component::Normal(part) if case_sensitive => key.push(part),
            Component::Normal(part) => key.push(part.to_str()?.to_ascii_lowercase()),
            Component::ParentDir if key.components().count() > root_depth => { key.pop(); }
            _ => {}
        }
    }
    Some(key)
}

#[cfg(windows)]
fn covered_windows_path(path: &Path, configured: &[PathBuf]) -> bool {
    windows_path_key(path).is_some_and(|key| configured.iter().any(|root| key.starts_with(root)))
}

fn transcript_roots(settings: &codeisland_domain::AppSettings) -> Vec<(TranscriptSource, PathBuf)> {
    let home = dirs::home_dir();
    let paths = codeisland_agents::paths::Paths::current().with_settings(settings);
    let mut roots = Vec::new();
    let mut push = |source: TranscriptSource, env_key: &str, default: Option<PathBuf>| {
        let dir = std::env::var_os(env_key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or(default);
        if let Some(dir) = dir {
            roots.push((source, dir));
        }
    };
    push(
        TranscriptSource::Codex,
        "CODEISLAND_CODEX_DIR",
        Some(paths.codex_config_dir().join("sessions")),
    );
    push(
        TranscriptSource::Cursor,
        "CODEISLAND_CURSOR_DIR",
        home.as_ref().map(|home| home.join(".cursor")),
    );
    push(
        TranscriptSource::Qoder,
        "CODEISLAND_QODER_DIR",
        home.as_ref().map(|home| home.join(".qoder")),
    );
    push(
        TranscriptSource::CodeBuddy,
        "CODEISLAND_CODEBUDDY_DIR",
        home.as_ref().map(|home| home.join(".codebuddy")),
    );
    if let Some(path) = std::env::var_os("CODEISLAND_CLAUDE_DIR").filter(|path| !path.is_empty()) {
        roots.push((TranscriptSource::Claude, PathBuf::from(path)));
    } else {
        roots.extend(paths.claude_discovery_dirs().into_iter().map(|root| {
            (TranscriptSource::Claude, root.join("projects"))
        }));
    }
    roots
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => collect_jsonl(&path, out, depth + 1),
            Ok(file_type) if file_type.is_file() => {
                if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct FileCursor {
    offset: u64,
    /// Bytes after the last newline, kept raw so a multi-byte UTF-8 character
    /// split across two polls is not corrupted by lossy decoding.
    partial: Vec<u8>,
    /// Absolute file offset where `partial` begins.
    partial_start: u64,
    meta: TranscriptMeta,
    initialized: bool,
}

fn load_offsets(path: &Path, check_file_exists: bool) -> HashMap<String, u64> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(map) = serde_json::from_str::<HashMap<String, u64>>(&text) else {
        return HashMap::new();
    };
    // Prune entries for transcripts that were deleted while the app was away.
    map.into_iter()
        .filter(|(path, _)| !check_file_exists || Path::new(path).is_file())
        .collect()
}

fn save_offsets(path: &Path, cursors: &HashMap<PathBuf, FileCursor>, check_file_exists: bool) {
    let map: HashMap<String, u64> = cursors
        .iter()
        .filter(|(path, cursor)| cursor.initialized && (!check_file_exists || path.is_file()))
        .map(|(path, cursor)| {
            // Resume from the start of an unfinished line so its completion is
            // parsed (with the same stable id) after a restart.
            let offset = if cursor.partial.is_empty() {
                cursor.offset
            } else {
                cursor.partial_start
            };
            (path.to_string_lossy().into_owned(), offset)
        })
        .collect();
    let Ok(payload) = serde_json::to_vec(&map) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, payload).is_ok() {
        let _ = std::fs::rename(&temporary, path);
    }
}

/// Session identity learned from `session_meta` (Codex) or the record headers
/// (Claude and the flat formats).
#[derive(Debug, Clone, Default)]
pub(crate) struct TranscriptMeta {
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
    codex_worker_title: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

fn is_live(metadata: &std::fs::Metadata) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < LIVE_FILE_WINDOW)
}

/// Reads the first line of a transcript to learn its identity before the tail
/// scan runs (the `session_meta` row may sit outside the 64 KB window).
fn read_head_line(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file).take(MAX_PARTIAL_BYTES as u64);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        None
    } else {
        Some(line.to_owned())
    }
}

fn tail_file(
    runtime: &Arc<Runtime>,
    source: TranscriptSource,
    path: &Path,
    cursors: &mut HashMap<PathBuf, FileCursor>,
    saved: &HashMap<String, u64>,
) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    let length = metadata.len();
    let key = path.to_string_lossy().into_owned();

    let cursor = cursors.entry(path.to_path_buf()).or_default();
    let mut replaying = false;
    if !cursor.initialized {
        let head = read_head_line(path);
        if let Some(line) = head.as_deref() {
            let _ = parse_line(source, line, &mut cursor.meta);
        }
        // Claude names its files after the session; flat formats do too. Codex
        // rollout filenames are not session ids, so never use them there.
        if cursor.meta.session_id.is_none() && source != TranscriptSource::Codex {
            cursor.meta.session_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(ToOwned::to_owned);
        }
        if !cursor.meta.session_id.as_deref().is_none_or(|id| runtime.can_tail_codex_desktop(id)) {
            return;
        }
        cursor.initialized = true;
        let desktop = cursor.meta.session_id.as_deref()
            .is_some_and(|id| id.starts_with(CODEX_DESKTOP_SESSION_PREFIX));
        let live = is_live(&metadata);
        // Older versions saved zero while the desktop gate was closed. Treat
        // it as no checkpoint so a stale desktop rollout starts at EOF.
        match saved
            .get(&key)
            .copied()
            .filter(|offset| !desktop || (!live && *offset != 0))
        {
            Some(offset) => {
                // Lines written while the tailer was detached rebuild history
                // without becoming the current turn's live preview.
                replaying = true;
                if offset <= length {
                    cursor.offset = offset;
                    if live && offset == length && matches!(source, TranscriptSource::Claude | TranscriptSource::Codex) {
                        let mut metadata = cursor.meta.clone();
                        let (events, _, _) = scan_tail(source, path, length, &mut metadata);
                        for event in events.iter().filter(|event| event.transcript_metadata.is_some()) {
                            runtime.backfill_transcript_metadata(event);
                        }
                    }
                } else {
                    // The file shrank while the app was closed.
                    cursor.offset = 0;
                    cursor.partial.clear();
                    cursor.partial_start = 0;
                }
            }
            None if live => {
                let (partial_start, partial) = recover_from_tail(
                    runtime,
                    source,
                    path,
                    length,
                    &mut cursor.meta,
                    head.as_deref(),
                );
                // A long active turn can leave only tool or reasoning rows
                // in the preview window. Recover its phase independently,
                // from the newest lifecycle marker in the bounded reverse scan.
                if desktop
                    && let Some(id) = cursor.meta.session_id.as_deref()
                    && let Ok(modified) = metadata.modified()
                    && modified.elapsed().is_ok_and(|age| age <= CODEX_DESKTOP_RECOVERY_WINDOW)
                    && let Some(turn) = codex_turn(path)
                {
                    runtime.restore_codex_rollout_turn(id, &key, DateTime::from(modified), turn);
                }
                // The scan consumed everything up to EOF; only the unfinished
                // trailing line stays buffered for the live tail.
                cursor.offset = length;
                cursor.partial = partial;
                cursor.partial_start = partial_start;
            }
            None => cursor.offset = length,
        }
        // The saved cursor may already be at EOF. Reconcile the header
        // without replaying the child's turn.
        reconcile_codex_worker(runtime, &cursor.meta, &key);
    }

    if !cursor.meta.session_id.as_deref().is_none_or(|id| runtime.can_tail_codex_desktop(id)) {
        return;
    }
    if length < cursor.offset {
        // Truncated or replaced while running: re-read from the start rather
        // than jumping to EOF, so the replacement's content is not lost.
        cursor.offset = 0;
        cursor.partial.clear();
        cursor.partial_start = 0;
    }
    if length == cursor.offset {
        return;
    }

    let read_start = cursor.offset;
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    if file.seek(SeekFrom::Start(read_start)).is_err() {
        return;
    }
    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).is_err() {
        return;
    }
    cursor.offset = read_start + buffer.len() as u64;

    let mut combined = std::mem::take(&mut cursor.partial);
    let base = if combined.is_empty() {
        read_start
    } else {
        cursor.partial_start
    };
    combined.extend_from_slice(&buffer);

    // Only complete lines are parsed; the trailing fragment waits for the next poll.
    let complete_end = combined
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let trailing = combined.split_off(complete_end);
    cursor.partial_start = base + complete_end as u64;
    cursor.partial = trailing;
    if cursor.partial.len() > MAX_PARTIAL_BYTES {
        cursor.partial.clear();
        cursor.partial_start = cursor.offset;
    }

    let mut meta = cursor.meta.clone();
    let mut events = Vec::new();
    parse_complete_lines(source, path, &combined, base, &mut meta, &mut events);
    cursor.meta = meta;

    for mut event in events {
        if replaying {
            event.codex_transcript_output = None;
        }
        runtime.handle_transcript_event(event);
    }
    if replaying {
        reconcile_codex_worker(runtime, &cursor.meta, &key);
    }
}

fn reconcile_codex_worker(runtime: &Runtime, meta: &TranscriptMeta, path: &str) {
    if let (Some(child), Some(parent)) = (meta.session_id.as_deref(), meta.parent_session_id.as_deref()) {
        runtime.reconcile_codex_rollout_parent(child, parent, path, meta.codex_worker_title.as_deref());
    }
}

/// Scans the last [`TAIL_SCAN_BYTES`] of a live transcript, aligned to line
/// boundaries, and emits only the state-bearing events it contains.
///
/// Returns the absolute offset and bytes of the unterminated trailing line so
/// the live tail can complete it on a later poll.
fn recover_from_tail(
    runtime: &Arc<Runtime>,
    source: TranscriptSource,
    path: &Path,
    length: u64,
    meta: &mut TranscriptMeta,
    head: Option<&str>,
) -> (u64, Vec<u8>) {
    let (mut events, partial_start, partial) = scan_tail(source, path, length, meta);
    events.retain(|event| is_state_bearing(event.event_type) || event.transcript_metadata.is_some());

    if events.is_empty() {
        // Nothing to restore, but the card still needs a session. This is the
        // only case where a bare SessionStart is synthesized.
        if let Some(mut event) = base_event(
            source,
            meta,
            AgentEventType::SessionStart,
            record_timestamp(head),
        ) {
            event.id = stable_event_id(path, u64::MAX);
            event.transcript_path = Some(path.to_string_lossy().into_owned());
            events.push(event);
        }
    }

    for mut event in events {
        event.codex_transcript_output = None;
        if event.event_type == AgentEventType::ToolStart && event.transcript_metadata.is_some() {
            event.event_type = AgentEventType::Heartbeat;
            event.tool = None;
            event.text = None;
        }
        runtime.handle_transcript_event(event);
    }
    (partial_start, partial)
}

fn scan_tail(
    source: TranscriptSource,
    path: &Path,
    length: u64,
    meta: &mut TranscriptMeta,
) -> (Vec<AgentEvent>, u64, Vec<u8>) {
    let empty = (Vec::new(), length, Vec::new());
    if length == 0 {
        return empty;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return empty;
    };
    // One byte before the window tells whether it starts on a line boundary.
    let probe_start = length.saturating_sub(TAIL_SCAN_BYTES + 1);
    if file.seek(SeekFrom::Start(probe_start)).is_err() {
        return empty;
    }
    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).is_err() {
        return empty;
    }

    let mut base = probe_start;
    let mut window: &[u8] = &buffer;
    if probe_start > 0 {
        match window.iter().position(|&byte| byte == b'\n') {
            Some(index) => {
                base = probe_start + index as u64 + 1;
                window = &window[index + 1..];
            }
            // The window sits inside one huge row: nothing complete to scan.
            None => return empty,
        }
    }

    let complete_end = window
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut events = Vec::new();
    parse_complete_lines(source, path, &window[..complete_end], base, meta, &mut events);
    let partial = window[complete_end..].to_vec();
    (events, base + complete_end as u64, partial)
}

fn is_state_bearing(event_type: AgentEventType) -> bool {
    matches!(
        event_type,
        AgentEventType::UserPrompt
            | AgentEventType::Message
            | AgentEventType::TurnEnd
            | AgentEventType::TurnFailed
            | AgentEventType::Interrupt
            | AgentEventType::ToolEnd
            | AgentEventType::Heartbeat
    )
}

/// Parses complete lines from `bytes`, assigning each event an id derived from
/// the transcript path and the absolute offset of its line.
fn parse_complete_lines(
    source: TranscriptSource,
    path: &Path,
    bytes: &[u8],
    base_offset: u64,
    meta: &mut TranscriptMeta,
    events: &mut Vec<AgentEvent>,
) {
    let transcript_path = path.to_string_lossy().into_owned();
    let mut line_start = 0usize;
    for line in bytes.split_inclusive(|&byte| byte == b'\n') {
        let line_end = line_start + line.len();
        let raw = line.strip_suffix(&[b'\n']).unwrap_or(line);
        let raw = raw.strip_suffix(&[b'\r']).unwrap_or(raw);
        if raw.iter().any(|byte| !byte.is_ascii_whitespace()) {
            let text = String::from_utf8_lossy(raw);
            if let Some(mut event) = parse_line(source, &text, meta) {
                event.id = stable_event_id(path, base_offset + line_start as u64);
                event.transcript_path = Some(transcript_path.clone());
                events.push(event);
            }
        }
        line_start = line_end;
    }
}

/// Stable id for a transcript event: the same line always maps to the same id,
/// so a crash replay is deduplicated by the session store.
fn stable_event_id(path: &Path, offset: u64) -> String {
    format!("transcript:{}:{offset}", path.to_string_lossy())
}

fn parse_line(
    source: TranscriptSource,
    line: &str,
    meta: &mut TranscriptMeta,
) -> Option<AgentEvent> {
    source.parse(line, meta)
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

pub(crate) fn is_codex_desktop_transcript(path: &str) -> bool {
    let path = Path::new(path);
    if !path.is_file() {
        return false;
    }
    let mut meta = TranscriptMeta::default();
    if let Some(line) = read_head_line(path) {
        parse_codex_line(&line, &mut meta);
    }
    meta.session_id.is_some_and(|id| id.starts_with(CODEX_DESKTOP_SESSION_PREFIX))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CodexOrigin {
    #[default]
    Unknown,
    Desktop,
    Cli,
}

pub(crate) fn codex_origin(originator: Option<&str>, source: Option<&str>, subagent: bool) -> CodexOrigin {
    codeisland_swift::call(c"codex.origin", serde_json::json!({
        "originator": originator, "source": source, "subagent": subagent,
    }))
}

fn codex_rollout_is_desktop(payload: &Value) -> bool {
    codex_origin(payload["originator"].as_str(), payload["source"].as_str(), payload["source"].is_object())
        == CodexOrigin::Desktop
}

fn codex_rollout_parent(payload: &Value) -> Option<String> {
    let subagent = payload.pointer("/source/subagent")?;
    match subagent {
        Value::Object(fields) if !fields.is_empty() => {
            string_field(&subagent["thread_spawn"], &["parent_thread_id"])
                .or_else(|| string_field(payload, &["parent_thread_id"]))
        }
        Value::String(name) if !name.is_empty() => string_field(payload, &["parent_thread_id"]),
        _ => None,
    }
}

fn codex_worker_title(payload: &Value) -> Option<String> {
    let subagent = payload.pointer("/source/subagent")?;
    let spawn = &subagent["thread_spawn"];
    string_field(spawn, &["agent_nickname"])
        .or_else(|| string_field(payload, &["agent_nickname"]))
        .or_else(|| string_field(spawn, &["agent_role"]))
        .or_else(|| string_field(subagent, &["other"]))
        .or_else(|| subagent.as_str().and_then(non_empty))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RecoveredTurn {
    Running,
    Settled,
}

#[cfg(any(windows, test))]
#[derive(Default)]
pub(crate) struct RecoverySummary {
    pub origin: CodexOrigin,
    pub raw_id: Option<String>,
    pub parent_id: Option<String>,
    pub codex_worker_title: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub last_prompt: Option<String>,
    pub last_message: Option<String>,
    pub turn: Option<RecoveredTurn>,
}

#[cfg(any(windows, test))]
pub(crate) fn read_recovery_summary(
    path: &Path, agent: AgentKind, expected_id: &str,
) -> Option<RecoverySummary> {
    const PREVIEW_BYTES: u64 = 256 * 1024;
    let source = match agent {
        AgentKind::Codex => TranscriptSource::Codex,
        AgentKind::Claude => TranscriptSource::Claude,
        _ => return None,
    };
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size == 0 { return None; }
    let mut meta = TranscriptMeta { session_id: Some(expected_id.to_owned()), ..TranscriptMeta::default() };
    let mut summary = RecoverySummary::default();
    if let Some(head) = read_head_line(path) {
        let head = head.trim_start_matches('\u{feff}');
        if source == TranscriptSource::Codex
            && let Ok(row) = serde_json::from_str::<Value>(head)
            && row["type"] == "session_meta"
        {
            let payload = &row["payload"];
            summary.raw_id = string_field(payload, &["id", "session_id"]);
            summary.origin = codex_origin(
                payload["originator"].as_str(), payload["source"].as_str(), payload["source"].is_object(),
            );
            summary.parent_id = codex_rollout_parent(payload);
        }
        let _ = parse_line(source, head, &mut meta);
    }
    let start = size.saturating_sub(PREVIEW_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.by_ref().take(size - start).read_to_end(&mut bytes).ok()?;
    let mut lines = bytes.split_inclusive(|byte| *byte == b'\n');
    if start != 0 { lines.next(); }
    for bytes in lines.filter(|line| line.last() == Some(&b'\n')) {
        let Ok(line) = std::str::from_utf8(bytes) else { continue };
        let line = line.trim_start_matches('\u{feff}');
        if source == TranscriptSource::Claude && let Some(turn) = claude_turn(line) {
            summary.turn = Some(turn);
        }
        let Some(event) = parse_line(source, line, &mut meta) else { continue };
        match event.event_type {
            AgentEventType::UserPrompt if event.text.is_some() => summary.last_prompt = event.text,
            AgentEventType::Message if event.text.is_some() => summary.last_message = event.text,
            _ => {}
        }
        if event.model.is_some() { meta.model = event.model; }
    }
    summary.cwd = meta.cwd;
    summary.codex_worker_title = meta.codex_worker_title;
    summary.model = meta.model;
    summary.reasoning = meta.effort;
    if source == TranscriptSource::Claude {
        summary.raw_id = meta.session_id;
    }
    if source == TranscriptSource::Codex {
        summary.turn = codex_turn(path);
    }
    Some(summary)
}

#[cfg(any(windows, test))]
fn claude_turn(line: &str) -> Option<RecoveredTurn> {
    let row: Value = serde_json::from_str(line).ok()?;
    match row["type"].as_str()? {
        "result" => Some(RecoveredTurn::Settled),
        "user" if !row["isMeta"].as_bool().unwrap_or(false)
            && chat_content(&row).and_then(extract_content_text).is_some() => Some(RecoveredTurn::Running),
        _ => None,
    }
}

fn codex_turn(path: &Path) -> Option<RecoveredTurn> {
    codeisland_swift::call(c"codex.rolloutTurn", path)
}

fn parse_codex_line(line: &str, meta: &mut TranscriptMeta) -> Option<AgentEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    let record_type = value.get("type").and_then(Value::as_str)?;
    let payload = value.get("payload").cloned().unwrap_or(Value::Null);
    let timestamp = timestamp_of(&value);

    match record_type {
        "session_meta" => {
            if let Some(id) = payload
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| payload.get("session_id").and_then(Value::as_str))
            {
                let desktop = codex_rollout_is_desktop(&payload);
                meta.session_id = Some(if desktop {
                    codex_desktop_session_id(id)
                } else {
                    id.to_owned()
                });
                meta.parent_session_id = codex_rollout_parent(&payload)
                    .map(|parent| if desktop { codex_desktop_session_id(&parent) } else { parent })
                    .filter(|parent| Some(parent) != meta.session_id.as_ref());
                if meta.parent_session_id.is_some() {
                    meta.codex_worker_title = codex_worker_title(&payload);
                }
            }
            if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                meta.cwd = Some(cwd.to_owned());
            }
            None
        }
        // Codex writes the turn's model and effort once per turn. Later records
        // inherit them through `base_event`.
        "turn_context" => {
            if let Some(model) = string_field(&payload, &["model"]) {
                meta.model = Some(model);
            }
            if let Some(effort) = string_field(&payload, &["effort", "reasoning_effort"]) {
                meta.effort = Some(effort);
            }
            if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                meta.cwd = Some(cwd.to_owned());
            }
            None
        }
        "event_msg" => parse_event_msg(&payload, timestamp, meta),
        "response_item" => parse_response_item(&payload, timestamp, meta),
        _ => None,
    }
}

fn parse_event_msg(
    payload: &Value,
    timestamp: DateTime<Utc>,
    meta: &TranscriptMeta,
) -> Option<AgentEvent> {
    let message_type = payload.get("type").and_then(Value::as_str)?;
    let (event_type, tool, text) = match message_type {
        "task_started" => (AgentEventType::UserPrompt, None, None),
        "user_message" => (
            AgentEventType::UserPrompt,
            None,
            string_field(payload, &["message", "text"]),
        ),
        "task_complete" => (AgentEventType::TurnEnd, None, None),
        "turn_failed" => (
            AgentEventType::TurnFailed,
            None,
            string_field(payload, &["message", "error", "reason"]),
        ),
        "agent_message" => (
            AgentEventType::Message,
            None,
            string_field(payload, &["message", "text"]),
        ),
        "agent_reasoning" => (
            AgentEventType::Heartbeat,
            Some("background".to_owned()),
            None,
        ),
        "mcp_tool_call_end" => (
            AgentEventType::ToolEnd,
            Some(mcp_tool_label(payload)),
            string_field(payload, &["result", "output"]),
        ),
        "turn_aborted" => (
            AgentEventType::Interrupt,
            None,
            string_field(payload, &["reason"]),
        ),
        // Plan records are intentionally not card state yet.
        "plan_update" | "plan_delta" => return None,
        _ => return None,
    };

    let mut event = base_event(TranscriptSource::Codex, meta, event_type, timestamp)?;
    event.tool = tool;
    event.text = text;
    if message_type == "agent_message" {
        event.codex_transcript_output = string_field(payload, &["message"]);
    }
    if let Some(model) = string_field(payload, &["model"]) {
        event.model = Some(model);
    }
    Some(event)
}

fn parse_response_item(
    payload: &Value,
    timestamp: DateTime<Utc>,
    meta: &TranscriptMeta,
) -> Option<AgentEvent> {
    let item_type = payload.get("type").and_then(Value::as_str)?;
    match item_type {
        "custom_tool_call" | "function_call" => {
            let mut event =
                base_event(TranscriptSource::Codex, meta, AgentEventType::ToolStart, timestamp)?;
            event.tool = string_field(payload, &["name", "tool_name"]);
            event.text = string_field(payload, &["input", "arguments"]);
            event.transcript_metadata = codex_transcript_checklist(payload);
            Some(event)
        }
        "function_call_output" => {
            let mut event =
                base_event(TranscriptSource::Codex, meta, AgentEventType::ToolEnd, timestamp)?;
            event.tool = string_field(payload, &["name", "tool_name"]);
            event.text = string_field(payload, &["output"]);
            Some(event)
        }
        // The public assistant reply. `content` is either a string or an array
        // of `{"type":"text"|"output_text","text":…}` blocks.
        "message" => {
            if payload.get("role").and_then(Value::as_str) != Some("assistant") {
                return None;
            }
            let content = payload.get("content")?;
            let text = extract_content_text(content)?;
            let mut event =
                base_event(TranscriptSource::Codex, meta, AgentEventType::Message, timestamp)?;
            event.text = Some(text);
            // Only explicit assistant output blocks enter the compact live
            // preview. Older generic blocks remain readable in history.
            if let Some(blocks) = content.as_array() {
                let public_text = blocks.iter()
                    .filter(|block| block.get("type").and_then(Value::as_str) == Some("output_text"))
                    .filter_map(|block| block.get("text")?.as_str().and_then(non_empty))
                    .collect::<Vec<_>>();
                if !public_text.is_empty() {
                    event.codex_transcript_output = Some(public_text.join("\n"));
                }
            }
            Some(event)
        }
        "reasoning" => {
            // Reasoning stays background motion; it is never the reply text.
            let mut event =
                base_event(TranscriptSource::Codex, meta, AgentEventType::Heartbeat, timestamp)?;
            event.tool = Some("background".to_owned());
            Some(event)
        }
        // `update_plan` and other plan records are intentionally ignored.
        _ => None,
    }
}

fn mcp_tool_label(payload: &Value) -> String {
    let invocation = payload.get("invocation");
    let server = invocation
        .and_then(|value| value.get("server"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("server").and_then(Value::as_str));
    let tool = invocation
        .and_then(|value| value.get("tool"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("tool").and_then(Value::as_str));
    match (server, tool) {
        (Some(server), Some(tool)) => format!("plugin {server}/{tool}"),
        (Some(server), None) => format!("plugin {server}"),
        (None, Some(tool)) => format!("plugin {tool}"),
        (None, None) => "plugin".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Claude Code
// ---------------------------------------------------------------------------

fn parse_claude_line(
    source: TranscriptSource,
    line: &str,
    meta: &mut TranscriptMeta,
) -> Option<AgentEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    update_common_meta(&value, meta);
    if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let record_type = value.get("type").and_then(Value::as_str)?;
    let timestamp = timestamp_of(&value);
    let metadata = if source == TranscriptSource::Claude {
        claude_transcript_metadata(&value)
    } else {
        None
    };

    let mut event = match record_type {
        "user" | "USER_INPUT" => {
            // A tool-result-only user row carries no prompt; `extract_content_text`
            // returns None for it.
            let text = chat_content(&value).and_then(extract_content_text).filter(|text| {
                source != TranscriptSource::Claude
                    || (value["isCompactSummary"] != true
                        && value["isSidechain"] != true
                        && ![
                            "<command-name>",
                            "<local-command-stdout>",
                            "<local-command-stderr>",
                            "<local-command-caveat>",
                        ].iter().any(|tag| text.starts_with(tag)))
            });
            let kind = if text.is_some() { AgentEventType::UserPrompt } else { AgentEventType::Heartbeat };
            if text.is_none() && metadata.is_none() { return None; }
            let mut event = base_event(source, meta, kind, timestamp)?;
            event.text = text;
            Some(event)
        }
        "assistant" | "PLANNER_RESPONSE" => {
            if let Some(model) = chat_model(&value) {
                meta.model = Some(model);
            }
            let text = chat_content(&value).and_then(extract_content_text);
            let kind = if text.is_some() { AgentEventType::Message } else { AgentEventType::Heartbeat };
            if text.is_none() && metadata.is_none() { return None; }
            let mut event = base_event(source, meta, kind, timestamp)?;
            event.text = text;
            Some(event)
        }
        "result" => {
            let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");
            let failed = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || (!subtype.is_empty() && subtype != "success");
            let (event_type, text) = if failed {
                (
                    AgentEventType::TurnFailed,
                    string_field(&value, &["result", "error"]),
                )
            } else {
                (AgentEventType::TurnEnd, None)
            };
            let mut event = base_event(source, meta, event_type, timestamp)?;
            event.text = text;
            Some(event)
        }
        "system" if metadata.is_some() => base_event(source, meta, AgentEventType::Heartbeat, timestamp),
        _ => None,
    }?;
    event.transcript_metadata = metadata;
    Some(event)
}

/// Claude-style content lives in `message.content`, but flat formats sometimes
/// put it at the top level.
fn chat_content(value: &Value) -> Option<&Value> {
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .or_else(|| value.get("content"))
}

fn chat_model(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(|message| message.get("model"))
        .and_then(Value::as_str)
        .or_else(|| value.get("model").and_then(Value::as_str))
        .and_then(non_empty)
}

fn update_common_meta(value: &Value, meta: &mut TranscriptMeta) {
    if let Some(id) = value
        .get("sessionId")
        .and_then(Value::as_str)
        .or_else(|| value.get("session_id").and_then(Value::as_str))
        .and_then(non_empty)
    {
        meta.session_id = Some(id);
    }
    if let Some(cwd) = value.get("cwd").and_then(Value::as_str).and_then(non_empty) {
        meta.cwd = Some(cwd);
    }
    if let Some(model) = value.get("model").and_then(Value::as_str).and_then(non_empty) {
        meta.model = Some(model);
    }
    if let Some(effort) = value
        .get("effort")
        .and_then(Value::as_str)
        .or_else(|| value.get("perTurnEffort").and_then(Value::as_str))
        .and_then(non_empty)
    {
        meta.effort = Some(effort);
    }
}

// ---------------------------------------------------------------------------
// Cursor / Qoder / CodeBuddy flat transcripts
// ---------------------------------------------------------------------------

fn parse_flat_line(
    source: TranscriptSource,
    line: &str,
    meta: &mut TranscriptMeta,
) -> Option<AgentEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value.get("type").and_then(Value::as_str) {
        // Claude Code forks (Qoder, CodeBuddy) share the Claude record shapes.
        Some("user" | "USER_INPUT" | "assistant" | "PLANNER_RESPONSE" | "result" | "system") => {
            parse_claude_line(source, line, meta)
        }
        // CodeBuddy wraps the role in a `message` record.
        Some("message") => parse_role_line(source, &value, meta),
        Some(_) => None,
        // Cursor keys its entries on a top-level `role` instead of `type`:
        // `{"role":"user","message":{"content":[…]}}`.
        None => parse_role_line(source, &value, meta),
    }
}

fn parse_role_line(
    source: TranscriptSource,
    value: &Value,
    meta: &mut TranscriptMeta,
) -> Option<AgentEvent> {
    update_common_meta(value, meta);
    if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let message = value.get("message").unwrap_or(value);
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| message.get("role").and_then(Value::as_str))?;
    let timestamp = timestamp_of(value);

    match role {
        "user" => {
            let text = message
                .get("content")
                .or_else(|| value.get("content"))
                .and_then(extract_content_text)?;
            let mut event = base_event(source, meta, AgentEventType::UserPrompt, timestamp)?;
            event.text = Some(text);
            Some(event)
        }
        "assistant" => {
            if let Some(model) = chat_model(value) {
                meta.model = Some(model);
            }
            let text = message
                .get("content")
                .or_else(|| value.get("content"))
                .and_then(extract_content_text)?;
            let mut event = base_event(source, meta, AgentEventType::Message, timestamp)?;
            event.text = Some(text);
            Some(event)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn base_event(
    source: TranscriptSource,
    meta: &TranscriptMeta,
    event_type: AgentEventType,
    timestamp: DateTime<Utc>,
) -> Option<AgentEvent> {
    let session_id = meta.session_id.clone()?;
    let mut event = AgentEvent::new(session_id, source.agent(), event_type);
    event.parent_session_id = meta.parent_session_id.clone();
    event.timestamp = timestamp;
    event.cwd = meta.cwd.clone();
    event.model = meta.model.clone();
    event.reasoning = meta.effort.clone();
    if event.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX) {
        event.terminal_kind = Some("codex".into());
    }
    event.raw_event_name =
        Some(format!("{}:{event_type:?}", source.raw_prefix()).to_ascii_lowercase());
    Some(event)
}

/// Extracts display text from a content field: either a plain string or an
/// array of `{"type":"text","text":…}` blocks (Codex also uses `output_text`).
/// Tool-use / tool-result / thinking blocks are skipped.
fn extract_content_text(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => non_empty(text),
        Value::Array(blocks) => {
            let mut parts = Vec::new();
            for block in blocks {
                let block_type = block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("text");
                if !matches!(block_type, "text" | "output_text" | "input_text") {
                    continue;
                }
                if let Some(text) = block.get("text").and_then(Value::as_str).and_then(non_empty) {
                    parts.push(text);
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn string_field(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        payload
            .get(*key)
            .and_then(Value::as_str)
            .and_then(non_empty)
    })
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn timestamp_of(value: &Value) -> DateTime<Utc> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
        .unwrap_or_else(Utc::now)
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn record_timestamp(line: Option<&str>) -> DateTime<Utc> {
    line.and_then(|line| serde_json::from_str::<Value>(line).ok())
        .map(|value| timestamp_of(&value))
        .unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use codeisland_domain::{AppSettings, SessionSnapshot, SessionStatus};

    use super::*;

    #[cfg(windows)]
    #[test]
    fn configured_paths_take_precedence_over_wsl_aliases() {
        let configured = [
            windows_path_key(Path::new(r"\\wsl$\Ubuntu\home\Alice\.claude\projects")).unwrap(),
            windows_path_key(Path::new(r"C:/Users/Alice/.codex/sessions")).unwrap(),
        ];
        for path in [
            r"\\wsl.localhost\ubuntu\home\Alice\.claude\projects\one\record.jsonl",
            r"\\?\UNC\wsl.localhost\Ubuntu\home\Alice\.claude\projects",
            r"\\?\C:\USERS\ALICE\.CODEX\SESSIONS\record.jsonl",
        ] {
            assert!(covered_windows_path(Path::new(path), &configured), "{path}");
        }
        for path in [
            r"\\wsl.localhost\Ubuntu\home\alice\.claude\projects\record.jsonl",
            r"\\wsl.localhost\Debian\home\Alice\.claude\projects",
            r"C:\Users\Alice\.codex\sessions-other",
            r"C:Users\Alice\.codex\sessions",
        ] {
            assert!(!covered_windows_path(Path::new(path), &configured), "{path}");
        }
    }

    const SESSION_META_LINE: &str = r#"{"timestamp":"2026-01-02T03:04:05Z","type":"session_meta","payload":{"id":"session-42","cwd":"/work/project"}}"#;
    const AGENT_MESSAGE_LINE: &str = r#"{"timestamp":"2026-01-02T03:04:07Z","type":"event_msg","payload":{"type":"agent_message","message":"hello there"}}"#;

    const CODEX_TRANSCRIPT: &str = concat!(
        r#"{"timestamp":"2026-01-02T03:04:05Z","type":"session_meta","payload":{"id":"session-42","cwd":"/work/project"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:05Z","type":"turn_context","payload":{"model":"gpt-5-codex","effort":"high"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:06Z","type":"event_msg","payload":{"type":"task_started"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:06Z","type":"event_msg","payload":{"type":"user_message","message":"fix the bug"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:07Z","type":"event_msg","payload":{"type":"agent_message","message":"hello there"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:08Z","type":"event_msg","payload":{"type":"agent_reasoning"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:09Z","type":"event_msg","payload":{"type":"mcp_tool_call_end","invocation":{"server":"filesystem","tool":"read_file"}}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:10Z","type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:11Z","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"step one"}]}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:12Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"ls\"}"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:13Z","type":"response_item","payload":{"type":"function_call_output","name":"shell","output":"file.txt"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:13Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"text","text":"final reply"}]}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:13Z","type":"response_item","payload":{"type":"update_plan","plan":[{"step":"one"}]}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:13Z","type":"event_msg","payload":{"type":"plan_update","plan":[]}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:14Z","type":"event_msg","payload":{"type":"turn_failed","message":"boom"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:14Z","type":"event_msg","payload":{"type":"turn_aborted"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:15Z","type":"event_msg","payload":{"type":"task_complete"}}"#,
        "\n",
        r#"{"timestamp":"2026-01-02T03:04:16Z","type":"unknown_record","payload":{}}"#,
        "\n",
    );

    const CLAUDE_TRANSCRIPT: &str = concat!(
        r#"{"type":"user","sessionId":"claude-1","cwd":"/work/claude","timestamp":"2026-01-02T03:04:05Z","message":{"role":"user","content":"hello claude"}}"#,
        "\n",
        r#"{"type":"user","sessionId":"claude-1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":"ok"}]}}"#,
        "\n",
        r#"{"type":"assistant","sessionId":"claude-1","message":{"role":"assistant","model":"claude-sonnet-5","content":[{"type":"text","text":"hi back"},{"type":"tool_use","id":"tool-1","name":"Bash","input":{}}]}}"#,
        "\n",
        r#"{"type":"assistant","sessionId":"claude-1","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-2","name":"Read","input":{}}]}}"#,
        "\n",
        r#"{"type":"system","subtype":"init","sessionId":"claude-1"}"#,
        "\n",
        r#"{"type":"result","subtype":"success","sessionId":"claude-1"}"#,
        "\n",
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom","sessionId":"claude-1"}"#,
        "\n",
    );

    fn temp_dir(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "codeisland-tailer-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn set_mtime(path: &Path, age: Duration) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    fn session(runtime: &Arc<Runtime>, id: &str) -> SessionSnapshot {
        runtime
            .state()
            .store
            .sessions
            .into_iter()
            .find(|snapshot| snapshot.session_id == id)
            .expect("session should exist")
    }

    #[test]
    fn parses_codex_jsonl() {
        let mut meta = TranscriptMeta::default();
        let mut events = Vec::new();
        for line in CODEX_TRANSCRIPT.lines() {
            if let Some(event) = parse_line(TranscriptSource::Codex, line, &mut meta) {
                events.push(event);
            }
        }

        assert_eq!(meta.session_id.as_deref(), Some("session-42"));
        assert_eq!(meta.cwd.as_deref(), Some("/work/project"));
        assert_eq!(meta.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(meta.effort.as_deref(), Some("high"));
        assert_eq!(events.len(), 13);

        assert_eq!(events[0].event_type, AgentEventType::UserPrompt);
        assert_eq!(events[0].model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(events[0].reasoning.as_deref(), Some("high"));

        assert_eq!(events[1].event_type, AgentEventType::UserPrompt);
        assert_eq!(events[1].text.as_deref(), Some("fix the bug"));

        assert_eq!(events[2].event_type, AgentEventType::Message);
        assert_eq!(events[2].text.as_deref(), Some("hello there"));

        assert_eq!(events[3].event_type, AgentEventType::Heartbeat);
        assert_eq!(events[3].tool.as_deref(), Some("background"));
        assert!(events[3].text.is_none());

        assert_eq!(events[4].event_type, AgentEventType::ToolEnd);
        assert_eq!(events[4].tool.as_deref(), Some("plugin filesystem/read_file"));

        assert_eq!(events[5].event_type, AgentEventType::ToolStart);
        assert_eq!(events[5].tool.as_deref(), Some("apply_patch"));

        assert_eq!(events[6].event_type, AgentEventType::Heartbeat);
        assert!(events[6].text.is_none());

        assert_eq!(events[7].event_type, AgentEventType::ToolStart);
        assert_eq!(events[7].tool.as_deref(), Some("shell"));

        assert_eq!(events[8].event_type, AgentEventType::ToolEnd);
        assert_eq!(events[8].tool.as_deref(), Some("shell"));

        assert_eq!(events[9].event_type, AgentEventType::Message);
        assert_eq!(events[9].text.as_deref(), Some("final reply"));

        assert_eq!(events[10].event_type, AgentEventType::TurnFailed);
        assert_eq!(events[10].text.as_deref(), Some("boom"));

        assert_eq!(events[11].event_type, AgentEventType::Interrupt);
        assert_eq!(events[12].event_type, AgentEventType::TurnEnd);

        for event in &events {
            assert_eq!(event.session_id, "session-42");
            assert_eq!(event.agent, AgentKind::Codex);
            assert_eq!(event.cwd.as_deref(), Some("/work/project"));
            assert_eq!(event.model.as_deref(), Some("gpt-5-codex"));
        }
    }

    #[test]
    fn live_tail_scan_recovers_state_without_replaying_history() {
        let root = temp_dir("recover");
        let path = root.join("rollout.jsonl");
        let mut content = String::new();
        content.push_str(SESSION_META_LINE);
        content.push('\n');
        content.push_str(
            r#"{"timestamp":"2026-01-02T03:04:06Z","type":"event_msg","payload":{"type":"agent_message","message":"ancient history"}}"#,
        );
        content.push('\n');
        let padding = "x".repeat(1000);
        for index in 0..80 {
            content.push_str(&format!(
                r#"{{"type":"noise","payload":{{"n":{index},"pad":"{padding}"}}}}"#
            ));
            content.push('\n');
        }
        content.push_str(r#"{"timestamp":"2026-01-02T03:04:07Z","type":"event_msg","payload":{"type":"task_started"}}"#);
        content.push('\n');
        content.push_str(r#"{"timestamp":"2026-01-02T03:04:08Z","type":"event_msg","payload":{"type":"user_message","message":"recent prompt"}}"#);
        content.push('\n');
        content.push_str(r#"{"timestamp":"2026-01-02T03:04:09Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"text","text":"recent reply"}]}}"#);
        content.push('\n');
        content.push_str(r#"{"timestamp":"2026-01-02T03:04:10Z","type":"event_msg","payload":{"type":"task_complete"}}"#);
        content.push('\n');
        std::fs::write(&path, content).unwrap();

        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut cursors,
            &HashMap::new(),
        );

        let snapshot = session(&runtime, "session-42");
        assert_eq!(snapshot.last_prompt.as_deref(), Some("recent prompt"));
        assert_eq!(snapshot.last_message.as_deref(), Some("recent reply"));
        assert_eq!(snapshot.status, SessionStatus::Completed);
        assert!(snapshot
            .history
            .iter()
            .all(|entry| entry.text.as_deref() != Some("ancient history")));
        assert!(snapshot
            .history
            .iter()
            .all(|entry| entry.kind != "sessionstart"));

        let cursor = cursors.get(&path).unwrap();
        assert_eq!(
            cursor.offset,
            std::fs::metadata(&path).unwrap().len(),
            "the recovery scan must leave the cursor at EOF"
        );
    }

    #[test]
    fn codex_rollout_parent_metadata_uses_child_provider_namespace() {
        let metadata = [
            (serde_json::json!({
                "id": "desktop-worker", "originator": "Codex Desktop",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent"}}}
            }), "codexapp:desktop-worker", Some("codexapp:parent")),
            (serde_json::json!({
                "id": "desktop-guardian", "originator": "Codex Desktop",
                "source": {"subagent": {"other": "guardian"}},
                "parent_thread_id": "parent"
            }), "codexapp:desktop-guardian", Some("codexapp:parent")),
            (serde_json::json!({
                "id": "cli-worker", "originator": "Codex CLI",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent"}}}
            }), "cli-worker", Some("parent")),
            (serde_json::json!({
                "id": "desktop-root", "originator": "Codex Desktop",
                "source": "vscode", "parent_thread_id": "unrelated-root"
            }), "codexapp:desktop-root", None),
        ];
        for (payload, expected_id, expected_parent) in metadata {
            let mut meta = TranscriptMeta::default();
            let head = serde_json::json!({"type": "session_meta", "payload": payload});
            assert!(parse_line(TranscriptSource::Codex, &head.to_string(), &mut meta).is_none());
            let event = parse_line(
                TranscriptSource::Codex,
                r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
                &mut meta,
            ).unwrap();
            assert_eq!(event.session_id, expected_id);
            assert_eq!(event.parent_session_id.as_deref(), expected_parent);
        }
    }

    #[test]
    fn codex_checkpoint_relinks_retained_child_without_replaying_its_turn() {
        let runtime = Runtime::new_for_test(AppSettings::default());
        runtime.set_codex_desktop_running(true);
        let path = temp_dir("checkpoint-codex-child").join("child.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"child","originator":"Codex Desktop","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent","agent_nickname":"Worker"}}}}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
                "\n",
            ),
        ).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        file.set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();
        let path_key = path.to_string_lossy().into_owned();
        let mut child = AgentEvent::new("codexapp:child", AgentKind::Codex, AgentEventType::SessionStart);
        child.transcript_path = Some(path_key.clone());
        assert!(runtime.handle_event(child).applied);

        let mut cursors = HashMap::new();
        let saved = HashMap::from([(path_key, file.metadata().unwrap().len())]);
        tail_file(&runtime, TranscriptSource::Codex, &path, &mut cursors, &saved);
        let child = session(&runtime, "codexapp:child");
        assert_eq!(child.parent_session_id.as_deref(), Some("codexapp:parent"));
        assert_eq!(child.title.as_deref(), Some("Worker"));
        assert_eq!(child.status, SessionStatus::Idle);
        assert!(child.history.is_empty());

        // The parent can be discovered after the child header. Its status
        // follows its own event and its child list is filled from known links.
        let parent = AgentEvent::new("codexapp:parent", AgentKind::Codex, AgentEventType::UserPrompt);
        assert!(runtime.handle_event(parent).applied);
        let parent = session(&runtime, "codexapp:parent");
        assert_eq!(parent.status, SessionStatus::Running);
        assert_eq!(parent.child_session_ids, ["codexapp:child"]);
    }

    #[test]
    fn compact_codex_preview_uses_only_new_public_output() {
        use std::io::Write;

        let path = temp_dir("compact-output").join("rollout.jsonl");
        let started = r#"{"type":"event_msg","payload":{"type":"task_started"}}"#;
        std::fs::write(&path, format!("{SESSION_META_LINE}\n{started}\n{AGENT_MESSAGE_LINE}\n")).unwrap();
        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        let poll = |cursors: &mut HashMap<_, _>| {
            tail_file(&runtime, TranscriptSource::Codex, &path, cursors, &HashMap::new());
        };
        let append = |line: &str| {
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, "{line}").unwrap();
        };

        poll(&mut cursors);
        let recovered = session(&runtime, "session-42");
        assert_eq!(recovered.status, SessionStatus::Running);
        assert_eq!(recovered.last_message.as_deref(), Some("hello there"));
        assert_eq!(recovered.live_codex_output, None);

        append(r#"{"type":"response_item","payload":{"type":"agent_message","message":"internal"}}"#);
        append(r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"input_text","text":"private context"}]}}"#);
        poll(&mut cursors);
        assert_eq!(session(&runtime, "session-42").live_codex_output, None);

        append(r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"input_text","text":"private context"},{"type":"output_text","text":"**Progress**"},{"type":"output_text","text":"next"}]}}"#);
        poll(&mut cursors);
        assert_eq!(
            session(&runtime, "session-42").live_codex_output.as_deref(),
            Some("**Progress**\nnext")
        );

        append(r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#);
        poll(&mut cursors);
        let finished = session(&runtime, "session-42");
        assert_eq!(finished.status, SessionStatus::Completed);
        assert_eq!(finished.live_codex_output, None);

        append(AGENT_MESSAGE_LINE);
        poll(&mut cursors);
        let late = session(&runtime, "session-42");
        assert_eq!(late.status, SessionStatus::Completed);
        assert_eq!(late.live_codex_output, None);

        append(started);
        poll(&mut cursors);
        assert_eq!(session(&runtime, "session-42").live_codex_output, None);
    }

    #[test]
    fn tail_scan_without_state_emits_session_start() {
        let root = temp_dir("session-start");
        let path = root.join("rollout.jsonl");
        std::fs::write(&path, format!("{SESSION_META_LINE}\n")).unwrap();

        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut cursors,
            &HashMap::new(),
        );

        let snapshot = session(&runtime, "session-42");
        assert_eq!(snapshot.status, SessionStatus::Idle);
        assert!(snapshot.history.is_empty());
    }

    #[test]
    fn partial_utf8_across_polls() {
        let root = temp_dir("utf8");
        let path = root.join("rollout.jsonl");
        let prefix = r#"{"timestamp":"2026-01-02T03:04:07Z","type":"event_msg","payload":{"type":"agent_message","message":""#;
        let text = "你好";
        let bytes = text.as_bytes();

        let mut first = Vec::new();
        first.extend_from_slice(SESSION_META_LINE.as_bytes());
        first.push(b'\n');
        first.extend_from_slice(prefix.as_bytes());
        // "你" is three bytes; keep one byte of "好" so the character straddles
        // the poll boundary.
        first.extend_from_slice(&bytes[..4]);
        std::fs::write(&path, &first).unwrap();

        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut cursors,
            &HashMap::new(),
        );
        assert_eq!(session(&runtime, "session-42").last_message, None);

        let mut rest = Vec::new();
        rest.extend_from_slice(&bytes[4..]);
        rest.extend_from_slice(b"\"}}\n");
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&rest).unwrap();
        }

        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut cursors,
            &HashMap::new(),
        );
        assert_eq!(
            session(&runtime, "session-42").last_message.as_deref(),
            Some("你好")
        );
    }

    #[test]
    fn stable_ids_dedupe_replayed_events() {
        let root = temp_dir("stable");
        let path = root.join("rollout.jsonl");
        std::fs::write(
            &path,
            format!("{SESSION_META_LINE}\n{AGENT_MESSAGE_LINE}\n"),
        )
        .unwrap();

        let runtime = Runtime::new_for_test(AppSettings::default());
        let key = path.to_string_lossy().into_owned();
        let saved = HashMap::from([(key, 0u64)]);

        let mut first = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut first,
            &saved,
        );
        let applied = session(&runtime, "session-42").history.len();
        assert_eq!(applied, 1);

        // A crash replay starts a fresh cursor at the same saved offset and
        // re-reads the same lines; stable ids make the store ignore them.
        let mut second = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut second,
            &saved,
        );
        assert_eq!(session(&runtime, "session-42").history.len(), applied);

        assert_eq!(stable_event_id(&path, 0), stable_event_id(&path, 0));
        assert_ne!(stable_event_id(&path, 0), stable_event_id(&path, 1));
    }

    #[test]
    fn shrink_re_reads_from_the_start() {
        let root = temp_dir("shrink");
        let path = root.join("rollout.jsonl");
        let padding = "x".repeat(500);
        let original = format!(
            "{SESSION_META_LINE}\n{AGENT_MESSAGE_LINE}\n{{\"type\":\"noise\",\"payload\":{{\"pad\":\"{padding}\"}}}}\n"
        );
        std::fs::write(&path, &original).unwrap();
        set_mtime(&path, Duration::from_secs(2 * 60 * 60));

        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut cursors,
            &HashMap::new(),
        );
        assert!(runtime.state().store.sessions.is_empty());

        let replacement = format!(
            "{SESSION_META_LINE}\n{{\"timestamp\":\"2026-01-02T03:05:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"new file\"}}}}\n"
        );
        assert!(replacement.len() < original.len());
        std::fs::write(&path, replacement).unwrap();

        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut cursors,
            &HashMap::new(),
        );
        assert_eq!(
            session(&runtime, "session-42").last_message.as_deref(),
            Some("new file")
        );
    }

    #[test]
    fn old_transcripts_start_at_the_end() {
        let root = temp_dir("old");
        let path = root.join("rollout.jsonl");
        std::fs::write(
            &path,
            format!("{SESSION_META_LINE}\n{AGENT_MESSAGE_LINE}\n"),
        )
        .unwrap();
        set_mtime(&path, Duration::from_secs(2 * 60 * 60));

        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Codex,
            &path,
            &mut cursors,
            &HashMap::new(),
        );

        assert!(runtime.state().store.sessions.is_empty());
        let cursor = cursors.get(&path).unwrap();
        assert_eq!(cursor.offset, std::fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn claude_recap_is_metadata_and_newer_prompt_clears_it() {
        use std::io::Write;

        let path = temp_dir("claude-recap").join("claude-recap.jsonl");
        let now = Utc::now();
        let prompt = serde_json::json!({
            "type": "user", "sessionId": "claude-recap", "timestamp": now,
            "message": {"content": "Previous prompt"}
        });
        let finished = serde_json::json!({
            "type": "result", "subtype": "success", "timestamp": now
        });
        std::fs::write(&path, format!("{prompt}\n{finished}\n")).unwrap();
        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        let poll = |cursors: &mut HashMap<_, _>| tail_file(&runtime, TranscriptSource::Claude, &path, cursors, &HashMap::new());
        poll(&mut cursors);
        let before = session(&runtime, "claude-recap");
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let later = now + chrono::TimeDelta::seconds(1);
        for row in [
            serde_json::json!({
                "type": "system", "subtype": "away_summary", "timestamp": later,
                "content": "  Tests passed. (disable recaps in /config)  "
            }),
            serde_json::json!({
                "type": "system", "subtype": "away_summary", "timestamp": later,
                "content": "Child recap", "isSidechain": true
            }),
        ] {
            writeln!(file, "{row}").unwrap();
        }
        poll(&mut cursors);
        let recapped = session(&runtime, "claude-recap");
        assert_eq!(recapped.recap.as_deref(), Some("Tests passed."));
        assert_eq!(recapped.last_activity, before.last_activity);
        assert_eq!(recapped.status, SessionStatus::Completed);

        let local_command = serde_json::json!({
            "type": "user", "timestamp": later,
            "message": {"content": "<command-name>/model</command-name>"}
        });
        writeln!(file, "{local_command}").unwrap();
        poll(&mut cursors);
        assert_eq!(session(&runtime, "claude-recap").recap.as_deref(), Some("Tests passed."));

        let next = serde_json::json!({
            "type": "user", "timestamp": later, "message": {"content": "New prompt"}
        });
        writeln!(file, "{next}").unwrap();
        poll(&mut cursors);
        assert_eq!(session(&runtime, "claude-recap").recap, None);
    }

    #[test]
    fn codex_and_claude_transcripts_supply_task_progress() {
        use codeisland_domain::TaskStatus;

        let now = Utc::now();
        let root = temp_dir("task-progress");
        let codex = root.join("codex.jsonl");
        let codex_rows = [
            serde_json::json!({"type": "session_meta", "payload": {"id": "plan-fixture"}}),
            serde_json::json!({"type": "event_msg", "timestamp": now, "payload": {"type": "task_started"}}),
            serde_json::json!({
                "type": "response_item", "timestamp": now,
                "payload": {"type": "function_call", "name": "update_plan",
                    "arguments": serde_json::json!({"plan": [
                        {"step": "Port", "status": "completed"},
                        {"step": "Verify", "status": "in_progress"}
                    ]}).to_string()
                }
            }),
        ];
        let claude = root.join("checklist.jsonl");
        let claude_rows = [
            serde_json::json!({"type": "user", "sessionId": "checklist", "timestamp": now, "message": {"content": "Work"}}),
            serde_json::json!({
                "type": "assistant", "timestamp": now, "message": {"content": [
                    {"type": "tool_use", "id": "todo", "name": "TodoWrite",
                     "input": {"todos": [{"id": "lint", "content": "Lint", "status": "pending"}]}}
                ]}
            }),
            serde_json::json!({
                "type": "user", "timestamp": now,
                "message": {"content": [{"type": "tool_result", "tool_use_id": "task"}]},
                "toolUseResult": {"task": {"id": "7", "subject": "Test"}}
            }),
            serde_json::json!({
                "type": "user", "timestamp": now,
                "message": {"content": [{"type": "tool_result", "tool_use_id": "update"}]},
                "toolUseResult": {"success": true, "taskId": "7", "updatedFields": ["status"],
                                  "statusChange": {"from": "pending", "to": "in_progress"}}
            }),
        ];
        for (path, rows) in [(&codex, codex_rows.as_slice()), (&claude, claude_rows.as_slice())] {
            let lines = rows.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n";
            std::fs::write(path, lines).unwrap();
        }
        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        tail_file(&runtime, TranscriptSource::Codex, &codex, &mut cursors, &HashMap::new());
        tail_file(&runtime, TranscriptSource::Claude, &claude, &mut cursors, &HashMap::new());
        let plan = session(&runtime, "plan-fixture").tasks;
        assert_eq!(plan.iter().map(|row| (row.title.as_str(), row.status)).collect::<Vec<_>>(), [
            ("Port", TaskStatus::Completed), ("Verify", TaskStatus::InProgress),
        ]);
        let checklist = session(&runtime, "checklist").tasks;
        assert_eq!(checklist.iter().map(|row| (row.id.as_str(), row.status)).collect::<Vec<_>>(), [
            ("lint", TaskStatus::Pending), ("7", TaskStatus::InProgress),
        ]);
    }

    #[test]
    fn desktop_restart_restores_latest_turn_beyond_preview_window() {
        use codeisland_domain::SessionStore;

        for (id, finish, age, expected) in [
            ("long-active", false, Duration::ZERO, SessionStatus::Running),
            ("long-settled", true, Duration::ZERO, SessionStatus::Idle),
            ("long-stale", false, Duration::from_secs(360), SessionStatus::Idle),
        ] {
            let path = temp_dir(id).join("rollout.jsonl");
            let timestamp = Utc::now() - chrono::TimeDelta::seconds(10);
            let mut rows = vec![
                serde_json::json!({
                    "type": "session_meta", "timestamp": timestamp,
                    "payload": {"id": id, "source": "vscode", "originator": "Codex Desktop"}
                }),
                serde_json::json!({
                    "type": "event_msg", "timestamp": timestamp, "payload": {"type": "task_started"}
                }),
            ];
            if finish {
                rows.push(serde_json::json!({
                    "type": "event_msg", "timestamp": timestamp, "payload": {"type": "task_complete"}
                }));
            }
            rows.push(serde_json::json!({"type": "unrelated", "padding": "x".repeat(TAIL_SCAN_BYTES as usize + 1000)}));
            rows.push(serde_json::json!({
                "type": "response_item", "timestamp": timestamp,
                "payload": if finish {
                    serde_json::json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Recovered reply"}]})
                } else {
                    serde_json::json!({"type": "reasoning"})
                }
            }));
            let contents = rows.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n";
            std::fs::write(&path, contents).unwrap();
            if !age.is_zero() {
                set_mtime(&path, age);
            }
            let key = path.to_string_lossy().into_owned();
            let session_id = codex_desktop_session_id(id);
            let mut old_store = SessionStore::default();
            let mut prompt = AgentEvent::new(&session_id, AgentKind::Codex, AgentEventType::UserPrompt);
            prompt.timestamp = timestamp;
            prompt.text = Some("Retained prompt".to_owned());
            prompt.transcript_path = Some(key.clone());
            assert!(old_store.apply(&prompt).applied);
            let runtime = Runtime::build(AppSettings::default(), old_store.persisted());
            assert_eq!(session(&runtime, &session_id).status, SessionStatus::Idle);
            runtime.set_codex_desktop_running(true);

            let saved = HashMap::from([(key, path.metadata().unwrap().len())]);
            tail_file(&runtime, TranscriptSource::Codex, &path, &mut HashMap::new(), &saved);
            let snapshot = session(&runtime, &session_id);
            assert_eq!(snapshot.status, expected, "{id}");
            assert_eq!(snapshot.last_prompt.as_deref(), Some("Retained prompt"));
            if finish {
                assert_eq!(snapshot.last_message.as_deref(), Some("Recovered reply"));
            }
        }
    }

    #[test]
    fn desktop_checkpoints_skip_uninitialized_and_legacy_zero_cursors() {
        use std::io::Write;

        let root = temp_dir("desktop-checkpoint");
        let old = root.join("old.jsonl");
        let recent = root.join("recent.jsonl");
        let checkpoint = root.join("offsets.json");
        for (path, id) in [(&old, "old"), (&recent, "recent")] {
            let meta = serde_json::json!({
                "type": "session_meta",
                "payload": {"id": id, "originator": "Codex Desktop", "source": "vscode"},
            });
            std::fs::write(path, format!("{meta}\n{AGENT_MESSAGE_LINE}\n")).unwrap();
        }
        set_mtime(&old, LIVE_FILE_WINDOW + Duration::from_secs(60));
        let legacy = HashMap::from([
            (old.to_string_lossy().into_owned(), 0),
            (recent.to_string_lossy().into_owned(), 0),
        ]);
        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        for path in [&old, &recent] {
            tail_file(&runtime, TranscriptSource::Codex, path, &mut cursors, &legacy);
            assert!(!cursors[path].initialized);
        }
        save_offsets(&checkpoint, &cursors, true);
        let saved: HashMap<String, u64> =
            serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
        assert!(saved.is_empty());

        runtime.set_codex_desktop_running(true);
        tail_file(&runtime, TranscriptSource::Codex, &old, &mut cursors, &legacy);
        assert!(runtime.state().store.sessions.is_empty());
        assert_eq!(cursors[&old].offset, std::fs::metadata(&old).unwrap().len());
        tail_file(&runtime, TranscriptSource::Codex, &recent, &mut cursors, &legacy);
        assert_eq!(
            session(&runtime, "codexapp:recent").last_message.as_deref(),
            Some("hello there")
        );

        let mut old_file = std::fs::OpenOptions::new().append(true).open(&old).unwrap();
        let new_line = serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "agent_message", "message": "new activity"},
        });
        writeln!(old_file, "{new_line}").unwrap();
        drop(old_file);
        tail_file(&runtime, TranscriptSource::Codex, &old, &mut cursors, &legacy);
        let resumed = session(&runtime, "codexapp:old");
        assert_eq!(resumed.last_message.as_deref(), Some("new activity"));
        assert_eq!(resumed.history.len(), 1);
        save_offsets(&checkpoint, &cursors, true);
        let saved: HashMap<String, u64> =
            serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
        assert_eq!(saved[old.to_str().unwrap()], std::fs::metadata(&old).unwrap().len());
        assert_eq!(saved[recent.to_str().unwrap()], std::fs::metadata(&recent).unwrap().len());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_claude_jsonl() {
        let mut meta = TranscriptMeta::default();
        let mut events = Vec::new();
        for line in CLAUDE_TRANSCRIPT.lines() {
            if let Some(event) = parse_line(TranscriptSource::Claude, line, &mut meta) {
                events.push(event);
            }
        }

        assert_eq!(meta.session_id.as_deref(), Some("claude-1"));
        assert_eq!(meta.cwd.as_deref(), Some("/work/claude"));
        assert_eq!(events.len(), 4);

        assert_eq!(events[0].event_type, AgentEventType::UserPrompt);
        assert_eq!(events[0].text.as_deref(), Some("hello claude"));
        assert_eq!(events[0].agent, AgentKind::Claude);
        assert_eq!(events[0].cwd.as_deref(), Some("/work/claude"));

        assert_eq!(events[1].event_type, AgentEventType::Message);
        assert_eq!(events[1].text.as_deref(), Some("hi back"));
        assert_eq!(events[1].model.as_deref(), Some("claude-sonnet-5"));

        assert_eq!(events[2].event_type, AgentEventType::TurnEnd);
        assert_eq!(events[3].event_type, AgentEventType::TurnFailed);
        assert_eq!(events[3].text.as_deref(), Some("boom"));
    }

    #[test]
    fn claude_live_tail_emits_only_tail_state() {
        let root = temp_dir("claude-tail");
        let path = root.join("claude-1.jsonl");
        std::fs::write(&path, CLAUDE_TRANSCRIPT).unwrap();

        let runtime = Runtime::new_for_test(AppSettings::default());
        let mut cursors = HashMap::new();
        tail_file(
            &runtime,
            TranscriptSource::Claude,
            &path,
            &mut cursors,
            &HashMap::new(),
        );

        let snapshot = session(&runtime, "claude-1");
        assert_eq!(snapshot.last_prompt.as_deref(), Some("hello claude"));
        assert_eq!(snapshot.last_message.as_deref(), Some("hi back"));
        assert_eq!(snapshot.status, SessionStatus::Failed);
        assert_eq!(snapshot.error.as_deref(), Some("boom"));
        assert_eq!(snapshot.model.as_deref(), Some("claude-sonnet-5"));
        let transcript_path = path.to_string_lossy().into_owned();
        assert_eq!(snapshot.transcript_path.as_deref(), Some(transcript_path.as_str()));
        assert!(snapshot
            .history
            .iter()
            .all(|entry| entry.kind != "sessionstart"));
    }

    #[test]
    fn ignores_events_before_session_meta() {
        let mut meta = TranscriptMeta::default();
        let line = r#"{"type":"event_msg","payload":{"type":"agent_message","message":"orphan"}}"#;
        assert!(parse_line(TranscriptSource::Codex, line, &mut meta).is_none());
    }

    #[test]
    fn codex_rollout_origin_distinguishes_desktop_and_other_app_server_clients() {
        for (originator, source, id) in [
            ("Codex Desktop", "appServer", "codexapp:thread"),
            ("", "vscode", "codexapp:thread"),
            ("Claude Code", "appServer", "thread"),
            ("codex_cli_rs", "cli", "thread"),
        ] {
            let mut meta = TranscriptMeta::default();
            let line = serde_json::json!({
                "type": "session_meta",
                "payload": { "id": "thread", "originator": originator, "source": source },
            }).to_string();
            parse_line(TranscriptSource::Codex, &line, &mut meta);
            assert_eq!(meta.session_id.as_deref(), Some(id));
        }
    }

    #[test]
    fn ignores_malformed_lines() {
        let mut meta = TranscriptMeta::default();
        assert!(parse_line(TranscriptSource::Codex, "not json", &mut meta).is_none());
        assert!(parse_line(TranscriptSource::Codex, "", &mut meta).is_none());
    }
}
