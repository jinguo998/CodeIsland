//! Read-only observation of Claude Desktop Cowork and local Chat sessions.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, TimeDelta, Utc};
use codeisland_domain::cowork::{self, Audit, AuditStream, Conversation, Metadata, Phase, Update};
use serde_json::Value;

const TAIL_BYTES: u64 = 256 * 1024;
const MAX_APPEND: u64 = TAIL_BYTES * 8;
const FRESHNESS: TimeDelta = TimeDelta::minutes(10);
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Host {
    pub pid: u32,
    pub started: DateTime<Utc>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: DateTime<Utc>,
    created: Option<SystemTime>,
}

impl FileStamp {
    fn read(path: &Path) -> Option<Self> {
        let metadata = path.metadata().ok()?;
        metadata.is_file().then_some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?.into(),
            created: metadata.created().ok(),
        })
    }
}

#[derive(Default)]
struct Cursor {
    stamp: Option<FileStamp>,
    offset: u64,
    fragment: Vec<u8>,
    discard_fragment: bool,
    audit: Audit,
    stream: AuditStream,
}

struct Delta {
    live: bool,
    prompts: bool,
    turns: bool,
    permissions: bool,
}

impl Cursor {
    fn read(&mut self, path: &Path, stamp: FileStamp, restore: bool) -> Option<Delta> {
        let replacing = self.stamp.is_some_and(|old| {
            old.created != stamp.created || self.offset > stamp.size || old.modified > stamp.modified
        });
        if replacing {
            self.offset = 0;
            self.fragment.clear();
            self.discard_fragment = false;
            self.audit = Audit::default();
            self.stream = AuditStream::default();
        }
        if stamp.size == self.offset {
            self.stamp = Some(stamp);
            return None;
        }
        let mut start = self.offset;
        let limit = if restore { TAIL_BYTES } else { MAX_APPEND };
        if stamp.size - start > limit {
            start = stamp.size - TAIL_BYTES;
            self.fragment.clear();
            self.discard_fragment = true;
            self.audit = Audit::default();
            self.stream = AuditStream::default();
        }
        let data = read_range(path, start, stamp.size - start)?;
        self.offset = start + data.len() as u64;
        self.stamp = Some(stamp);
        let mut lines = data.split_inclusive(|byte| *byte == b'\n');
        let mut records = Vec::new();
        for line in &mut lines {
            if self.discard_fragment {
                self.discard_fragment = line.last() != Some(&b'\n');
                continue;
            }
            self.fragment.extend_from_slice(line);
            if line.last() != Some(&b'\n') {
                if self.fragment.len() > MAX_APPEND as usize {
                    self.fragment.clear();
                    self.discard_fragment = true;
                }
                continue;
            }
            records.append(&mut self.fragment);
        }
        let batch = self.stream.append(&records);
        self.audit = batch.audit;
        Some(Delta { live: batch.live, prompts: batch.prompts, turns: batch.turns, permissions: batch.permissions })
    }
}

#[derive(Default)]
struct Tracked {
    metadata_stamp: Option<FileStamp>,
    metadata: Option<Metadata>,
    audit_path: Option<PathBuf>,
    cursor: Cursor,
    transcript_path: Option<PathBuf>,
    transcript_stamp: Option<FileStamp>,
    conversation: Conversation,
    surfaced: bool,
    settled: bool,
    host: Option<Host>,
}

#[derive(Default)]
pub(crate) struct Watcher {
    sessions: HashMap<PathBuf, Tracked>,
    baseline_roots: HashSet<PathBuf>,
    audit_ids: HashMap<PathBuf, (FileStamp, HashSet<String>)>,
}

#[derive(Default)]
struct Scan {
    updates: Vec<Update>,
    removed: Vec<String>,
}

