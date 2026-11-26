use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use codeisland_domain::{PushChannelSetting, PushKind};
use reqwest::Url;
use ring::hmac;
use serde_json::{Value, json};

use crate::push::{PushChannel, PushError, PushRequest};

pub(crate) fn build(
    channel: PushChannel,
    config: &PushChannelSetting,
    kind: PushKind,
    title: &str,
    headline: &str,
    details: &str,
    now: DateTime<Utc>,
) -> Result<PushRequest, PushError> {
    let hosted = match channel {
        PushChannel::Bark => "https://api.day.app",
        PushChannel::Ntfy => "https://ntfy.sh",
        PushChannel::Telegram => "https://api.telegram.org",
        _ => "",
    };
    let raw = present(&config.endpoint).unwrap_or(hosted);
    let (mut url, mut authorization) = parse_endpoint(raw)?;
    let target = present(&config.target);
    let blocked = matches!(
        kind,
        PushKind::Approval | PushKind::Question | PushKind::DisplayOnlyWait
    );
    let text = if config.include_details {
        details.trim()
    } else {
        ""
    };
    let content = joined(headline, text);
    let chat = joined(title, &content);

    let body = match channel {
        PushChannel::Bark => {
            let mut path = path_components(&url);
            let device = match target {
                Some(key) => {
                    if path.last().is_some_and(|part| part == key) {
                        path.pop();
                    }
                    key.to_owned()
                }
                None => path
                    .pop()
                    .filter(|last| last != "push")
                    .ok_or_else(missing_target)?,
            };
            if path.last().is_none_or(|part| part != "push") {
                path.push("push".to_owned());
            }
            set_path(&mut url, &path);
            let mut body = json!({
                "device_key": device,
                "title": truncate(title, 120),
                "level": if blocked { "timeSensitive" } else { "active" },
                "group": present(&config.group).unwrap_or("CodeIsland"),
                "body": if text.is_empty() { truncate(headline, 200) } else { truncate_bytes(text, 2_500) },
            });
            if !text.is_empty() && !headline.is_empty() {
                body["subtitle"] = json!(truncate(headline, 200));
            }
            for (key, value) in [("icon", &config.icon), ("sound", &config.sound)] {
                if let Some(value) = present(value) {
                    body[key] = json!(value);
                }
            }
            body
        }
        PushChannel::Ntfy => {
            let mut path = path_components(&url);
            let in_path = path.pop();
            if let (Some(typed), Some(pasted)) = (target, in_path.as_deref())
                && typed != pasted
            {
                return Err(PushError::InvalidTarget(
                    "ntfy topic does not match the server URL".into(),
                ));
            }
            let topic = target.or(in_path.as_deref()).ok_or_else(missing_target)?;
            set_path(&mut url, &path);
            if let Some(token) = present(&config.token) {
                authorization = Some(if token.to_ascii_lowercase().starts_with("bearer ") {
                    token.to_owned()
                } else {
                    format!("Bearer {token}")
                });
            }
            let priority = config.priority.clamp(1, 5);
            json!({
                "topic": topic, "title": title, "message": truncate_bytes(&content, 3_800),
                "priority": if blocked { priority } else { priority.min(3) },
            })
        }
        PushChannel::DingTalk => {
            if let Some(secret) = present(&config.secret) {
                let time = now.timestamp_millis().to_string();
                let signature = sign(secret, format!("{time}\n{secret}").as_bytes());
                let preserved: Vec<_> = url
                    .query_pairs()
                    .filter(|(key, _)| key != "timestamp" && key != "sign")
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect();
                url.query_pairs_mut()
                    .clear()
                    .extend_pairs(preserved)
                    .append_pair("timestamp", &time)
                    .append_pair("sign", &signature);
            }
            json!({ "msgtype": "text", "text": { "content": truncate(&joined(&chat, "— CodeIsland"), 4_000) } })
        }
        PushChannel::Lark => {
            let text = truncate(&joined(&content, "— CodeIsland"), 4_000);
            let paragraphs: Vec<_> = text
                .split('\n')
                .map(|line| json!([{"tag":"text","text":line}]))
                .collect();
            let mut body = json!({
                "msg_type": "post",
                "content": { "post": { "zh_cn": { "title": title, "content": paragraphs } } }
            });
            if let Some(secret) = present(&config.secret) {
                let time = now.timestamp().to_string();
                body["timestamp"] = json!(time);
                body["sign"] = json!(sign(&format!("{time}\n{secret}"), &[]));
            }
            body
        }
        PushChannel::WeCom => {
            json!({ "msgtype": "text", "text": { "content": truncate_bytes(&chat, 2_048) } })
        }
        PushChannel::Slack => {
            json!({ "text": html_escape(&truncate(&chat, 3_000)), "mrkdwn": false })
        }
        PushChannel::Telegram => {
            let token = present(&config.token).ok_or_else(missing_target)?;
            let token = token
                .strip_prefix("bot")
                .filter(|value| value.starts_with(|ch: char| ch.is_ascii_digit()))
                .unwrap_or(token);
            let chat_id = target.ok_or_else(missing_target)?;
            let mut path = path_components(&url);
            path.push(format!("bot{token}"));
            path.push("sendMessage".to_owned());
            set_path(&mut url, &path);
            let text = truncate_utf16(
                &content,
                3_900usize
                    .saturating_sub(title.encode_utf16().count() + 1)
                    .max(100),
            );
            json!({
                "chat_id": chat_id, "text": format!("<b>{}</b>\n{}", html_escape(title), html_escape(&text)),
                "parse_mode": "HTML", "link_preview_options": { "is_disabled": true },
            })
        }
        PushChannel::Discord | PushChannel::Webhook => {
            let mut request = PushRequest::build(channel, url.as_str(), kind, title, &content)?;
            if let Some(auth) = authorization {
                request.headers.push(("Authorization".into(), auth));
            }
            return Ok(request);
        }
    };
    let mut request = PushRequest::json_post(url.into(), body);
    request
        .headers
        .push(("User-Agent".into(), "CodeIsland-Push/1.0".into()));
    if let Some(authorization) = authorization {
        request
            .headers
            .push(("Authorization".into(), authorization));
    }
    Ok(request)
}

