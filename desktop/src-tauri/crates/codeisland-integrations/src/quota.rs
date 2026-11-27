//! Best-effort local Claude usage scanner.
//!
//! Reads `%USERPROFILE%/.claude/projects/**/*.jsonl` (or `$CLAUDE_CONFIG_DIR`)
//! and sums token usage over the last five hours and local calendar day.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub message_count: u64,
}

impl UsageTotals {
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
struct ScanState {
    files: codeisland_swift::Context,
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
    scan_with_cache(roots, now, &codeisland_swift::Context::default())
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
    let snapshot = scan_with_cache(roots, now, &cache.files);
    cache.last_scan = Some((roots.to_vec(), now, snapshot.clone()));
    snapshot
}

fn scan_with_cache(roots: &[PathBuf], now: DateTime<Utc>, cache: &codeisland_swift::Context) -> Option<QuotaSnapshot> {
    #[derive(Deserialize)]
    struct Totals {
        last_5h: UsageTotals,
        today: UsageTotals,
        hourly_output_tokens: Vec<u64>,
    }
    let paths: Vec<_> = roots.iter().map(|path| path.to_string_lossy()).collect();
    let result: Totals = cache.call::<Option<Totals>>(c"scanUsage",
        serde_json::json!({ "roots": paths, "now": now.timestamp_micros() }))?;
    Some(QuotaSnapshot {
        plan: roots.iter().find_map(|root| read_plan(root)),
        used_today: result.today.total(),
        limit: None,
        last_5h: result.last_5h,
        today: result.today,
        hourly_output_tokens: result.hourly_output_tokens,
    })
}

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
    use chrono::Local;
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
        let mut cache = codeisland_swift::Context::default();
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