impl Watcher {
    fn scan(&mut self, roots: &[PathBuf], host: Option<Host>, now: DateTime<Utc>) -> Scan {
        let mut output = Scan::default();
        let mut seen = HashSet::new();
        let mut audit_paths = HashSet::new();

        for root in roots {
            let Ok(accounts) = fs::read_dir(root) else { continue };
            let baseline = self.baseline_roots.insert(root.clone());
            for account in accounts.flatten().filter(is_dir) {
                if account.file_name() == "skills-plugin" {
                    continue;
                }
                let Ok(organizations) = fs::read_dir(account.path()) else { continue };
                for organization in organizations.flatten().filter(is_dir) {
                    self.scan_org(&organization.path(), baseline, host, now, &mut seen, &mut audit_paths, &mut output);
                }
            }
        }

        self.sessions.retain(|path, tracked| {
            if !seen.contains(path) && path.try_exists().is_ok_and(|exists| !exists) {
                if tracked.surfaced && let Some(metadata) = &tracked.metadata {
                    output.removed.push(metadata.id.clone());
                }
                return false;
            }
            true
        });
        self.audit_ids.retain(|path, _| audit_paths.contains(path));
        output
    }

    fn scan_org(
        &mut self,
        organization: &Path,
        baseline: bool,
        host: Option<Host>,
        now: DateTime<Utc>,
        seen: &mut HashSet<PathBuf>,
        audit_paths: &mut HashSet<PathBuf>,
        output: &mut Scan,
    ) {
        let Ok(entries) = fs::read_dir(organization) else { return };
        let mut session_dirs = HashMap::new();
        let mut transcripts: HashMap<String, Option<(PathBuf, PathBuf)>> = HashMap::new();
        let mut metadata_paths = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if is_dir(&entry) {
                let audit = path.join("audit.jsonl");
                if !audit.is_file() {
                    continue;
                }
                audit_paths.insert(audit);
                let found = transcript_files(&path);
                for (id, transcript) in &found {
                    transcripts.entry(id.clone())
                        .and_modify(|previous| *previous = None)
                        .or_insert_with(|| Some((path.clone(), transcript.clone())));
                }
                session_dirs.insert(path, found);
            } else if path.extension().is_some_and(|extension| extension == "json")
                && path.file_stem().and_then(|stem| stem.to_str()).is_some_and(cowork::valid_id)
            {
                metadata_paths.push(path);
            }
        }
        metadata_paths.sort();

