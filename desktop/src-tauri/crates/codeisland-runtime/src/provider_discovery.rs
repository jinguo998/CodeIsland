//! Read-only recovery from the local provider stores.

use std::collections::HashMap;
#[cfg(windows)]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use codeisland_domain::{
    AgentKind, SessionStatus, codex_desktop_session_id, discovery::DiscoveredSession,
};
use serde::Deserialize;
use serde_json::json;

use crate::tailer::{CodexOrigin, RecoveredTurn, codex_origin, read_recovery_summary};

const STATE_DB: &str = "state_5.sqlite";
const ACTIVE_LIMIT: TimeDelta = TimeDelta::minutes(5);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexRow {
    id: String,
    path: String,
    cwd: Option<String>,
    modified: f64,
    created: Option<f64>,
    source: String,
    originator: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
    title: Option<String>,
    prompt: Option<String>,
    branch: Option<String>,
    parent: Option<String>,
    edge_status: Option<String>,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn sqlite_time(seconds: f64) -> Option<DateTime<Utc>> {
    if seconds.is_finite() && seconds > 0.0 {
        DateTime::from_timestamp_millis((seconds * 1000.0).round() as i64)
    } else {
        None
    }
}

pub(crate) fn read_codex_desktop(path: &Path, now: DateTime<Utc>) -> std::io::Result<Vec<DiscoveredSession>> {
    let rows: Option<Vec<CodexRow>> = codeisland_swift::call(c"codex.storeRows", json!({
        "path": path, "now": now.timestamp_millis() as f64 / 1_000.0,
    }));
    let rows = rows.ok_or_else(|| std::io::Error::other("could not open the Codex state database"))?;
    Ok(rows.into_iter().filter_map(|row| recover_codex(row, path, now)).collect())
}

fn recover_codex(row: CodexRow, state_path: &Path, now: DateTime<Utc>) -> Option<DiscoveredSession> {
    if row.id.trim().is_empty() || row.path.trim().is_empty() {
        return None;
    }
    let mut rollout = PathBuf::from(&row.path);
    if !rollout.is_absolute() {
        rollout = state_path.parent()?.join(rollout);
    }
    let stat = rollout.metadata().ok()?;
    if !stat.is_file() { return None; }
    let modified = sqlite_time(row.modified)?.max(DateTime::from(stat.modified().ok()?));
    if now - modified > TimeDelta::minutes(10) { return None; }

    let summary = read_recovery_summary(&rollout, AgentKind::Codex, &row.id)?;
    if summary.raw_id.as_deref().is_some_and(|id| id != row.id) || summary.origin == CodexOrigin::Cli {
        return None;
    }
    let subagent_source = row.source.trim_start().starts_with('{');
    let db_origin = codex_origin(row.originator.as_deref(), Some(&row.source), subagent_source);
    if db_origin == CodexOrigin::Cli
        || (subagent_source && summary.origin != CodexOrigin::Desktop && db_origin != CodexOrigin::Desktop)
    {
        return None;
    }

    let parent = row.parent.or(summary.parent_id).filter(|id| id != &row.id);
    let finished_by_edge = row.edge_status.as_deref().is_some_and(|status| {
        matches!(status, "completed" | "done" | "failed" | "errored" | "closed" | "shutdown")
    });
    let status: Option<SessionStatus> = codeisland_swift::call(c"codex.visibleStatus", json!({
        "now": now.timestamp_millis() as f64 / 1_000.0,
        "modified": modified.timestamp_millis() as f64 / 1_000.0,
        "running": summary.turn == Some(RecoveredTurn::Running) && !finished_by_edge,
    }));
    let status = status?;
    let cwd = row.cwd.or(summary.cwd).filter(|cwd| !subagent_worktree(cwd));
    Some(DiscoveredSession {
        session_id: codex_desktop_session_id(&row.id),
        provider_session_id: row.id,
        claude_desktop_session_id: None,
        agent: AgentKind::Codex,
        finished_subagent: parent.is_some() && status == SessionStatus::Idle,
        parent_session_id: parent.as_deref().map(codex_desktop_session_id),
        title: row.title.or(summary.codex_worker_title),
        cwd,
        git_branch: row.branch,
        model: row.model.or(summary.model),
        reasoning: row.reasoning.or(summary.reasoning),
        last_prompt: summary.last_prompt.or(row.prompt),
        last_message: summary.last_message,
        transcript_path: rollout.to_string_lossy().into_owned(),
        pid: None,
        started_at: row.created.and_then(sqlite_time),
        modified_at: modified,
        status,
    })
}

fn subagent_worktree(path: &str) -> bool {
    let path = path.replace('\\', "/");
    path.contains("/.claude/worktrees/agent-") || path.contains("/.git/worktrees/agent-")
}

#[cfg(windows)]
fn codex_state_paths(settings: &codeisland_domain::AppSettings) -> Vec<PathBuf> {
    let paths = codeisland_agents::paths::Paths::current().with_settings(settings);
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() { roots.push(home.join(".codex")); }
    roots.push(paths.codex_config_dir());
    if let Some(sqlite) = std::env::var_os("CODEX_SQLITE_HOME").filter(|value| !value.is_empty()) {
        roots.push(PathBuf::from(sqlite));
    }
    roots.extend(settings.extra_config_dirs.iter().filter(|root| !root.trim().is_empty()).map(|root| paths.expand(root)));
    let mut seen = HashSet::new();
    roots.into_iter().flat_map(|root| [root.join(STATE_DB), root.join("sqlite").join(STATE_DB)])
        .filter(|path| seen.insert(path.clone())).collect()
}

fn merge_sources(output: &mut HashMap<String, DiscoveredSession>, records: Vec<DiscoveredSession>) {
    for record in records {
        match output.entry(record.session_id.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => { entry.insert(record); }
            std::collections::hash_map::Entry::Occupied(mut entry) if record.modified_at > entry.get().modified_at => {
                entry.insert(record);
            }
            _ => {}
        }
    }
}

#[cfg(windows)]
pub(crate) async fn watch(runtime: std::sync::Arc<crate::Runtime>) {
    loop {
        let settings = runtime.settings();
        let codex_running = runtime.is_codex_desktop_running();
        if let Ok(records) = tokio::task::spawn_blocking(move || {
            let now = Utc::now();
            let mut output = HashMap::new();
            if codex_running {
                for path in codex_state_paths(&settings).into_iter().filter(|path| path.is_file()) {
                    if let Ok(records) = read_codex_desktop(&path, now) {
                        merge_sources(&mut output, records);
                    }
                }
            }
            merge_sources(&mut output, read_claude_configured(&settings, now));
            output.into_values().collect::<Vec<_>>()
        }).await {
            runtime.merge_provider_discovery(records);
        }
        tokio::time::sleep(Duration::from_secs(6)).await;
    }
}

#[derive(Clone, Debug)]
struct ClaudeRecord {
    id: String,
    cwd: String,
    title: Option<String>,
    desktop_session_id: Option<String>,
    status: Option<(SessionStatus, DateTime<Utc>)>,
    pid: u32,
    started: DateTime<Utc>,
}

fn read_claude_record(
    path: &Path,
    pid: u32,
    inspect: &impl Fn(u32) -> Option<DateTime<Utc>>,
) -> Option<ClaudeRecord> {
    let record: Option<serde_json::Value> = codeisland_swift::call(c"claude.readRecord", path);
    let record = record?;
    if record["pid"].as_u64()? != u64::from(pid) {
        return None;
    }
    let domain = record["pidDomain"].as_str().unwrap_or_default();
    let domain = domain.split(':').next().unwrap_or_default().to_ascii_lowercase();
    if matches!(domain.as_str(), "linux" | "wsl") {
        return None;
    }
    let cwd = record["cwd"].as_str()?.trim();
    if cwd.is_empty() || subagent_worktree(cwd) {
        return None;
    }
    let native_windows = domain == "win32";
    let raw_start = record["procStart"].as_str()?;
    let platform_time = if native_windows {
        Some(crate::process::windows_filetime(raw_start.parse().ok()?)?.timestamp_millis())
    } else {
        DateTime::parse_from_rfc3339(raw_start).ok().map(|date| date.timestamp_millis())
    };
    let started = inspect(pid)?;
    let id: Option<String> = codeisland_swift::call(c"claude.recordID", json!({
        "record": record, "pid": pid, "started": started.timestamp_millis(),
        "platformTime": platform_time, "exact": native_windows,
    }));
    let id = id.filter(|id| id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))?;
    let desktop_session_id = record["hostSessionId"].as_str()
        .filter(|id| codeisland_domain::claude_desktop_code::valid_id(id)).map(ToOwned::to_owned);
    let status = record["statusUpdatedAt"].as_i64()
        .and_then(DateTime::from_timestamp_millis)
        .filter(|updated| *updated >= started - TimeDelta::seconds(1))
        .and_then(|updated| match record["status"].as_str()? {
            "busy" => Some((SessionStatus::Running, updated)),
            "idle" => Some((SessionStatus::Idle, updated)),
            _ => None,
        });
    Some(ClaudeRecord {
        id,
        cwd: cwd.to_owned(),
        title: non_empty(record["name"].as_str().map(ToOwned::to_owned)),
        desktop_session_id,
        status,
        pid,
        started,
    })
}

