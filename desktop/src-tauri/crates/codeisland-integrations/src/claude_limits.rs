//! Claude Code subscription windows and event-driven refresh policy.

use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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

static CLIENT: LazyLock<Result<reqwest::Client, reqwest::Error>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
});

pub async fn fetch(claude_dir: &Path) -> Result<Snapshot, FetchError> {
    let path = claude_dir.join(".credentials.json");
    let (token, expires_at) = tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(path).ok()?;
        parse_credential(&bytes)
    })
    .await
    .ok()
    .flatten()
    .ok_or(FetchError::NoCredential)?;
    if expires_at.is_some_and(|time| time <= Utc::now()) {
        return Err(FetchError::Unauthorized);
    }

    let response = CLIENT
        .as_ref()
        .map_err(|_| FetchError::Transport)?
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("accept", "application/json")
        .header("user-agent", "CodeIsland")
        .send()
        .await
        .map_err(|_| FetchError::Transport)?;
    match response.status().as_u16() {
        200..=299 => {
            let value = response.json().await.map_err(|_| FetchError::Parse)?;
            parse_snapshot(&value, Utc::now()).ok_or(FetchError::Parse)
        }
        401 | 403 => Err(FetchError::Unauthorized),
        429 => Err(FetchError::RateLimited),
        status => Err(FetchError::Http(status)),
    }
}

fn parse_credential(bytes: &[u8]) -> Option<(String, Option<DateTime<Utc>>)> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let oauth = &value["claudeAiOauth"];
    let token = oauth["accessToken"]
        .as_str()
        .filter(|token| !token.is_empty())?;
    let expires_at = oauth["expiresAt"]
        .as_f64()
        .and_then(|millis| DateTime::from_timestamp_millis(millis as i64));
    Some((token.to_owned(), expires_at))
}

fn parse_snapshot(value: &Value, fetched_at: DateTime<Utc>) -> Option<Snapshot> {
    #[derive(Deserialize)]
    struct Row {
        kind: Kind, percent: f64, severity: String,
        #[serde(default, with = "chrono::serde::ts_microseconds_option")]
        resets_at: Option<DateTime<Utc>>,
        scope_label: Option<String>,
    }
    let rows: Vec<Row> = codeisland_swift::call::<Option<Vec<Row>>>(c"parseQuota", value)?;
    let limits = rows.into_iter().map(|row| Limit {
        kind: row.kind, percent: row.percent, severity: row.severity,
        resets_at: row.resets_at, scope_label: row.scope_label,
    }).collect();
    Some(Snapshot { limits, fetched_at })
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