        for path in metadata_paths {
            seen.insert(path.clone());
            let Some(stamp) = FileStamp::read(&path) else { continue };
            let tracked = self.sessions.entry(path.clone()).or_default();
            let had_metadata = tracked.metadata.is_some();
            let metadata_changed = tracked.metadata_stamp != Some(stamp);
            if metadata_changed && stamp.size <= MAX_METADATA_BYTES {
                if let Some(metadata) = read_range(&path, 0, MAX_METADATA_BYTES).as_deref().and_then(Metadata::parse)
                    .filter(|metadata| path.file_stem().is_some_and(|stem| stem == metadata.id.as_str()))
                {
                    tracked.metadata = Some(metadata);
                    tracked.metadata_stamp = Some(stamp);
                }
            }
            let Some(metadata) = &tracked.metadata else { continue };
            let canonical = path.with_extension("");
            let direct = session_dirs.get(&canonical);
            let joined = metadata.cli_id.as_deref().and_then(|id| transcripts.get(id)).and_then(Option::as_ref);
            let mut audit_path = if direct.is_some() { Some(canonical.join("audit.jsonl")) }
                else { joined.map(|(directory, _)| directory.join("audit.jsonl")) };
            if audit_path.is_none() && let Some(cli_id) = metadata.cli_id.as_deref() {
                let mut matches = session_dirs.keys().filter_map(|directory| {
                    let path = directory.join("audit.jsonl");
                    let stamp = FileStamp::read(&path)?;
                    let cached = self.audit_ids.entry(path.clone()).or_insert_with(|| (stamp, audit_identities(&path, stamp)));
                    if cached.0.created != stamp.created || cached.0.size > stamp.size
                        || (!cached.1.contains(cli_id) && cached.0.modified != stamp.modified)
                    {
                        *cached = (stamp, audit_identities(&path, stamp));
                    }
                    cached.1.contains(cli_id).then_some(path)
                });
                let first = matches.next();
                if matches.next().is_none() { audit_path = first; }
            }

            let transcript_path = if let Some(direct) = direct {
                metadata.cli_id.as_deref().and_then(|id| direct.get(id)).cloned()
            } else {
                joined.map(|(_, transcript)| transcript.clone())
            };
            let transcript_stamp = transcript_path.as_deref().and_then(FileStamp::read);
            let transcript_changed = tracked.transcript_path != transcript_path
                || tracked.transcript_stamp != transcript_stamp;
            if transcript_changed {
                tracked.conversation = transcript_path.as_deref().zip(transcript_stamp)
                    .map(|(path, stamp)| read_conversation(path, stamp)).unwrap_or_default();
                tracked.transcript_stamp = transcript_stamp;
                tracked.transcript_path = transcript_path;
            }
            if tracked.audit_path != audit_path {
                tracked.cursor = Cursor::default();
                tracked.audit_path = audit_path;
            }
            let audit_file = tracked.audit_path.as_deref()
                .and_then(|path| FileStamp::read(path).map(|stamp| (path, stamp)));
            let transcript_activity = if baseline { None } else { transcript_stamp.map(|stamp| stamp.modified) };
            let last_activity = metadata.last_activity.into_iter()
                .chain(audit_file.map(|(_, stamp)| stamp.modified))
                .chain(transcript_activity)
                .max().unwrap_or(stamp.modified);

            let restore = baseline && !had_metadata && host.is_some() && metadata.trackable()
                && audit_file.is_some_and(|(_, stamp)| stamp.size > 0)
                && now - last_activity <= FRESHNESS;
            let delta = if let Some((path, stamp)) = audit_file {
                if baseline && !had_metadata && !restore {
                    tracked.cursor.stamp = Some(stamp);
                    tracked.cursor.offset = stamp.size;
                    None
                } else {
                    tracked.cursor.read(path, stamp, restore)
                }
            } else {
                None
            };
            if !metadata.trackable() {
                if tracked.surfaced {
                    output.removed.push(metadata.id.clone());
                    tracked.surfaced = false;
                }
                continue;
            }
            let live = !baseline && delta.as_ref().is_some_and(|delta| delta.live);
            if live || restore {
                tracked.surfaced = true;
                tracked.host = host;
                tracked.settled = false;
            }
            if !tracked.surfaced {
                continue;
            }
            let timeout = match tracked.cursor.audit.phase {
                Phase::Idle => None,
                Phase::Running => Some(TimeDelta::minutes(30)),
                Phase::WaitingApproval | Phase::WaitingQuestion => Some(TimeDelta::hours(4)),
            };
            let settle = !tracked.settled && timeout.is_some_and(|timeout| {
                (restore && host.is_some_and(|host| last_activity < host.started))
                    || tracked.host.is_some_and(|previous| Some(previous) != host)
                    || now - last_activity > timeout
            });
            tracked.settled |= settle;
            if !live && !restore && !settle
                && (tracked.settled || tracked.cursor.audit.phase == Phase::Idle)
                && now - last_activity > FRESHNESS
            {
                tracked.surfaced = false;
                output.removed.push(metadata.id.clone());
                continue;
            }
            if live || restore || settle || metadata_changed || transcript_changed {
                output.updates.push(Update {
                    metadata: metadata.clone(),
                    audit: tracked.cursor.audit.clone(),
                    conversation: tracked.conversation.clone(),
                    transcript_path: tracked.transcript_path.as_ref().map(|path| path.to_string_lossy().into_owned()),
                    last_activity,
                    restore,
                    live,
                    settle,
                    prompts_started: delta.as_ref().is_some_and(|delta| delta.prompts),
                    turns_completed: delta.as_ref().is_some_and(|delta| delta.turns),
                    permissions_requested: delta.as_ref().is_some_and(|delta| delta.permissions),
                });
            }
        }
    }
}

fn is_dir(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_dir())
}

