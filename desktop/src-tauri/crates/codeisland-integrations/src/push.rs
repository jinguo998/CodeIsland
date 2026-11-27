//! Push notification channels.
//!
//! Every channel is reduced to a [`PushRequest`] (method, URL, headers, body)
//! before any network I/O happens, so the wire shapes are unit-testable without
//! a server. [`PushNotifier::send`] is the only function that touches reqwest.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use codeisland_domain::{PushChannelSetting, PushKind};
use reqwest::Url;
use serde::{Deserialize, Serialize};

/// Delivery services understood by [`PushNotifier`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushChannel {
    Bark,
    Ntfy,
    Telegram,
    DingTalk,
    WeCom,
    Lark,
    Slack,
    Discord,
    /// Anything that accepts a JSON `{kind,title,body}` POST.
    Webhook,
}

impl PushChannel {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "bark" => Some(Self::Bark),
            "ntfy" => Some(Self::Ntfy),
            "telegram" => Some(Self::Telegram),
            "dingtalk" => Some(Self::DingTalk),
            "wecom" => Some(Self::WeCom),
            "lark" | "feishu" => Some(Self::Lark),
            "slack" => Some(Self::Slack),
            "discord" => Some(Self::Discord),
            "webhook" | "generic" => Some(Self::Webhook),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bark => "bark",
            Self::Ntfy => "ntfy",
            Self::Telegram => "telegram",
            Self::DingTalk => "dingtalk",
            Self::WeCom => "wecom",
            Self::Lark => "lark",
            Self::Slack => "slack",
            Self::Discord => "discord",
            Self::Webhook => "webhook",
        }
    }

    pub fn rate_limits(self) -> &'static [(usize, u64)] {
        match self {
            Self::DingTalk | Self::WeCom => &[(18, 60)],
            Self::Lark => &[(90, 60), (4, 1)],
            Self::Slack => &[(1, 1)],
            _ => &[],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("push notifications are disabled")]
    Disabled,
    #[error("invalid push target: {0}")]
    InvalidTarget(String),
    #[error("push request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("push service rejected the request with HTTP {0}")]
    Rejected(u16),
    #[error("the push service rejected the message")]
    ServiceRejected,
    #[error("the push service redirected to an address that cannot be followed")]
    RedirectRefused,
    #[error("the push service did not answer in time")]
    Timeout,
    #[error("push channel is not supported in this build")]
    Unsupported,
}

