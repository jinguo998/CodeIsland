//! Claude Code subscription windows and event-driven refresh policy.

use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Session,
    WeeklyAll,
    WeeklyScoped,
}

#[derive(Clone, Debug, Serialize)]
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
    let mut limits = Vec::with_capacity(3);
    if let Some(items) = value["limits"].as_array() {
        for item in items {
            let kind = match item["kind"].as_str() {
                Some("session") => Kind::Session,
                Some("weekly_all") => Kind::WeeklyAll,
                Some("weekly_scoped") => Kind::WeeklyScoped,
                _ => continue,
            };
            if limits.iter().any(|limit: &Limit| limit.kind == kind) {
                continue;
            }
            let model = &item["scope"]["model"];
            if let Some(limit) = parse_limit(
                item,
                kind,
                "percent",
                model["display_name"]
                    .as_str()
                    .or_else(|| model["id"].as_str()),
            ) {
                limits.push(limit);
            }
        }
    }
    if limits.is_empty() {
        for (key, kind, label) in [
            ("five_hour", Kind::Session, None),
            ("seven_day", Kind::WeeklyAll, None),
            ("seven_day_opus", Kind::WeeklyScoped, Some("Opus")),
            ("seven_day_sonnet", Kind::WeeklyScoped, Some("Sonnet")),
        ] {
            if !limits.iter().any(|limit| limit.kind == kind)
                && let Some(limit) = parse_limit(&value[key], kind, "utilization", label)
            {
                limits.push(limit);
            }
        }
    }
    (!limits.is_empty()).then_some(Snapshot { limits, fetched_at })
}

fn parse_limit(item: &Value, kind: Kind, field: &str, scope_label: Option<&str>) -> Option<Limit> {
    let percent = item[field]
        .as_f64()
        .or_else(|| item[field].as_str()?.parse().ok())
        .filter(|percent| percent.is_finite())?;
    Some(Limit {
        kind,
        percent,
        severity: item["severity"].as_str().unwrap_or("normal").to_owned(),
        resets_at: item["resets_at"]
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|date| date.with_timezone(&Utc)),
        scope_label: scope_label.map(str::to_owned),
    })
}

#[derive(Default)]
pub struct Scheduler {
    last_fetch: Option<Instant>,
    last_pending_stop: Option<Instant>,
    failures: u32,
    needs_login: bool,
}

impl Scheduler {
    pub fn record_stop(&mut self, now: Instant) {
        self.last_pending_stop = Some(now);
    }

    pub fn record_start(&mut self, now: Instant) {
        self.last_fetch = Some(now);
        self.last_pending_stop = None;
    }

    pub fn record_result(&mut self, error: Option<&FetchError>) {
        match error {
            None => {
                self.failures = 0;
                self.needs_login = false;
            }
            Some(error) if error.needs_login() => {
                self.failures = 0;
                self.needs_login = true;
            }
            Some(_) => self.failures = self.failures.saturating_add(1),
        }
    }

    fn backoff(&self) -> Option<Duration> {
        (self.failures > 0)
            .then(|| Duration::from_secs((60 << (self.failures - 1).min(10)).min(900)))
    }

    pub fn next_fire(&self, now: Instant) -> Option<Instant> {
        if self.needs_login {
            return None;
        }
        let candidate = match (self.last_pending_stop, self.last_fetch) {
            (Some(stop), Some(fetch)) => {
                (stop + Duration::from_secs(15)).max(fetch + Duration::from_secs(60))
            }
            (Some(stop), None) => stop + Duration::from_secs(15),
            (None, Some(fetch)) => fetch + Duration::from_secs(600),
            (None, None) => now,
        };
        Some(match (self.last_fetch, self.backoff()) {
            (Some(fetch), Some(backoff)) => candidate.max(fetch + backoff),
            _ => candidate,
        })
    }

    pub fn due_on_expand(&self, now: Instant) -> bool {
        self.last_fetch.is_none_or(|fetch| {
            now.duration_since(fetch) >= self.backoff().unwrap_or(Duration::from_secs(60))
        })
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
