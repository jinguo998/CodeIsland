//! Best-effort local Claude usage scanner.
//!
//! Reads `%USERPROFILE%/.claude/projects/**/*.jsonl` (or `$CLAUDE_CONFIG_DIR`)
//! and sums token usage over the last five hours and local calendar day.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

const SPARKLINE_HOURS: usize = 12;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub message_count: u64,
}

impl UsageTotals {
    fn add(&mut self, other: Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_creation_tokens = self.cache_creation_tokens.saturating_add(other.cache_creation_tokens);
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(other.cache_read_tokens);
        self.message_count = self.message_count.saturating_add(other.message_count);
    }

    fn total(self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
            .saturating_add(self.cache_creation_tokens)
            .saturating_add(self.cache_read_tokens)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    /// Plan name if a local config names one (e.g. "max").
    pub plan: Option<String>,
    /// Total tokens observed for today (input + output + cache).
    pub used_today: u64,
    /// Best-effort daily budget; `None` because the plan limit is not local.
    pub limit: Option<u64>,
    #[serde(default)]
    pub last_5h: UsageTotals,
    #[serde(default)]
    pub today: UsageTotals,
    #[serde(default)]
    pub hourly_output_tokens: Vec<u64>,
}

#[derive(Default)]
struct FileUsage {
    consumed_bytes: u64,
    seen_messages: HashSet<String>,
    entries: Vec<(DateTime<Utc>, UsageTotals)>,
}

type UsageCache = HashMap<PathBuf, FileUsage>;

#[derive(Default)]
struct ScanState {
    files: UsageCache,
    last_scan: Option<(Vec<PathBuf>, DateTime<Utc>, Option<QuotaSnapshot>)>,
}

static CACHE: LazyLock<Mutex<ScanState>> = LazyLock::new(|| Mutex::new(ScanState::default()));

/// Scans the default Claude config directory. Returns `None` when the
/// directory is absent or cannot be read.
pub fn scan() -> Option<QuotaSnapshot> {
    let home = claude_home()?;
    scan_in(&home, Utc::now())
}

/// Testable variant: scan `claude_home` as of `now`.
pub fn scan_in(claude_home: &Path, now: DateTime<Utc>) -> Option<QuotaSnapshot> {
    scan_roots(&[claude_home.to_path_buf()], now)
}

pub fn scan_roots(roots: &[PathBuf], now: DateTime<Utc>) -> Option<QuotaSnapshot> {
    scan_with_cache(roots, now, &mut UsageCache::new())
}

/// Retain each transcript's consumed offset while the island is open.
pub fn scan_roots_cached(roots: &[PathBuf], now: DateTime<Utc>) -> Option<QuotaSnapshot> {
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached_roots, scanned_at, snapshot)) = &cache.last_scan
        && cached_roots == roots
        && now >= *scanned_at
        && now - *scanned_at < chrono::Duration::seconds(120)
    {
        return snapshot.clone();
    }
    let snapshot = scan_with_cache(roots, now, &mut cache.files);
    cache.last_scan = Some((roots.to_vec(), now, snapshot.clone()));
    snapshot
}

