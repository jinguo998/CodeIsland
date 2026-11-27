//! Wire protocol: NDJSON messages exchanged between `codeisland-hook` and the
//! desktop app.
//!
//! Every frame is exactly one JSON object followed by `\n`. The `kind` tag
//! selects the variant and is snake_case.

use codeisland_domain::{AgentEvent, UserAction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Version carried in every message. Bump when the wire format changes.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        v: u32,
        id: String,
        #[serde(default)]
        source: Option<String>,
    },
    Event {
        v: u32,
        id: String,
        source: String,
        event: AgentEvent,
        #[serde(default)]
        waits: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Original Swift HookIntake payload plus named-pipe correlation and timeout.
    /// Older clients continue to send the projected Event variant.
    Hook {
        v: u32,
        id: String,
        payload: serde_json::Value,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Heartbeat {
        v: u32,
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerMessage {
    Ack {
        v: u32,
        id: String,
        ack_for: String,
    },
    Decision {
        v: u32,
        id: String,
        ack_for: String,
        action: UserAction,
        #[serde(default)]
        text: Option<String>,
        stdout: String,
        exit_code: i32,
    },
    Error {
        v: u32,
        id: String,
        error: String,
    },
}

/// Serialize one message as a single NDJSON frame (JSON plus a trailing `\n`).
pub fn encode_line<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Parse one NDJSON frame. A trailing `\n` or `\r\n` is tolerated; anything
/// else (including an empty line) must be valid JSON for the target type.
pub fn decode_line<T: DeserializeOwned>(line: &[u8]) -> serde_json::Result<T> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    serde_json::from_slice(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEventType, AgentKind};

    fn sample_event() -> AgentEvent {
        let mut event = AgentEvent::new(
            "session-1",
            AgentKind::Claude,
            AgentEventType::PermissionRequest,
        );
        event.cwd = Some("/work/project".to_string());
        event.tool = Some("Bash".to_string());
        event.raw_event_name = Some("PermissionRequest".to_string());
        event
    }

    #[test]
    fn encode_line_appends_exactly_one_newline() {
        let bytes = encode_line(&ClientMessage::Heartbeat {
            v: PROTOCOL_VERSION,
            id: "hb-1".to_string(),
        })
        .expect("encode");
        assert!(bytes.ends_with(b"\n"));
        assert!(!bytes.ends_with(b"\n\n"));
    }

    #[test]
    fn hello_round_trips() {
        let message = ClientMessage::Hello {
            v: PROTOCOL_VERSION,
            id: "hello-1".to_string(),
            source: Some("claude".to_string()),
        };
        let bytes = encode_line(&message).expect("encode");
        match decode_line::<ClientMessage>(&bytes).expect("decode") {
            ClientMessage::Hello { v, id, source } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(id, "hello-1");
                assert_eq!(source.as_deref(), Some("claude"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn event_round_trips() {
        let event = sample_event();
        let message = ClientMessage::Event {
            v: PROTOCOL_VERSION,
            id: "req-1".to_string(),
            source: "claude".to_string(),
            event: event.clone(),
            waits: true,
            timeout_ms: Some(30_000),
        };
        let bytes = encode_line(&message).expect("encode");
        match decode_line::<ClientMessage>(&bytes).expect("decode") {
            ClientMessage::Event {
                v,
                id,
                source,
                event: decoded,
                waits,
                timeout_ms,
            } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(id, "req-1");
                assert_eq!(source, "claude");
                assert!(waits);
                assert_eq!(timeout_ms, Some(30_000));
                assert_eq!(decoded.id, event.id);
                assert_eq!(decoded.session_id, "session-1");
                assert_eq!(decoded.event_type, AgentEventType::PermissionRequest);
                assert_eq!(decoded.tool.as_deref(), Some("Bash"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn decision_round_trips() {
        let message = ServerMessage::Decision {
            v: PROTOCOL_VERSION,
            id: "reply-1".to_string(),
            ack_for: "req-1".to_string(),
            action: UserAction::Answer,
            text: Some("yes".to_string()),
            stdout: r#"{"decision":"allow"}"#.to_string(),
            exit_code: 0,
        };
        let bytes = encode_line(&message).expect("encode");
        match decode_line::<ServerMessage>(&bytes).expect("decode") {
            ServerMessage::Decision {
                v,
                id,
                ack_for,
                action,
                text,
                stdout,
                exit_code,
            } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(id, "reply-1");
                assert_eq!(ack_for, "req-1");
                assert_eq!(action, UserAction::Answer);
                assert_eq!(text.as_deref(), Some("yes"));
                assert_eq!(stdout, r#"{"decision":"allow"}"#);
                assert_eq!(exit_code, 0);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn wire_shape_uses_kind_tag_and_snake_case() {
        let value = serde_json::to_value(ClientMessage::Heartbeat {
            v: PROTOCOL_VERSION,
            id: "hb".to_string(),
        })
        .expect("serialize");
        assert_eq!(value["kind"], "heartbeat");

        let value = serde_json::to_value(ServerMessage::Ack {
            v: PROTOCOL_VERSION,
            id: "ack".to_string(),
            ack_for: "req".to_string(),
        })
        .expect("serialize");
        assert_eq!(value["kind"], "ack");

        let value = serde_json::to_value(ClientMessage::Event {
            v: PROTOCOL_VERSION,
            id: "req-1".to_string(),
            source: "claude".to_string(),
            event: sample_event(),
            waits: false,
            timeout_ms: None,
        })
        .expect("serialize");
        assert_eq!(value["kind"], "event");
        assert_eq!(value["event"]["type"], "permission_request");
    }

    #[test]
    fn defaults_are_optional() {
        let json = br#"{"kind":"event","v":1,"id":"x","source":"claude","event":{"id":"e","session_id":"s","type":"tool_start","timestamp":"2026-01-01T00:00:00Z"}}"#;
        match decode_line::<ClientMessage>(json).expect("decode") {
            ClientMessage::Event {
                waits, timeout_ms, ..
            } => {
                assert!(!waits);
                assert_eq!(timeout_ms, None);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn malformed_lines_are_rejected() {
        assert!(decode_line::<ClientMessage>(b"not json\n").is_err());
        assert!(decode_line::<ClientMessage>(b"{}\n").is_err());
        assert!(decode_line::<ClientMessage>(b"\n").is_err());
        assert!(decode_line::<ClientMessage>(b"").is_err());
        assert!(decode_line::<ClientMessage>(br#"{"kind":"bogus"}"#).is_err());
        assert!(decode_line::<ServerMessage>(br#"{"kind":"hello","v":1,"id":"x"}"#).is_err());
    }

    #[test]
    fn trailing_crlf_is_tolerated() {
        let json = br#"{"kind":"heartbeat","v":1,"id":"hb"}"#;
        let mut line = json.to_vec();
        line.extend_from_slice(b"\r\n");
        match decode_line::<ClientMessage>(&line).expect("decode") {
            ClientMessage::Heartbeat { id, .. } => assert_eq!(id, "hb"),
            other => panic!("unexpected message: {other:?}"),
        }
    }
}