/// A fully rendered HTTP request. Kept public so tests can assert the exact
/// URL/body without making a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl PushRequest {
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn body_json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }

    pub(crate) fn build(
        channel: PushChannel,
        target: &str,
        kind: PushKind,
        title: &str,
        body: &str,
    ) -> Result<Self, PushError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(PushError::InvalidTarget("target is empty".into()));
        }
        match channel {
            PushChannel::Bark => Self::bark(target, title, body),
            PushChannel::Ntfy => Self::ntfy(target, kind, title, body),
            PushChannel::Telegram => Self::telegram(target, title, body),
            PushChannel::DingTalk | PushChannel::WeCom => {
                let config = PushChannelSetting { endpoint: target.to_owned(), include_details: true, ..Default::default() };
                crate::push_channels::build(channel, &config, kind, title, "", body, chrono::Utc::now())
            }
            PushChannel::Lark => Self::lark(target, title, body),
            PushChannel::Slack => Self::slack(target, title, body),
            PushChannel::Discord => Self::discord(target, title, body),
            PushChannel::Webhook => Self::webhook(target, kind, title, body),
        }
    }

    /// Bark's path API: `{base}/{device}/{title}/{body}`. When the target is a
    /// full URL (`https://api.day.app/<device>`) it is used as base+device;
    /// otherwise the hosted `https://api.day.app` base is assumed.
    fn bark(target: &str, title: &str, body: &str) -> Result<Self, PushError> {
        let prefix = if target.starts_with("http://") || target.starts_with("https://") {
            target.trim_end_matches('/').to_owned()
        } else {
            format!("https://api.day.app/{}", encode_path(target))
        };
        let url = format!(
            "{prefix}/{}/{}",
            encode_path(title),
            encode_path(body)
        );
        Ok(Self {
            method: "GET",
            url,
            headers: Vec::new(),
            body: Vec::new(),
        })
    }

    /// ntfy publishes the raw request body to `{base}/{topic}`; `Title` and
    /// `Priority` ride in headers so the body stays human-readable.
    fn ntfy(
        target: &str,
        kind: PushKind,
        title: &str,
        body: &str,
    ) -> Result<Self, PushError> {
        let url = if target.starts_with("http://") || target.starts_with("https://") {
            target.trim_end_matches('/').to_owned()
        } else {
            format!("https://ntfy.sh/{}", encode_path(target))
        };
        let mut headers = vec![(
            "Content-Type".to_owned(),
            "text/plain; charset=utf-8".to_owned(),
        )];
        if !title.trim().is_empty() {
            headers.push(("Title".to_owned(), title.to_owned()));
        }
        if blocks_agent(kind) {
            headers.push(("Priority".to_owned(), "high".to_owned()));
            headers.push(("Tags".to_owned(), "warning".to_owned()));
        }
        let text = if title.trim().is_empty() {
            body.to_owned()
        } else if body.trim().is_empty() {
            title.to_owned()
        } else {
            format!("{title}\n{body}")
        };
        Ok(Self {
            method: "POST",
            url,
            headers,
            body: text.into_bytes(),
        })
    }

    /// Telegram target is `token/chat_id` (a bot token never contains `/`).
    fn telegram(target: &str, title: &str, body: &str) -> Result<Self, PushError> {
        let (token, chat_id) = target
            .rsplit_once('/')
            .ok_or_else(|| PushError::InvalidTarget("expected <bot-token>/<chat-id>".into()))?;
        let token = token.trim().trim_start_matches("bot");
        let chat_id = chat_id.trim();
        if token.is_empty() || chat_id.is_empty() {
            return Err(PushError::InvalidTarget(
                "expected <bot-token>/<chat-id>".into(),
            ));
        }
        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
        let text = join_title_body(title, body);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        Ok(Self::json_post(url, payload))
    }

    fn lark(target: &str, title: &str, body: &str) -> Result<Self, PushError> {
        let url = require_url(target)?;
        let payload = serde_json::json!({
            "msg_type": "text",
            "content": { "text": join_title_body(title, body) },
        });
        Ok(Self::json_post(url, payload))
    }

    fn slack(target: &str, title: &str, body: &str) -> Result<Self, PushError> {
        let url = require_url(target)?;
        let payload = serde_json::json!({ "text": join_title_body(title, body) });
        Ok(Self::json_post(url, payload))
    }

    fn discord(target: &str, title: &str, body: &str) -> Result<Self, PushError> {
        let url = require_url(target)?;
        // Discord rejects content over 2000 characters.
        let text = truncate_chars(&join_title_body(title, body), 2000);
        let payload = serde_json::json!({ "content": text });
        Ok(Self::json_post(url, payload))
    }

    fn webhook(
        target: &str,
        kind: PushKind,
        title: &str,
        body: &str,
    ) -> Result<Self, PushError> {
        let url = require_url(target)?;
        let payload = serde_json::json!({
            "kind": kind_name(kind),
            "title": title,
            "body": body,
        });
        Ok(Self::json_post(url, payload))
    }

    pub(crate) fn json_post(url: String, payload: serde_json::Value) -> Self {
        Self {
            method: "POST",
            url,
            headers: vec![(
                "Content-Type".to_owned(),
                "application/json; charset=utf-8".to_owned(),
            )],
            body: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

/// Configured notifier. `target` is channel-specific:
/// - Bark: device key or full `https://api.day.app/<key>` URL
/// - ntfy: topic name or full topic URL
/// - Telegram: `<bot-token>/<chat-id>`
/// - Lark / Slack / Discord / Webhook: the full webhook URL
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushNotifier {
    pub enabled: bool,
    pub channel: PushChannel,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config: Option<PushChannelSetting>,
    #[serde(default = "default_summary_length")]
    summary_length: usize,
}

fn default_summary_length() -> usize { 200 }

impl PushNotifier {
    pub fn new(channel: PushChannel, target: impl Into<String>) -> Self {
        Self {
            enabled: true,
            channel,
            target: target.into(),
            config: None,
            summary_length: default_summary_length(),
        }
    }

    pub fn configured(config: PushChannelSetting, summary_length: usize) -> Option<Self> {
        let channel = PushChannel::from_name(&config.kind)?;
        Some(Self {
            enabled: config.enabled,
            channel,
            target: config.target.clone(),
            config: Some(config),
            summary_length,
        })
    }

    pub fn accepts(&self, kind: PushKind, reminder: bool) -> bool {
        let event = if reminder {
            "reminder"
        } else {
            match kind {
                PushKind::Approval => "permission",
                PushKind::Question | PushKind::DisplayOnlyWait => "question",
                PushKind::Completion => "completion",
                PushKind::Error => "error",
            }
        };
        self.enabled && self.config.as_ref().is_none_or(|config| {
            config.events.iter().any(|kind| kind == event || (event == "error" && kind == "completion"))
        })
    }

    pub fn build_request(
        &self,
        kind: PushKind,
        title: &str,
        body: &str,
    ) -> Result<PushRequest, PushError> {
        self.build_content(kind, title, "", body)
    }

    fn build_content(
        &self,
        kind: PushKind,
        title: &str,
        headline: &str,
        body: &str,
    ) -> Result<PushRequest, PushError> {
        if !self.enabled {
            return Err(PushError::Disabled);
        }
        match self.config.as_ref() {
            Some(config) => crate::push_channels::build(
                self.channel, config, kind, title, headline,
                &crate::push_channels::truncate(body, self.summary_length), chrono::Utc::now(),
            ),
            None => PushRequest::build(self.channel, &self.target, kind, title, &join_title_body(headline, body)),
        }
    }

    /// Sends one notification. A non-2xx response is reported as
    /// [`PushError::Rejected`].
    pub async fn send(
        &self,
        kind: PushKind,
        title: &str,
        body: &str,
    ) -> Result<(), PushError> {
        self.send_for(kind, title, "", body, false).await
    }

    pub async fn send_for(
        &self,
        kind: PushKind,
        title: &str,
        headline: &str,
        body: &str,
        reminder: bool,
    ) -> Result<(), PushError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let request = self.build_content(kind, title, headline, body)?;
        let (result, retry) = exchange(&client, self.channel, &request).await;
        if !reminder && matches!(kind, PushKind::Approval | PushKind::Question | PushKind::Error)
            && let Some(delay) = retry
        {
            tokio::time::sleep(delay).await;
            // Signed chat requests get a fresh timestamp.
            let request = self.build_content(kind, title, headline, body)?;
            return exchange(&client, self.channel, &request).await.0;
        }
        result
    }

    /// Sends only when [`PushGate`] admits the event.
    pub async fn send_if_allowed(
        &self,
        gate: &mut PushGate,
        event_id: &str,
        session_id: &str,
        kind: PushKind,
        title: &str,
        body: &str,
    ) -> Result<bool, PushError> {
        if !gate.should_send(event_id, session_id) {
            return Ok(false);
        }
        self.send(kind, title, body).await?;
        Ok(true)
    }
}

async fn exchange(
    client: &reqwest::Client,
    channel: PushChannel,
    request: &PushRequest,
) -> (Result<(), PushError>, Option<Duration>) {
    let fallback_delay = Duration::from_secs(5);
    let response = tokio::time::timeout(Duration::from_secs(8), send_request(client, request)).await;
    let (status, retry_after, body) = match response {
        Err(_) => return (Err(PushError::Timeout), Some(fallback_delay)),
        Ok(Err(error @ PushError::Http(_))) => return (Err(error), Some(fallback_delay)),
        Ok(Err(error)) => return (Err(error), None),
        Ok(Ok(response)) => response,
    };
    let code = status.as_u16();
    if status.is_success() {
        let result = crate::push_channels::accepted(channel, code, &body).then_some(())
            .ok_or(PushError::ServiceRejected);
        return (result, None);
    }
    let retry = match code {
        429 => {
            let delay = retry_after.or_else(|| {
                let json = serde_json::from_slice::<serde_json::Value>(&body).ok()?;
                json["parameters"]["retry_after"].as_u64().map(Duration::from_secs)
            }).unwrap_or(fallback_delay);
            Some(delay.clamp(Duration::from_secs(1), Duration::from_secs(30)))
        }
        500..=599 => Some(fallback_delay),
        _ => None,
    };
    (Err(PushError::Rejected(code)), retry)
}

async fn send_request(
    client: &reqwest::Client,
    request: &PushRequest,
) -> Result<(reqwest::StatusCode, Option<Duration>, Vec<u8>), PushError> {
    let mut url = Url::parse(&request.url)
        .map_err(|_| PushError::InvalidTarget("expected an HTTP or HTTPS URL".into()))?;
    let method = if request.method == "GET" { reqwest::Method::GET } else { reqwest::Method::POST };
    for hop in 0..=5 {
        let mut builder = client.request(method.clone(), url.clone());
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }
        let response = builder.send().await.map_err(|error| error.without_url())?;
        let status = response.status();
        if status.is_redirection()
            && let Some(location) = response.headers().get(reqwest::header::LOCATION)
        {
            let next = location.to_str().ok().and_then(|location| url.join(location).ok())
                .filter(|next| {
                    hop < 5 && matches!(next.scheme(), "http" | "https")
                        && next.host_str() == url.host_str()
                        && (url.scheme() != "https" || next.scheme() == "https")
                        && next.username().is_empty() && next.password().is_none()
                }).ok_or(PushError::RedirectRefused)?;
            url = next;
            continue;
        }
        let retry_after = response.headers().get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()).and_then(|value| {
                value.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
                    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
                    Some((date.with_timezone(&chrono::Utc) - chrono::Utc::now()).to_std().unwrap_or_default())
                })
            });
        let body = response.bytes().await.map_err(|error| error.without_url())?.to_vec();
        return Ok((status, retry_after, body));
    }
    Err(PushError::RedirectRefused)
}