pub(crate) fn read_claude_sessions(
    root: &Path,
    now: DateTime<Utc>,
    inspect: impl Fn(u32) -> Option<DateTime<Utc>>,
) -> Vec<DiscoveredSession> {
    let Ok(records) = std::fs::read_dir(root.join("sessions")) else { return Vec::new() };
    let mut candidates: HashMap<String, ClaudeRecord> = records.flatten().filter_map(|entry| {
        let path = entry.path();
        if path.extension()?.to_str()? != "json" { return None; }
        let pid: u32 = path.file_stem()?.to_str()?.parse().ok()?;
        let record = read_claude_record(&path, pid, &inspect)?;
        Some((record.id.clone(), record))
    }).collect();
    if candidates.is_empty() { return Vec::new(); }

    let Ok(projects) = std::fs::read_dir(root.join("projects")) else { return Vec::new() };
    let mut sessions = Vec::new();
    for project in projects.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else { continue };
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") { continue; }
            let Some(record) = candidates.get(id) else { continue };
            let Ok(metadata) = file.metadata() else { continue };
            if !metadata.is_file() { continue; }
            let Ok(transcript_modified) = metadata.modified().map(DateTime::<Utc>::from) else { continue };
            let status = record.status.filter(|(_, updated)| *updated <= now + TimeDelta::seconds(1));
            let modified = status.map_or(transcript_modified, |(_, updated)| transcript_modified.max(updated));
            if transcript_modified < record.started - TimeDelta::seconds(10) || now - modified > ACTIVE_LIMIT {
                continue;
            }
            let Some(summary) = read_recovery_summary(&path, AgentKind::Claude, id) else { continue };
            if summary.raw_id.as_deref().is_some_and(|raw| raw != id)
                || summary.cwd.as_deref().is_some_and(|cwd| {
                    !codeisland_domain::discovery::same_transcript_path(cwd, &record.cwd)
                })
            {
                continue;
            }
            let record = candidates.remove(id).expect("record was validated above");
            sessions.push(DiscoveredSession {
                session_id: record.id.clone(),
                provider_session_id: record.id,
                claude_desktop_session_id: record.desktop_session_id,
                agent: AgentKind::Claude,
                parent_session_id: None,
                title: record.title,
                cwd: Some(record.cwd),
                git_branch: None,
                model: summary.model,
                reasoning: summary.reasoning,
                last_prompt: summary.last_prompt,
                last_message: summary.last_message,
                transcript_path: path.to_string_lossy().into_owned(),
                pid: Some(record.pid),
                started_at: Some(record.started),
                modified_at: modified,
                status: status.map_or_else(|| {
                    if summary.turn == Some(RecoveredTurn::Running) { SessionStatus::Running } else { SessionStatus::Idle }
                }, |(status, _)| status),
                finished_subagent: false,
            });
        }
    }
    sessions
}