fn missing_target() -> PushError {
    PushError::InvalidTarget("required channel field is empty".into())
}

fn parse_endpoint(raw: &str) -> Result<(Url, Option<String>), PushError> {
    let raw = present(raw).ok_or_else(missing_target)?;
    let address = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let mut url = Url::parse(&address)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .ok_or_else(|| PushError::InvalidTarget("expected an HTTP or HTTPS URL".into()))?;
    if url.scheme() == "http" && !local_host(&url) {
        return Err(PushError::InvalidTarget(
            "HTTP is only supported for local addresses".into(),
        ));
    }
    let authorization = (!url.username().is_empty()).then(|| {
        let auth = format!(
            "{}:{}",
            decoded(url.username()),
            decoded(url.password().unwrap_or_default())
        );
        format!("Basic {}", STANDARD.encode(auth))
    });
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    Ok((url, authorization))
}

fn path_components(url: &Url) -> Vec<String> {
    url.path_segments()
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .map(decoded)
        .collect()
}

fn decoded(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

fn local_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let unbracketed = host.trim_matches(['[', ']']);
    match unbracketed.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|ip| ip.is_private() || ip.is_loopback() || ip.is_link_local())
        }
        Err(_) => !host.contains('.') || host.trim_end_matches('.').ends_with(".local"),
    }
}
fn set_path(url: &mut Url, components: &[String]) {
    if let Ok(mut path) = url.path_segments_mut() {
        path.clear().extend(components);
    }
}

fn sign(secret: &str, message: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    STANDARD.encode(hmac::sign(&key, message).as_ref())
}

