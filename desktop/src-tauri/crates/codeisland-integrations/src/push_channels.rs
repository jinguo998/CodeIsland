use chrono::{DateTime, Utc};
use codeisland_domain::{PushChannelSetting, PushKind};
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;

use crate::push::{PushChannel, PushError, PushRequest};

fn native_name(channel: PushChannel) -> &'static str {
    match channel {
        PushChannel::Lark => "feishu",
        _ => channel.name(),
    }
}

pub(crate) fn event_name(kind: PushKind) -> &'static str {
    match kind {
        PushKind::Approval => "permission",
        PushKind::Question | PushKind::DisplayOnlyWait => "question",
        PushKind::Completion => "completion",
        PushKind::Error => "error",
    }
}

#[derive(Deserialize)]
struct RenderedRequest {
    url: String,
    headers: std::collections::BTreeMap<String, String>,
    body: String,
}

#[derive(Deserialize)]
struct RequestResult {
    request: Option<RenderedRequest>,
    problem: Option<String>,
}

pub(crate) fn build(
    channel: PushChannel,
    config: &PushChannelSetting,
    kind: PushKind,
    title: &str,
    headline: &str,
    details: &str,
    now: DateTime<Utc>,
) -> Result<PushRequest, PushError> {
    if matches!(channel, PushChannel::Discord | PushChannel::Webhook) {
        // These desktop extensions accept the same endpoint and basic-auth
        // syntax as the built-in channels.
        let (url, auth) = parse_extension_endpoint(&config.endpoint)?;
        let content = match (headline, details.trim()) {
            ("", body) if config.include_details => body.to_owned(),
            (_, "") => headline.to_owned(),
            (_, body) if config.include_details => format!("{headline}\n{body}"),
            _ => headline.to_owned(),
        };
        let mut request = PushRequest::build(channel, url.as_str(), kind, title, &content)?;
        if let Some(auth) = auth { request.headers.push(("Authorization".into(), auth)); }
        return Ok(request);
    }
    let mut native_config = serde_json::to_value(config).expect("encode push settings");
    native_config["kind"] = json!(native_name(channel));
    let event = event_name(kind);
    let result: RequestResult = codeisland_swift::call(c"renderPush", json!({
        "config": native_config, "kind": event, "title": title, "headline": headline,
        "body": details.trim(), "now": now.timestamp_millis() as f64 / 1000.0,
    }));
    let request = result.request.ok_or_else(|| PushError::InvalidTarget(
        result.problem.unwrap_or_else(|| "invalid push configuration".into())
    ))?;
    let mut url = Url::parse(&request.url)
        .map_err(|_| PushError::InvalidTarget("invalid push endpoint".into()))?;
    check_transport(&url)?;
    url.set_fragment(None);
    Ok(PushRequest {
        method: "POST", url: url.to_string(), headers: request.headers.into_iter().collect(),
        body: request.body.into_bytes(),
    })
}

fn check_transport(url: &Url) -> Result<(), PushError> {
    if url.scheme() == "http" && !local_host(url) {
        return Err(PushError::InvalidTarget("HTTP is only supported for local addresses".into()));
    }
    Ok(())
}

pub(crate) fn accepted(channel: PushChannel, status: u16, body: &[u8]) -> bool {
    if matches!(channel, PushChannel::Discord | PushChannel::Webhook) {
        return (200..300).contains(&status);
    }
    codeisland_swift::call(c"acceptPush", json!({
        "channel": native_name(channel), "status": status, "body": String::from_utf8_lossy(body),
    }))
}

pub(crate) fn truncate(value: &str, count: usize) -> String {
    codeisland_swift::call(c"truncatePushText", json!({"text":value, "limit":count.min(value.len())}))
}

fn parse_extension_endpoint(raw: &str) -> Result<(Url, Option<String>), PushError> {
    let endpoint: Option<(String, Option<String>)> = codeisland_swift::call(c"pushEndpoint", raw);
    let (raw_url, auth) = endpoint.ok_or_else(|| PushError::InvalidTarget("invalid push endpoint".into()))?;
    let mut url = Url::parse(&raw_url).map_err(|_| PushError::InvalidTarget("invalid push endpoint".into()))?;
    check_transport(&url)?;
    url.set_fragment(None);
    Ok((url, auth))
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
        assert!(!accepted(PushChannel::Lark, 200, br#"{"StatusCode":17}"#));
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