#[cfg(windows)]
fn read_claude_configured(settings: &codeisland_domain::AppSettings, now: DateTime<Utc>) -> Vec<DiscoveredSession> {
    let paths = codeisland_agents::paths::Paths::current().with_settings(settings);
    paths.claude_discovery_dirs().into_iter().flat_map(|root| {
        read_claude_sessions(&root, now, |pid| {
            let (image, started) = crate::process::windows_process_info(pid)?;
            let name = image.rsplit(['\\', '/']).next()?;
            if !name.eq_ignore_ascii_case("claude.exe") { return None; }
            let lower = image.to_ascii_lowercase();
            (!lower.contains("\\windowsapps\\claude_") && !lower.contains("\\claude\\app-"))
                .then_some(started)
        })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, FileTimes};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use codeisland_domain::{AgentEvent, AgentEventType, AppSettings};
    use rusqlite::{Connection, params};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "codeisland-discovery-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn codex(&self, id: &str, origin: &str, working: bool, at: DateTime<Utc>) -> PathBuf {
            let path = self.0.join(format!("{id}.jsonl"));
            let mut file = File::create(&path).unwrap();
            for row in [
                json!({ "type":"session_meta", "payload": { "id":id, "source":"vscode", "originator":origin, "cwd":"C:/work" } }),
                json!({ "type":"turn_context", "payload": { "model":"rollout-model", "effort":"high" } }),
                json!({ "type":"event_msg", "payload": { "type":"user_message", "message":"Read this" } }),
                json!({ "type":"event_msg", "payload": { "type":if working { "task_started" } else { "task_complete" } } }),
            ] {
                writeln!(file, "{row}").unwrap();
            }
            file.set_times(FileTimes::new().set_modified(at.into())).unwrap();
            path
        }

        fn sqlite(&self) -> Connection {
            let db = Connection::open(self.0.join(STATE_DB)).unwrap();
            db.execute_batch(
                "CREATE TABLE threads (
                    id TEXT, rollout_path TEXT, cwd TEXT, archived INTEGER,
                    source TEXT, updated_at INTEGER, updated_at_ms INTEGER,
                    created_at INTEGER, name TEXT, title TEXT, model TEXT,
                    preview TEXT, originator TEXT
                );
                CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT, status TEXT);",
            ).unwrap();
            db
        }

        fn row(
            &self, db: &Connection, id: &str, path: &Path, modified: DateTime<Utc>,
            source: &str, archived: bool,
        ) {
            db.execute(
                "INSERT INTO threads VALUES (?1, ?2, 'C:/work', ?3, ?4, 1, ?5, ?6,
                 'State name', 'Older title', 'state-model', 'DB preview', NULL)",
                params![
                    id, path.to_string_lossy().as_ref(), i32::from(archived), source,
                    modified.timestamp_millis(), (modified - TimeDelta::hours(2)).timestamp(),
                ],
            ).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn sqlite_filters_origin_archives_and_freshness_and_loads_spawn_edges() {
        let fixture = Fixture::new();
        let now = Utc::now();
        let db = fixture.sqlite();
        for (id, origin, active, age, source, archived) in [
            ("parent", "Codex Desktop", true, 120, "vscode", false),
            ("child", "Codex Desktop", true, 90, r#"{"subagent":{}}"#, false),
            ("fresh-idle", "Codex Desktop", false, 10, "appServer", false),
            ("settled-child", "Codex Desktop", true, 10, r#"{"subagent":{}}"#, false),
            ("old-idle", "Codex Desktop", false, 31, "vscode", false),
            ("old-active", "Codex Desktop", true, 301, "vscode", false),
            ("archived", "Codex Desktop", true, 1, "vscode", true),
            ("plugin", "Claude Code", true, 2, "vscode", false),
            ("cli", "codex_cli_rs", true, 2, "cli", false),
            ("unknown-child", "", true, 2, r#"{"subagent":{}}"#, false),
        ] {
            let at = now - TimeDelta::seconds(age);
            let path = fixture.codex(id, origin, active, at);
            if id == "unknown-child" {
                let mut unknown = File::create(&path).unwrap();
                writeln!(unknown, r#"{{"type":"session_meta","payload":{{"id":"unknown-child"}}}}"#).unwrap();
                writeln!(unknown, r#"{{"type":"event_msg","payload":{{"type":"task_started"}}}}"#).unwrap();
                unknown.set_times(FileTimes::new().set_modified(at.into())).unwrap();
            }
            fixture.row(&db, id, &path, at, source, archived);
        }
        db.execute_batch(
            "INSERT INTO thread_spawn_edges VALUES ('parent','child','running'),
                                                  ('parent','settled-child','completed');"
        ).unwrap();
        drop(db);

        let records = read_codex_desktop(&fixture.0.join(STATE_DB), now).unwrap();
        let mut by_id = HashMap::new();
        merge_sources(&mut by_id, records.clone());
        merge_sources(&mut by_id, records);
        assert_eq!(by_id.len(), 4);
        let parent = &by_id["codexapp:parent"];
        assert_eq!(parent.status, SessionStatus::Running);
        assert_eq!(parent.title.as_deref(), Some("State name"));
        assert_eq!(parent.model.as_deref(), Some("state-model"));
        assert_eq!(parent.reasoning.as_deref(), Some("high"));
        assert_eq!(parent.last_prompt.as_deref(), Some("Read this"));
        assert_eq!(by_id["codexapp:child"].parent_session_id.as_deref(), Some("codexapp:parent"));
        assert!(by_id["codexapp:settled-child"].finished_subagent);
        assert_eq!(by_id["codexapp:fresh-idle"].status, SessionStatus::Idle);
        let absent = fixture.0.join("does-not-exist.sqlite");
        assert!(read_codex_desktop(&absent, now).is_err());
        assert!(!absent.exists());
    }

    #[test]
    fn sqlite_legacy_schema_recovers_a_turn_older_than_the_preview() {
        let fixture = Fixture::new();
        let now = Utc::now();
        let path = fixture.codex("long", "", true, now);
        let mut file = File::options().append(true).open(&path).unwrap();
        writeln!(file, "{}", json!({
            "type":"response_item", "payload": { "type":"reasoning", "content":"x".repeat(300 * 1024) }
        })).unwrap();
        let db = Connection::open(fixture.0.join(STATE_DB)).unwrap();
        db.execute_batch(
            "CREATE TABLE threads (id TEXT, rollout_path TEXT, cwd TEXT, archived INTEGER,
              source TEXT, updated_at INTEGER);"
        ).unwrap();
        db.execute(
            "INSERT INTO threads VALUES ('long', ?1, 'C:/work', 0, 'vscode', ?2)",
            params![path.to_string_lossy().as_ref(), now.timestamp()],
        ).unwrap();
        let records = read_codex_desktop(&fixture.0.join(STATE_DB), now).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, SessionStatus::Running);
    }

    #[test]
    fn runtime_merges_native_without_rekeying_cli_or_reopening_closed_threads() {
        let fixture = Fixture::new();
        let now = Utc::now();
        let path = fixture.codex("shared", "", true, now);
        let db = fixture.sqlite();
        fixture.row(&db, "shared", &path, now, "vscode", false);
        let records = read_codex_desktop(&fixture.0.join(STATE_DB), now).unwrap();
        let runtime = crate::runtime::tests::runtime_with(AppSettings::default());
        assert!(!runtime.is_codex_desktop_running());
        runtime.merge_provider_discovery(records.clone());
        assert!(runtime.state().store.sessions.is_empty());
        runtime.set_codex_desktop_running(true);

        let mut cli = AgentEvent::new("shared", AgentKind::Codex, AgentEventType::UserPrompt);
        cli.transcript_path = Some("C:/different-cli.jsonl".into());
        runtime.handle_event(cli);
        let native = AgentEvent::new("codexapp:shared", AgentKind::Codex, AgentEventType::SessionStart);
        runtime.observe_codex_desktop_thread(&native, Some(SessionStatus::WaitingAnswer), None);
        let mut updates = runtime.subscribe();
        runtime.merge_provider_discovery(records.clone());
        let state = runtime.state();
        assert_eq!(state.store.sessions.len(), 2);
        let observed = state.store.sessions.iter().find(|session| session.session_id == "codexapp:shared").unwrap();
        assert_eq!(observed.status, SessionStatus::WaitingAnswer);
        assert_eq!(observed.model.as_deref(), Some("state-model"));
        assert!(std::iter::from_fn(|| updates.try_recv().ok()).all(|update| {
            !matches!(update, crate::RuntimeUpdate::Attention { .. })
        }));

        let mut replay = AgentEvent::new("shared", AgentKind::Codex, AgentEventType::Message);
        replay.transcript_path = Some(path.to_string_lossy().into_owned());
        assert_eq!(runtime.handle_transcript_event(replay).session_id, "codexapp:shared");
        runtime.close_codex_desktop_thread("codexapp:shared");
        runtime.set_codex_desktop_running(false);
        runtime.set_codex_desktop_running(true);
        runtime.merge_provider_discovery(records.clone());
        assert_eq!(runtime.state().store.sessions.len(), 1);
        assert_eq!(runtime.state().store.sessions[0].session_id, "shared");

        let mut resumed = records[0].clone();
        resumed.modified_at = Utc::now() + TimeDelta::seconds(1);
        resumed.status = SessionStatus::Idle;
        runtime.merge_provider_discovery(vec![resumed.clone()]);
        assert_eq!(runtime.state().store.sessions.len(), 1);
        resumed.status = SessionStatus::Running;
        runtime.merge_provider_discovery(vec![resumed]);
        assert_eq!(runtime.state().store.sessions.len(), 2);
    }

    #[test]
    fn claude_requires_matching_pid_creation_and_transcript() {
        let fixture = Fixture::new();
        let now = Utc::now();
        let started = now - TimeDelta::minutes(2);
        let project = fixture.0.join("projects").join("C--work");
        let records = fixture.0.join("sessions");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&records).unwrap();
        for (id, pid, seconds) in [("live", 31, 0), ("recycled", 32, 5)] {
            fs::write(
                records.join(format!("{pid}.json")),
                json!({
                    "pid": pid, "sessionId": id, "cwd": "C:/work", "procStart":
                    (started - TimeDelta::seconds(seconds)).format("%a %b %e %T %Y").to_string(),
                    "name":"Recorded Claude",
                }).to_string(),
            ).unwrap();
            fs::write(
                project.join(format!("{id}.jsonl")),
                format!("{}\n", json!({
                    "type":"user","sessionId":id,"cwd":"C:\\work","message":{"content":"Test discovery"}
                })),
            ).unwrap();
        }
        let found = read_claude_sessions(&fixture.0, now, |_| Some(started));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_id, "live");
        assert_eq!(found[0].pid, Some(31));
        assert_eq!(found[0].started_at, Some(started));
        assert_eq!(found[0].status, SessionStatus::Running);
        assert_eq!(found[0].last_prompt.as_deref(), Some("Test discovery"));
        assert!(read_claude_sessions(&fixture.0, now + TimeDelta::minutes(6), |_| Some(started)).is_empty());
    }

    #[test]
    fn discovers_native_claude_code_with_verified_process_and_desktop_route() {
        let fixture = Fixture::new();
        let now = DateTime::from_timestamp_millis(1_790_000_100_500).unwrap();
        let started = now - TimeDelta::minutes(1);
        let ticks = (started.timestamp_millis() as u64 + 11_644_473_600_000) * 10_000 + 250;
        let status_at = now - TimeDelta::seconds(1);
        let host_id = "local_4321acbd-1111-4222-8333-998877665544";
        let project = fixture.0.join("projects").join("C--work");
        let records = fixture.0.join("sessions");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&records).unwrap();
        for (pid, id, domain, proc_start, host, status) in [
            (41, "busy", "win32:test", ticks, host_id, "busy"),
            (42, "resting", "win32:test", ticks, "local_invalid&route", "idle"),
            (43, "recycled", "win32:test", ticks - 10_000, host_id, "busy"),
            (44, "other-os", "linux:machine:pid:[4026531836]", ticks, host_id, "busy"),
        ] {
            fs::write(records.join(format!("{pid}.json")), json!({
                "pid": pid,
                "pidDomain": domain,
                "procStart": proc_start.to_string(),
                "sessionId": id,
                "hostSessionId": host,
                "entrypoint": "claude-desktop-3p",
                "cwd": "C:/work",
                "status": status,
                "statusUpdatedAt": status_at.timestamp_millis(),
            }).to_string()).unwrap();
            let mut transcript = File::create(project.join(format!("{id}.jsonl"))).unwrap();
            writeln!(transcript, "{}", json!({
                "type": "user", "sessionId": id, "cwd": "C:\\work",
                "message": { "content": "Test discovery" },
            })).unwrap();
            transcript.set_times(FileTimes::new().set_modified((now - TimeDelta::seconds(10)).into())).unwrap();
        }

        let records = read_claude_sessions(&fixture.0, now, |pid| {
            assert_ne!(pid, 44, "a WSL PID must never be queried on the Windows host");
            Some(started)
        });
        assert_eq!(records.len(), 2);
        let runtime = crate::runtime::tests::runtime_with(AppSettings::default());
        runtime.merge_provider_discovery(records);
        let state = runtime.state();
        let busy = state.store.sessions.iter().find(|session| session.session_id == "busy").unwrap();
        assert_eq!(busy.provider_session_id.as_deref(), Some("busy"));
        assert_eq!(busy.claude_desktop_session_id.as_deref(), Some(host_id));
        assert_eq!(busy.terminal_kind.as_deref(), Some("claude"));
        assert_eq!(busy.status, SessionStatus::Running);
        assert_eq!(busy.pid, Some(41));
        assert_eq!(busy.last_activity, status_at);
        let resting = state.store.sessions.iter().find(|session| session.session_id == "resting").unwrap();
        assert_eq!(resting.status, SessionStatus::Idle);
        assert_eq!(resting.claude_desktop_session_id, None);

        let mut hooked = AgentEvent::new("hooked", AgentKind::Claude, AgentEventType::SessionStart);
        hooked.payload = Some(json!({ "_claude_desktop_session": host_id }));
        runtime.handle_event(hooked);
        let state = runtime.state();
        let hooked = state.store.sessions.iter().find(|session| session.session_id == "hooked").unwrap();
        assert_eq!(hooked.claude_desktop_session_id.as_deref(), Some(host_id));
        assert_eq!(hooked.terminal_kind.as_deref(), Some("claude"));
    }
}