fn present(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn joined(first: &str, second: &str) -> String {
    [first, second]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn truncate(value: &str, count: usize) -> String {
    if value.chars().count() <= count {
        return value.to_owned();
    }
    value
        .chars()
        .take(count.saturating_sub(1))
        .chain(['…'])
        .collect()
}

fn truncate_bytes(value: &str, bytes: usize) -> String {
    if value.len() <= bytes {
        return value.to_owned();
    }
    let prefix = value.floor_char_boundary(bytes.saturating_sub('…'.len_utf8()));
    format!("{}…", &value[..prefix])
}

fn truncate_utf16(value: &str, max: usize) -> String {
    if value.encode_utf16().count() <= max {
        return value.to_owned();
    }
    let mut remaining = max.saturating_sub(1);
    value
        .chars()
        .take_while(|ch| {
            let fits = ch.len_utf16() <= remaining;
            remaining = remaining.saturating_sub(ch.len_utf16());
            fits
        })
        .chain(['…'])
        .collect()
}

pub(crate) fn accepted(channel: PushChannel, body: &[u8]) -> bool {
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        return true;
    };
    let code = |key: &str| {
        json[key]
            .as_i64()
            .or_else(|| json[key].as_str()?.trim().parse().ok())
    };
    match channel {
        PushChannel::Bark => code("code").is_none_or(|code| code == 200),
        PushChannel::DingTalk | PushChannel::WeCom => code("errcode").is_none_or(|code| code == 0),
        PushChannel::Lark => code("code")
            .or_else(|| code("StatusCode"))
            .is_none_or(|code| code == 0),
        PushChannel::Telegram => json["ok"].as_bool().unwrap_or(true),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_bark_ntfy_and_feishu_use_original_wire_shapes() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut config = PushChannelSetting {
            endpoint: "http://user:pass@localhost:8000/proxy/device".into(),
            include_details: true,
            ..Default::default()
        };
        let bark = build(
            PushChannel::Bark,
            &config,
            PushKind::Approval,
            "Title",
            "Headline",
            "Details",
            now,
        )
        .unwrap();
        assert_eq!(bark.method, "POST");
        assert_eq!(bark.url, "http://localhost:8000/proxy/push");
        assert!(
            bark.headers
                .contains(&("Authorization".into(), "Basic dXNlcjpwYXNz".into()))
        );
        assert_eq!(bark.body_json().unwrap()["level"], "timeSensitive");
        assert_eq!(bark.body_json().unwrap()["subtitle"], "Headline");

        config.endpoint = "https://ntfy.sh/topic".into();
        config.token = "a-token".into();
        let ntfy = build(
            PushChannel::Ntfy,
            &config,
            PushKind::Completion,
            "Title",
            "Headline",
            "Details",
            now,
        )
        .unwrap();
        assert_eq!(ntfy.url, "https://ntfy.sh/");
        assert_eq!(ntfy.body_json().unwrap()["topic"], "topic");
        assert_eq!(ntfy.body_json().unwrap()["priority"], 3);
        assert_eq!(ntfy.body_json().unwrap()["message"], "Headline\nDetails");
        assert!(
            ntfy.headers
                .contains(&("Authorization".into(), "Bearer a-token".into()))
        );

        config.endpoint = "example.test/hook".into();
        config.secret = "secret".into();
        config.include_details = false;
        let lark = build(
            PushChannel::Lark,
            &config,
            PushKind::Question,
            "Title",
            "Headline",
            "Secret details",
            now,
        )
        .unwrap();
        assert_eq!(lark.url, "https://example.test/hook");
        assert_eq!(lark.body_json().unwrap()["timestamp"], "1700000000");
        assert_eq!(
            lark.body_json().unwrap()["sign"].as_str().unwrap().len(),
            44
        );
        assert!(!lark.body_text().contains("Secret details"));
        assert!(lark.body_text().contains("— CodeIsland"));
        assert!(!accepted(PushChannel::Lark, br#"{"StatusCode":17}"#));
    }

    #[test]
    fn public_http_and_topic_mismatch_fail_without_exposing_credentials() {
        let mut config = PushChannelSetting {
            endpoint: "http://some-user:example-secret@public.example/my-topic".into(),
            ..Default::default()
        };
        let error = build(
            PushChannel::Ntfy,
            &config,
            PushKind::Completion,
            "T",
            "",
            "",
            Utc::now(),
        )
        .unwrap_err();
        assert!(!error.to_string().contains("example-secret"));
        config.endpoint = "https://ntfy.sh/first".into();
        config.target = "second".into();
        assert!(
            build(
                PushChannel::Ntfy,
                &config,
                PushKind::Completion,
                "T",
                "",
                "",
                Utc::now()
            )
            .is_err()
        );
    }
}