fn scan_with_cache(roots: &[PathBuf], now: DateTime<Utc>, cache: &mut UsageCache) -> Option<QuotaSnapshot> {
    let midnight = now.with_timezone(&Local).date_naive().and_hms_opt(0, 0, 0)
        .and_then(|date| Local.from_local_datetime(&date).earliest())
        .map(|date| date.with_timezone(&Utc))
        .unwrap_or(now - chrono::Duration::days(1));
    let five_hours_ago = now - chrono::Duration::hours(5);
    let cutoff = midnight.min(now - chrono::Duration::hours(SPARKLINE_HOURS as i64));
    let mut last_5h = UsageTotals::default();
    let mut today = UsageTotals::default();
    let mut hourly_output_tokens = vec![0u64; SPARKLINE_HOURS];
    let mut active_files = HashSet::new();
    let mut scanned_dirs = HashSet::new();
    for claude_home in roots {
        let Ok(projects) = claude_home.join("projects").canonicalize() else {
            continue;
        };
        if !scanned_dirs.insert(projects.clone()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&projects) else { continue };
        for project in entries.flatten() {
            let Ok(files) = std::fs::read_dir(project.path()) else { continue };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(metadata) = std::fs::metadata(&path) else { continue };
                if metadata.modified().is_ok_and(|modified| DateTime::<Utc>::from(modified) < cutoff) {
                    continue;
                }
                let usage = cache.entry(path.clone()).or_default();
                if metadata.len() < usage.consumed_bytes {
                    *usage = FileUsage::default();
                }
                if metadata.len() > usage.consumed_bytes && consume_file(&path, usage).is_err() {
                    continue;
                }
                active_files.insert(path);
                usage.entries.retain(|(timestamp, _)| *timestamp >= cutoff);
                for &(timestamp, totals) in &usage.entries {
                    if timestamp > now {
                        continue;
                    }
                    if timestamp >= five_hours_ago {
                        last_5h.add(totals);
                    }
                    if timestamp >= midnight {
                        today.add(totals);
                    }
                    let age = (now - timestamp).num_hours() as usize;
                    if age < SPARKLINE_HOURS {
                        let bucket = &mut hourly_output_tokens[SPARKLINE_HOURS - 1 - age];
                        *bucket = bucket.saturating_add(totals.output_tokens);
                    }
                }
            }
        }
    }
    cache.retain(|path, _| active_files.contains(path));
    if active_files.is_empty() {
        return None;
    }

    Some(QuotaSnapshot {
        plan: roots.iter().find_map(|root| read_plan(root)),
        used_today: today.total(),
        limit: None,
        last_5h,
        today,
        hourly_output_tokens,
    })
}

fn consume_file(path: &Path, usage: &mut FileUsage) -> std::io::Result<()> {
    let mut file = BufReader::new(std::fs::File::open(path)?);
    file.seek(SeekFrom::Start(usage.consumed_bytes))?;
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let count = file.read_until(b'\n', &mut bytes)?;
        if bytes.last() != Some(&b'\n') {
            break;
        }
        usage.consumed_bytes += count as u64;
        let Ok(line) = std::str::from_utf8(&bytes) else { continue };
        if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|kind| kind.as_str()) != Some("assistant") {
            continue;
        }
        let message = &value["message"];
        let (Some(timestamp), Some(id), Some(fields)) = (
            value["timestamp"].as_str().and_then(parse_timestamp),
            message["id"].as_str(),
            message["usage"].as_object(),
        ) else {
            continue;
        };
        if usage.seen_messages.insert(id.to_owned()) {
            let field = |key: &str| fields.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0);
            usage.entries.push((timestamp, UsageTotals {
                input_tokens: field("input_tokens"),
                output_tokens: field("output_tokens"),
                cache_creation_tokens: field("cache_creation_input_tokens"),
                cache_read_tokens: field("cache_read_input_tokens"),
                message_count: 1,
            }));
        }
    }
    Ok(())
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

/// Claude Code's config directory: `$CLAUDE_CONFIG_DIR`, else
/// `%USERPROFILE%\.claude` on Windows, else `$HOME/.claude`.
pub fn claude_home() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        let path = PathBuf::from(raw);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    Some(home.join(".claude"))
}

