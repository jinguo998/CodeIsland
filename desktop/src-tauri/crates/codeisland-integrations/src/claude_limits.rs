//! Claude Code subscription windows and event-driven refresh policy.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Session,
    WeeklyAll,
    WeeklyScoped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Limit {
    pub kind: Kind,
    pub percent: f64,
    pub severity: String,
    pub resets_at: Option<DateTime<Utc>>,
    pub scope_label: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub limits: Vec<Limit>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchError {
    NoCredential,
    Unauthorized,
    RateLimited,
    Http(u16),
    Transport,
    Parse,
}

impl FetchError {
    pub fn needs_login(&self) -> bool {
        matches!(self, Self::NoCredential | Self::Unauthorized)
    }
}

// The Swift client receives the configured directory. OAuth credentials stay
// inside the native client and never cross the desktop bridge.
#[derive(Deserialize)]
struct Row {
    kind: Kind,
    percent: f64,
    severity: String,
    #[serde(default, with = "chrono::serde::ts_microseconds_option")]
    resets_at: Option<DateTime<Utc>>,
    scope_label: Option<String>,
}

impl From<Row> for Limit {
    fn from(row: Row) -> Self {
        Self { kind: row.kind, percent: row.percent, severity: row.severity,
               resets_at: row.resets_at, scope_label: row.scope_label }
    }
}

pub async fn fetch(claude_dir: &Path) -> Result<Snapshot, FetchError> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Response {
        Success {
            limits: Vec<Row>,
            #[serde(with = "chrono::serde::ts_microseconds")]
            fetched_at: DateTime<Utc>,
        },
        Failure { error: String, status: Option<u16> },
    }
    let directory = claude_dir.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        codeisland_swift::call::<Response>(c"fetchQuota", directory.to_string_lossy())
    }).await.map_err(|_| FetchError::Transport)?;
    match result {
        Response::Success { limits, fetched_at } => Ok(Snapshot {
            limits: limits.into_iter().map(Limit::from).collect(), fetched_at,
        }),
        Response::Failure { error, status } => Err(match error.as_str() {
            "no_credential" => FetchError::NoCredential,
            "unauthorized" => FetchError::Unauthorized,
            "rate_limited" => FetchError::RateLimited,
            "http" => FetchError::Http(status.unwrap_or(0)),
            "parse" => FetchError::Parse,
            _ => FetchError::Transport,
        }),
    }
}

#[cfg(test)]
fn parse_snapshot(value: &Value, fetched_at: DateTime<Utc>) -> Option<Snapshot> {
    let rows: Vec<Row> = codeisland_swift::call::<Option<Vec<Row>>>(c"parseQuota", value)?;
    Some(Snapshot { limits: rows.into_iter().map(Limit::from).collect(), fetched_at })
}

#[derive(Default)]
pub struct Scheduler {
    swift: codeisland_swift::Context,
    origin: std::sync::OnceLock<Instant>,
}

impl Scheduler {
    fn seconds(&self, now: Instant) -> f64 {
        let origin = *self.origin.get_or_init(|| now);
        if now >= origin { (now - origin).as_secs_f64() } else { -(origin - now).as_secs_f64() }
    }

    pub fn record_stop(&mut self, now: Instant) { self.swift.call(c"quota.stop", self.seconds(now)) }
    pub fn record_start(&mut self, now: Instant) { self.swift.call(c"quota.start", self.seconds(now)) }
    pub fn record_result(&mut self, error: Option<&FetchError>) {
        let operation = match error {
            None => c"quota.success",
            Some(error) if error.needs_login() => c"quota.unauthorized",
            Some(_) => c"quota.failure",
        };
        self.swift.call(operation, 0)
    }

    pub fn next_fire(&self, now: Instant) -> Option<Instant> {
        let seconds = self.swift.call::<Option<f64>>(c"quota.next", self.seconds(now))?;
        let origin = *self.origin.get()?;
        let offset = Duration::from_secs_f64(seconds.abs());
        if seconds >= 0.0 { origin.checked_add(offset) } else { origin.checked_sub(offset) }
    }
    pub fn due_on_expand(&self, now: Instant) -> bool {
        self.swift.call(c"quota.expand", self.seconds(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn shared_client_reports_a_missing_credential() {
        let directory = std::env::temp_dir().join(format!("codeisland-missing-quota-{}", std::process::id()));
        assert!(matches!(fetch(&directory).await, Err(FetchError::NoCredential)));
    }

    #[test]
    fn supports_both_usage_responses_and_takes_the_first_window_of_each_kind() {
        let value = json!({"limits": [
            {"kind": "future", "percent": 5},
            {"kind": "weekly_scoped", "percent": "112", "scope": {"model": {"display_name": "Fable"}}},
            {"kind": "weekly_scoped", "percent": 6}
        ], "five_hour": {"utilization": 5}});
        let snapshot = parse_snapshot(&value, Utc::now()).unwrap();
        assert_eq!(snapshot.limits.len(), 1);
        assert_eq!(snapshot.limits[0].percent, 112.);
        assert_eq!(snapshot.limits[0].scope_label.as_deref(), Some("Fable"));

        let value = json!({"five_hour": {"utilization": 17, "resets_at": "2026-11-11T12:00:00Z"},
            "seven_day_opus": {"utilization": 5}, "seven_day_sonnet": {"utilization": 7}});
        let snapshot = parse_snapshot(&value, Utc::now()).unwrap();
        assert_eq!(snapshot.limits.len(), 2);
        assert!(snapshot.limits[0].resets_at.is_some());
        assert_eq!(snapshot.limits[1].scope_label.as_deref(), Some("Opus"));
    }

    #[test]
    fn debounces_stops_throttles_and_waits_for_expansion_after_unauthorized() {
        let now = Instant::now();
        let after = |seconds| now + Duration::from_secs(seconds);
        let mut scheduler = Scheduler::default();
        assert_eq!(scheduler.next_fire(now), Some(now));
        scheduler.record_start(now);
        assert_eq!(scheduler.next_fire(now), Some(after(600)));
        scheduler.record_stop(after(30));
        assert_eq!(scheduler.next_fire(now), Some(after(60)));
        scheduler.record_stop(after(50));
        assert_eq!(scheduler.next_fire(now), Some(after(65)));
        scheduler.record_result(Some(&FetchError::RateLimited));
        scheduler.record_result(Some(&FetchError::RateLimited));
        assert_eq!(scheduler.next_fire(now), Some(after(120)));
        scheduler.record_result(Some(&FetchError::Unauthorized));
        assert_eq!(scheduler.next_fire(now), None);
        assert!(!scheduler.due_on_expand(after(59)));
        assert!(scheduler.due_on_expand(after(60)));
    }
}