fn transcript_files(directory: &Path) -> HashMap<String, PathBuf> {
    let Ok(projects) = fs::read_dir(directory.join(".claude").join("projects")) else {
        return HashMap::new();
    };
    let mut files = HashMap::new();
    for project in projects.flatten().filter(is_dir) {
        let Ok(entries) = fs::read_dir(project.path()) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "jsonl")
                && entry.file_type().is_ok_and(|kind| kind.is_file())
                && let Some(id) = path.file_stem().and_then(|stem| stem.to_str())
            {
                files.entry(id.to_owned()).or_insert(path);
            }
        }
    }
    files
}

fn read_range(path: &Path, start: u64, count: u64) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::with_capacity(count as usize);
    file.take(count).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn tail(path: &Path, stamp: FileStamp) -> Vec<u8> {
    let start = stamp.size.saturating_sub(TAIL_BYTES);
    let mut bytes = read_range(path, start, stamp.size - start).unwrap_or_default();
    if start > 0 {
        let partial = bytes.iter().position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |newline| newline + 1);
        bytes.drain(..partial);
    }
    bytes
}

fn audit_identities(path: &Path, stamp: FileStamp) -> HashSet<String> {
    let head = read_range(path, 0, stamp.size.min(64 * 1024)).unwrap_or_default();
    let recent = if stamp.size > 64 * 1024 { tail(path, stamp) } else { Vec::new() };
    head.split_inclusive(|byte| *byte == b'\n').chain(recent.split_inclusive(|byte| *byte == b'\n'))
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter_map(|row| row["session_id"].as_str().map(ToOwned::to_owned))
        .collect()
}

fn read_conversation(path: &Path, stamp: FileStamp) -> Conversation {
    let mut conversation = Conversation::default();
    for row in tail(path, stamp).split_inclusive(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
    {
        let content = &row["message"]["content"];
        let text = content.as_str().map(ToOwned::to_owned).or_else(|| {
            let parts: Vec<_> = content.as_array()?.iter().filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str()).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        });
        if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
            match row["type"].as_str() {
                Some("user") if row["isMeta"] != true && row["isSynthetic"] != true => {
                    conversation.prompt = Some(text);
                    conversation.response = None;
                }
                Some("assistant") => conversation.response = Some(text),
                _ => {}
            }
        }
        if let Some(model) = row["message"]["model"].as_str() {
            conversation.model = Some(model.to_owned());
        }
    }
    conversation
}