/// Deduplicates by event id and rate-limits to at most one push per session
/// every five seconds. Both checks are local; nothing is persisted.
#[derive(Debug)]
pub struct PushGate {
    seen: HashSet<String>,
    last_sent: HashMap<String, Instant>,
    window: Duration,
}

impl Default for PushGate {
    fn default() -> Self {
        Self::new()
    }
}

impl PushGate {
    pub fn new() -> Self {
        Self {
            seen: HashSet::new(),
            last_sent: HashMap::new(),
            window: Duration::from_secs(5),
        }
    }

    pub fn with_window(window: Duration) -> Self {
        Self {
            seen: HashSet::new(),
            last_sent: HashMap::new(),
            window,
        }
    }

    pub fn should_send(&mut self, event_id: &str, session_id: &str) -> bool {
        self.should_send_at(event_id, session_id, Instant::now())
    }

    /// `now` is injectable so the rate limit can be tested without sleeping.
    pub fn should_send_at(&mut self, event_id: &str, session_id: &str, now: Instant) -> bool {
        if self.seen.contains(event_id) {
            return false;
        }
        if let Some(last) = self.last_sent.get(session_id) {
            if now.saturating_duration_since(*last) < self.window {
                return false;
            }
        }
        // Bound memory on a long-running process.
        if self.seen.len() >= 4096 {
            self.seen.clear();
        }
        self.last_sent.retain(|_, sent| now.saturating_duration_since(*sent) < self.window);
        self.seen.insert(event_id.to_owned());
        self.last_sent.insert(session_id.to_owned(), now);
        true
    }