/// Look for a plan label in the local account config. Never fails the scan.
fn read_plan(claude_home: &Path) -> Option<String> {
    let mut candidates = vec![claude_home.join(".claude.json")];
    if let Some(parent) = claude_home.parent() {
        candidates.push(parent.join(".claude.json"));
    }
    for path in candidates {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
            continue;
        };
        for key in ["plan", "subscriptionType", "subscription_type", "subscription"] {
            if let Some(plan) = value.get(key).and_then(|plan| plan.as_str()) {
                if !plan.trim().is_empty() {
                    return Some(plan.trim().to_owned());
                }
            }
        }
        if let Some(plan) = value
            .get("account")
            .and_then(|account| account.get("plan"))
            .and_then(|plan| plan.as_str())
        {
            if !plan.trim().is_empty() {
                return Some(plan.trim().to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = format!(
            "codeisland-quota-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_transcript(root: &Path, name: &str, lines: &[String]) {
        let project = root.join(".claude").join("projects").join("demo");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join(name), lines.join("\n")).unwrap();
    }

    fn assistant_line(id: &str, timestamp: &str, input: u64, output: u64, cache_read: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{timestamp}","message":{{"id":"{id}","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_creation_input_tokens":0,"cache_read_input_tokens":{cache_read}}}}}}}"#
        )
    }

    #[test]
    fn sums_today_and_dedupes_message_ids() {
        let dir = temp_dir("sum");
        let now = Utc::now();
        // Build timestamps in the same instant as the scan so the calendar-day
        // comparison is stable regardless of the test machine's timezone.
        let today = now.with_timezone(&Local).to_rfc3339();
        let yesterday = (now - chrono::Duration::days(2))
            .with_timezone(&Local)
            .to_rfc3339();
        write_transcript(
            &dir,
            "session.jsonl",
            &[
                assistant_line("msg-1", &today, 100, 50, 25),
                // Same API response continuation: must not double count.
                assistant_line("msg-1", &today, 100, 50, 25),
                assistant_line("msg-2", &today, 10, 5, 0),
                assistant_line("msg-old", &yesterday, 1000, 1000, 0),
                r#"{"type":"user","message":{"content":"hi"}}"#.to_owned(),
                "not json at all".to_owned(),
            ],
        );
        let snapshot = scan_in(&dir.join(".claude"), now).expect("snapshot");
        assert_eq!(snapshot.used_today, 100 + 50 + 25 + 10 + 5);
        assert!(snapshot.limit.is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_directory_returns_none() {
        let dir = temp_dir("missing");
        assert!(scan_in(&dir.join("does-not-exist"), Utc::now()).is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn empty_projects_directory_returns_none() {
        let dir = temp_dir("empty");
        fs::create_dir_all(dir.join(".claude").join("projects")).unwrap();
        assert!(scan_in(&dir.join(".claude"), Utc::now()).is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reads_plan_label_when_present() {
        let dir = temp_dir("plan");
        let now = Utc::now();
        write_transcript(
            &dir,
            "session.jsonl",
            &[assistant_line(
                "msg-1",
                &now.with_timezone(&Local).to_rfc3339(),
                1,
                2,
                3,
            )],
        );
        fs::write(
            dir.join(".claude").join(".claude.json"),
            r#"{"subscriptionType":"max"}"#,
        )
        .unwrap();
        let snapshot = scan_in(&dir.join(".claude"), now).unwrap();
        assert_eq!(snapshot.plan.as_deref(), Some("max"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn incremental_scan_waits_for_a_complete_line_and_keeps_hour_buckets() {
        use std::io::Write;

        let root = temp_dir("incremental");
        let now = Utc::now();
        let within_five_hours = (now - chrono::Duration::hours(2)).to_rfc3339();
        let lines = [
            assistant_line("first", &within_five_hours, 10, 20, 30),
            assistant_line("second", &within_five_hours, 40, 50, 60),
        ];
        write_transcript(&root, "session.jsonl", &lines);
        let claude_home = root.join(".claude");
        let roots = [claude_home.clone()];
        let mut cache = UsageCache::new();
        let snapshot = scan_with_cache(&roots, now, &mut cache).unwrap();
        assert_eq!(snapshot.last_5h.message_count, 1);
        assert_eq!(snapshot.hourly_output_tokens[9], 20);

        let path = claude_home.join("projects/demo/session.jsonl");
        fs::OpenOptions::new().append(true).open(path).unwrap()
            .write_all(format!("\n{}\n", lines[0]).as_bytes()).unwrap();
        let snapshot = scan_with_cache(&roots, now, &mut cache).unwrap();
        assert_eq!(snapshot.last_5h.message_count, 2);
        assert_eq!(snapshot.last_5h.cache_read_tokens, 90);
        assert_eq!(snapshot.hourly_output_tokens[9], 70);
        fs::remove_dir_all(root).ok();
    }
}