#[cfg(windows)]
pub(crate) async fn watch(runtime: std::sync::Arc<crate::Runtime>) {
    let roots = super::cowork_host::roots();
    let mut watcher = Watcher::default();
    let mut was_enabled = None;
    loop {
        let enabled = runtime.settings().track_claude_desktop_cowork;
        if enabled {
            let roots = roots.clone();
            let Ok((next, scan)) = tokio::task::spawn_blocking(move || {
                let scan = watcher.scan(&roots, super::cowork_host::running(), Utc::now());
                (watcher, scan)
            }).await else { return };
            watcher = next;
            if was_enabled != Some(true) {
                runtime.reconcile_cowork(&scan.updates);
            }
            for id in scan.removed {
                runtime.remove_session(&format!("{}{}", cowork::PREFIX, id));
            }
            for update in scan.updates {
                runtime.observe_cowork(update);
            }
        } else if was_enabled != Some(false) {
            watcher = Watcher::default();
            runtime.reconcile_cowork(&[]);
        }
        was_enabled = Some(enabled);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, AppSettings, SessionStatus};
    use serde_json::json;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "codeisland-cowork-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("account").join("organization")).unwrap();
            Self(root)
        }

        fn org(&self) -> PathBuf {
            self.0.join("account").join("organization")
        }

        fn metadata(&self, archived: bool) {
            let data = json!({
                "sessionId": "local_task",
                "cliSessionId": "sdk-id",
                "title": "Build review",
                "cwd": "C:\\work\\project",
                "createdAt": Utc::now().timestamp_millis(),
                "sessionType": "chat",
                "isArchived": archived,
            });
            fs::write(self.org().join("local_task.json"), data.to_string()).unwrap();
        }

        fn audit(&self, directory: &str, bytes: &[u8]) -> PathBuf {
            let dir = self.org().join(directory);
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("audit.jsonl");
            File::options().append(true).create(true).open(&path)
                .unwrap().write_all(bytes).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn host() -> Host {
        Host { pid: 100, started: Utc::now() - TimeDelta::hours(1) }
    }

    #[test]
    fn live_cowork_turns_and_requests_use_the_configured_sound_cues() {
        use codeisland_domain::attention::{CUE_PROMPT, CUE_REQUEST, CUE_SESSION_START};

        let fixture = Fixture::new();
        let roots = std::slice::from_ref(&fixture.0);
        let mut watcher = Watcher::default();
        let host = Some(host());
        let runtime = crate::runtime::tests::runtime_with(AppSettings {
            sound_enabled: true,
            audible_cues: [CUE_SESSION_START, CUE_PROMPT, CUE_REQUEST].map(str::to_owned).to_vec(),
            ..AppSettings::default()
        });
        let mut updates = runtime.subscribe();
        fixture.metadata(false);
        assert!(watcher.scan(roots, host, Utc::now()).updates.is_empty());

        for (line, expected) in [
            (r#"{"type":"user","session_id":"sdk-id","message":{"content":"First prompt"}}"#, CUE_SESSION_START),
            (r#"{"type":"user","session_id":"sdk-id","message":{"content":"Next prompt"}}"#, CUE_PROMPT),
            (r#"{"type":"system","subtype":"permission_request","session_id":"sdk-id","uuid":"approval","tool_name":"Bash","tool_input":{"command":"git status"}}"#, CUE_REQUEST),
        ] {
            fixture.audit("local_task", format!("{line}\n").as_bytes());
            for update in watcher.scan(roots, host, Utc::now()).updates {
                runtime.observe_cowork(update);
            }
            let cues = std::iter::from_fn(|| updates.try_recv().ok()).filter_map(|update| match update {
                crate::RuntimeUpdate::Attention { decision, .. } => decision.sound,
                _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(cues, [expected]);
        }
    }

    #[test]
    fn windows_directory_joins_audit_and_transcript_and_keeps_waits_read_only() {
        let fixture = Fixture::new();
        let roots = std::slice::from_ref(&fixture.0);
        let mut watcher = Watcher::default();
        let host = Some(host());
        let runtime = crate::runtime::tests::runtime_with(AppSettings {
            collapsed: true,
            follow_up_reminder_minutes: 1,
            ..AppSettings::default()
        });
        fixture.metadata(false);
        assert!(watcher.scan(roots, host, Utc::now()).updates.is_empty());
        fixture.audit("d4a73e9f", br#"{"type":"user","session_id":"sdk-id","message":{"content":"Review the build"}}
{"type":"system","subtype":"permission_request","session_id":"sdk-id","uuid":"approval","tool_name":"Bash","tool_input":{"command":"git status"}}
{"type":"system","subtype":"permission_request","session_id":"sdk-id","uuid":"question","tool_name":"AskUserQuestion","tool_input":{"question":"Which branch?"}}
"#);
        let mut scan = watcher.scan(roots, host, Utc::now());
        assert_eq!(scan.updates.len(), 1);
        runtime.observe_cowork(scan.updates.remove(0));
        let state = runtime.state();
        assert!(state.store.pending.is_empty());
        assert!(state.surface.is_collapsed());
        let session = &state.store.sessions[0];
        assert_eq!(session.session_id, "cowork:local_task");
        assert_eq!(session.status, SessionStatus::WaitingAnswer);
        assert_eq!(session.project.as_deref(), Some("project"));
        assert_eq!(session.active_tool_detail.as_deref(), Some("Which branch?"));

        let transcript = fixture.org().join("d4a73e9f/.claude/projects/example");
        fs::create_dir_all(&transcript).unwrap();
        fs::write(transcript.join("sdk-id.jsonl"), br#"{"type":"assistant","message":{"model":"fixture-model","content":[{"type":"text","text":"Reviewed."}]}}
"#).unwrap();
        fixture.audit("d4a73e9f", br#"{"type":"system","subtype":"permission_response","session_id":"sdk-id","uuid":"question"}
"#);
        let scan = watcher.scan(roots, host, Utc::now());
        for update in scan.updates {
            runtime.observe_cowork(update);
        }
        let state = runtime.state();
        assert!(state.store.pending.is_empty());
        let session = &state.store.sessions[0];
        assert_eq!(session.status, SessionStatus::WaitingPermission);
        assert_eq!(session.active_tool_detail.as_deref(), Some("git status"));
        assert_eq!(session.last_message.as_deref(), Some("Reviewed."));
        assert_eq!(session.model.as_deref(), Some("fixture-model"));
        assert!(session.transcript_path.as_deref().unwrap().ends_with("sdk-id.jsonl"));

        runtime.handle_event(AgentEvent::new("sdk-id", AgentKind::Claude, AgentEventType::SessionStart));
        assert_eq!(runtime.state().store.sessions.len(), 1);
        assert_eq!(runtime.state().store.sessions[0].session_id, "sdk-id");
        fixture.metadata(true);
        assert_eq!(watcher.scan(roots, host, Utc::now()).removed, ["local_task"]);
    }

    #[test]
    fn launch_rebuilds_recent_turn_silently_then_settles_on_host_restart() {
        let fixture = Fixture::new();
        let roots = std::slice::from_ref(&fixture.0);
        fixture.metadata(false);
        fixture.audit("local_task", br#"{"type":"system","subtype":"init"}
"#);
        let mut watcher = Watcher::default();
        let host = host();
        let initial = watcher.scan(roots, Some(host), Utc::now());
        assert_eq!(initial.updates.len(), 1);
        assert!(initial.updates[0].restore);
        assert!(!initial.updates[0].live);
        assert_eq!(initial.updates[0].audit.phase, Phase::Running);
        let runtime = crate::runtime::tests::runtime_with(AppSettings::default());
        runtime.reconcile_cowork(&initial.updates);
        runtime.observe_cowork(initial.updates[0].clone());
        assert_eq!(runtime.state().store.sessions[0].status, SessionStatus::Running);
        let restarted = Host { pid: host.pid + 1, started: Utc::now() };
        let changed = watcher.scan(roots, Some(restarted), Utc::now());
        assert_eq!(changed.updates.len(), 1);
        assert!(changed.updates[0].settle);
        runtime.observe_cowork(changed.updates[0].clone());
        assert_eq!(runtime.state().store.sessions[0].status, SessionStatus::Idle);
        runtime.reconcile_cowork(&[]);
        assert!(runtime.state().store.sessions.is_empty());
        assert!(watcher.scan(roots, Some(restarted), Utc::now()).updates.is_empty());
        assert_eq!(
            watcher.scan(roots, Some(restarted), Utc::now() + TimeDelta::minutes(11)).removed,
            ["local_task"],
        );
        let mut old = Watcher::default();
        assert!(old.scan(roots, Some(host), Utc::now() + TimeDelta::minutes(11)).updates.is_empty());
    }

    #[test]
    fn partial_audit_does_not_move_turn_until_line_completes() {
        let fixture = Fixture::new();
        fixture.metadata(false);
        let roots = std::slice::from_ref(&fixture.0);
        let mut watcher = Watcher::default();
        watcher.scan(roots, Some(host()), Utc::now());
        fixture.audit("local_task", br#"{"type":"result","is_error":true,"terminal_reason":"aborted_"#);
        assert!(watcher.scan(roots, Some(host()), Utc::now()).updates.is_empty());
        fixture.audit("local_task", b"streaming\"}\n");
        let scan = watcher.scan(roots, Some(host()), Utc::now());
        assert_eq!(scan.updates.len(), 1);
        assert!(scan.updates[0].turns_completed);
        assert!(scan.updates[0].audit.interrupted);
        assert!(!scan.updates[0].audit.failed);
    }
}