    pub fn has_seen(&self, event_id: &str) -> bool {
        self.seen.contains(event_id)
    }
}

fn require_url(target: &str) -> Result<String, PushError> {
    if target.starts_with("http://") || target.starts_with("https://") {
        Ok(target.to_owned())
    } else {
        Err(PushError::InvalidTarget(format!(
            "expected an http(s) webhook URL, got {target}"
        )))
    }
}

fn blocks_agent(kind: PushKind) -> bool {
    matches!(
        kind,
        PushKind::Approval | PushKind::Question | PushKind::DisplayOnlyWait
    )
}

fn kind_name(kind: PushKind) -> &'static str {
    match kind {
        PushKind::Error => "error",
        PushKind::Completion => "completion",
        PushKind::Approval => "approval",
        PushKind::Question => "question",
        PushKind::DisplayOnlyWait => "display_only_wait",
    }
}

fn join_title_body(title: &str, body: &str) -> String {
    match (title.trim().is_empty(), body.trim().is_empty()) {
        (true, _) => body.to_owned(),
        (false, true) => title.to_owned(),
        (false, false) => format!("{title}\n{body}"),
    }
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Percent-encodes one URL path segment (RFC 3986 unreserved characters stay
/// literal). Bark puts the title and body in the path, so `/`, `?` and `#`
/// must not leak into the URL structure.
fn encode_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(channel: PushChannel, target: &str, title: &str, body: &str) -> PushRequest {
        PushRequest::build(channel, target, PushKind::Approval, title, body)
            .expect("request")
    }

    #[test]
    fn bark_uses_path_segments_and_escapes_them() {
        let request = req(PushChannel::Bark, "device-key", "Needs approval", "Bash: rm -rf /");
        assert_eq!(request.method, "GET");
        assert_eq!(
            request.url,
            "https://api.day.app/device-key/Needs%20approval/Bash%3A%20rm%20-rf%20%2F"
        );
        assert!(request.body.is_empty());
    }

    #[test]
    fn bark_accepts_full_device_url() {
        let request = req(
            PushChannel::Bark,
            "https://bark.example.com/my-key/",
            "t",
            "b",
        );
        assert_eq!(request.url, "https://bark.example.com/my-key/t/b");
    }

    #[test]
    fn bark_accepts_self_hosted_device_url() {
        let request = PushRequest::build(
            PushChannel::Bark,
            "https://bark.local/dev",
            PushKind::Error,
            "t",
            "b",
        )
        .unwrap();
        assert_eq!(request.url, "https://bark.local/dev/t/b");
    }

    #[test]
    fn ntfy_posts_to_topic_with_headers() {
        let request = req(PushChannel::Ntfy, "my-topic", "Title", "Body");
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "https://ntfy.sh/my-topic");
        assert_eq!(request.body_text(), "Title\nBody");
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k == "Title" && v == "Title"));
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k == "Priority" && v == "high"));
    }

    #[test]
    fn ntfy_accepts_full_topic_url() {
        let request = PushRequest::build(
            PushChannel::Ntfy,
            "https://ntfy.example.com/topic",
            PushKind::Completion,
            "",
            "done",
        )
        .unwrap();
        assert_eq!(request.url, "https://ntfy.example.com/topic");
        assert_eq!(request.body_text(), "done");
        assert!(!request.headers.iter().any(|(k, _)| k == "Priority"));
    }

    #[test]
    fn telegram_builds_bot_url_and_json() {
        let request = req(PushChannel::Telegram, "123456:ABC-DEF/98765", "T", "B");
        assert_eq!(
            request.url,
            "https://api.telegram.org/bot123456:ABC-DEF/sendMessage"
        );
        let json = request.body_json().unwrap();
        assert_eq!(json["chat_id"], "98765");
        assert_eq!(json["text"], "T\nB");
    }

    #[test]
    fn telegram_rejects_target_without_chat() {
        let error = PushRequest::build(
            PushChannel::Telegram,
            "just-a-token",
            PushKind::Approval,
            "t",
            "b",
        )
        .unwrap_err();
        assert!(matches!(error, PushError::InvalidTarget(_)));
    }

    #[test]
    fn lark_uses_msg_type_and_content_text() {
        let request = req(PushChannel::Lark, "https://open.feishu.cn/hook/x", "T", "B");
        let json = request.body_json().unwrap();
        assert_eq!(json["msg_type"], "text");
        assert_eq!(json["content"]["text"], "T\nB");
    }

    #[test]
    fn slack_uses_text() {
        let request = req(PushChannel::Slack, "https://hooks.slack.com/services/x", "T", "B");
        let json = request.body_json().unwrap();
        assert_eq!(json["text"], "T\nB");
    }

    #[test]
    fn discord_uses_content_and_truncates() {
        let long = "x".repeat(2100);
        let request = req(PushChannel::Discord, "https://discord.com/api/webhooks/x", "T", &long);
        let json = request.body_json().unwrap();
        let content = json["content"].as_str().unwrap();
        assert_eq!(content.chars().count(), 2000);
        assert!(content.ends_with('…'));
    }

    #[test]
    fn generic_webhook_carries_structured_body() {
        let request = req(PushChannel::Webhook, "https://example.com/hook", "T", "B");
        let json = request.body_json().unwrap();
        assert_eq!(json["kind"], "approval");
        assert_eq!(json["title"], "T");
        assert_eq!(json["body"], "B");
    }

    #[test]
    fn webhook_requires_http_url() {
        let error = PushRequest::build(
            PushChannel::Webhook,
            "example.com/hook",
            PushKind::Error,
            "t",
            "b",
        )
        .unwrap_err();
        assert!(matches!(error, PushError::InvalidTarget(_)));
    }

    #[test]
    fn disabled_notifier_never_builds() {
        let mut notifier = PushNotifier::new(PushChannel::Slack, "https://example.com");
        notifier.enabled = false;
        assert!(matches!(
            notifier.build_request(PushKind::Error, "t", "b"),
            Err(PushError::Disabled)
        ));
    }

    #[test]
    fn gate_deduplicates_event_ids() {
        let mut gate = PushGate::new();
        let now = Instant::now();
        assert!(gate.should_send_at("e1", "s1", now));
        assert!(!gate.should_send_at("e1", "s1", now + Duration::from_secs(60)));
        assert!(gate.has_seen("e1"));
    }

    #[test]
    fn gate_rate_limits_each_session() {
        let mut gate = PushGate::new();
        let now = Instant::now();
        assert!(gate.should_send_at("e1", "s1", now));
        assert!(!gate.should_send_at("e2", "s1", now + Duration::from_secs(4)));
        // A different session is independent.
        assert!(gate.should_send_at("e3", "s2", now + Duration::from_secs(4)));
        // Once the window passes the same session may push again.
        assert!(gate.should_send_at("e4", "s1", now + Duration::from_secs(5)));
    }

    #[test]
    fn path_encoding_is_rfc3986() {
        assert_eq!(encode_path("a b/c?d#e"), "a%20b%2Fc%3Fd%23e");
        assert_eq!(encode_path("Az09-._~"), "Az09-._~");
        assert_eq!(encode_path("你好"), "%E4%BD%A0%E5%A5%BD");
    }

    #[tokio::test]
    async fn same_host_redirect_keeps_post_body_and_verifies_provider_reply() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = format!("http://{}/initial", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (path, reply) in [
                ("/initial", "HTTP/1.1 302 Found\r\nLocation: /delivery\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                ("/delivery", "HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{\"errcode\":41}"),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0, "client closed before delivering body");
                    bytes.extend_from_slice(&chunk[..count]);
                    let text = String::from_utf8_lossy(&bytes);
                    if text.split_once("\r\n\r\n").is_some_and(|(_, body)| body.ends_with('}')) {
                        assert!(text.starts_with(&format!("POST {path} HTTP/1.1")));
                        assert!(text.contains("Example body"));
                        break;
                    }
                }
                stream.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let notifier = PushNotifier::configured(PushChannelSetting {
            kind: "wecom".to_owned(), endpoint, enabled: true, include_details: true,
            ..Default::default()
        }, 200).unwrap();
        let result = notifier.send(PushKind::Completion, "Test", "Example body").await;
        assert!(matches!(result, Err(PushError::ServiceRejected)), "{result:?}");
        server.await.unwrap();
    }
}
