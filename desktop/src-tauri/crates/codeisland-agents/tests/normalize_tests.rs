use codeisland_agents::{normalize, normalize_with_ppid};
use codeisland_domain::{AgentEventType, AgentKind, SessionStore};
use serde_json::json;

#[test]
fn normalizes_claude_permission_request() {
    let raw = r#"{
        "hook_event_name": "PermissionRequest",
        "session_id": "claude-session",
        "cwd": "C:\\work\\demo",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test"},
        "transcript_path": "C:\\logs\\session.jsonl"
    }"#;

    let event = normalize("claude", "PermissionRequest", raw, None).expect("normalize");
    assert_eq!(event.agent, AgentKind::Claude);
    assert_eq!(event.event_type, AgentEventType::PermissionRequest);
    assert_eq!(event.session_id, "claude-session");
    assert_eq!(event.cwd.as_deref(), Some("C:\\work\\demo"));
    assert_eq!(event.tool.as_deref(), Some("Bash"));
    assert_eq!(event.transcript_path.as_deref(), Some("C:\\logs\\session.jsonl"));
    assert_eq!(event.raw_event_name.as_deref(), Some("PermissionRequest"));
    assert!(event.waits);
}

#[test]
fn claude_pre_tool_use_keeps_input_detail_for_the_compact_strip() {
    let cases = [
        ("Bash", json!({"command": "cargo test --workspace"}), "cargo test --workspace"),
        ("Bash", json!({"description": "Run tests", "command": "cargo test"}), "Run tests\nCommand:\ncargo test"),
        ("Bash", json!({"description": "Run cargo test", "command": "cargo test"}), "Run cargo test"),
        ("Read", json!({"file_path": r"C:\work\src\main.rs", "offset": 20}), "main.rs:20"),
        ("Read", json!({"AbsolutePath": "/work/src/main.rs", "StartLine": 5, "EndLine": 10}), "main.rs:5-10"),
        ("Edit", json!({"file_path": "/work/src/lib.rs"}), "lib.rs"),
        ("Write", json!({"file_path": r"C:\work\NOTES.md"}), "NOTES.md"),
        ("Grep", json!({"pattern": "live_output", "path": r"C:\work\src"}), "live_output in src"),
        ("Glob", json!({"pattern": "**/*.rs"}), "**/*.rs"),
        ("WebSearch", json!({"query": "Tauri app window"}), "Tauri app window"),
        ("WebFetch", json!({"url": "https://example.com:443/details"}), "example.com"),
        ("Agent", json!({"description": "Review reducer", "prompt": "full prompt"}), "Review reducer"),
        ("Task", json!({"prompt": "Review lifecycle"}), "Review lifecycle"),
        ("TodoWrite", json!({}), "Updating tasks"),
        ("Custom", json!({"file_path": "/work/custom.json"}), "custom.json"),
    ];
    let mut store = SessionStore::default();
    for (tool, input, expected) in cases {
        let raw = json!({
            "session_id": "claude-session",
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": input
        });
        let event = normalize("claude", "PreToolUse", &raw.to_string(), None).unwrap();
        assert!(store.apply(&event).applied);
        let snapshot = store.get("claude-session").unwrap();
        assert_eq!(snapshot.active_tool.as_deref(), Some(tool));
        assert_eq!(snapshot.active_tool_detail.as_deref(), Some(expected), "{tool}");
    }
}

#[test]
fn claude_post_tool_use_only_clears_the_matching_compact_detail() {
    let mut store = SessionStore::default();
    for (kind, tool, id, input, detail) in [
        ("PreToolUse", "Bash", "first", json!({"command": "cargo test"}), Some("cargo test")),
        ("PreToolUse", "Read", "second", json!({"file_path": "/work/Cargo.toml"}), Some("Cargo.toml")),
        ("PostToolUse", "Bash", "first", json!({}), Some("Cargo.toml")),
        ("PostToolUse", "Read", "second", json!({}), None),
    ] {
        let raw = json!({
            "session_id": "claude-session",
            "hook_event_name": kind,
            "tool_name": tool,
            "tool_use_id": id,
            "tool_input": input
        });
        let event = normalize("claude", kind, &raw.to_string(), None).unwrap();
        assert!(store.apply(&event).applied);
        let snapshot = store.get("claude-session").unwrap();
        assert_eq!(snapshot.active_tool_detail.as_deref(), detail, "{kind} {id}");
    }
}

#[test]
fn normalizes_claude_ask_user_question_options() {
    let raw = r#"{
        "hook_event_name": "PreToolUse",
        "session_id": "claude-question",
        "cwd": "/tmp/project",
        "tool_name": "AskUserQuestion",
        "tool_input": {
            "questions": [{
                "question": "Which database?",
                "header": "Database",
                "options": [
                    {"label": "Postgres", "description": "Relational"},
                    {"label": "SQLite", "description": "Embedded"}
                ]
            }]
        }
    }"#;

    let event = normalize("claude", "PreToolUse", raw, None).expect("normalize");
    assert_eq!(event.event_type, AgentEventType::Question);
    assert_eq!(event.tool.as_deref(), Some("AskUserQuestion"));
    assert_eq!(event.title.as_deref(), Some("Which database?"));
    assert_eq!(event.text.as_deref(), Some("Which database?"));
    assert!(event.waits);

    let payload = event.payload.expect("payload");
    assert_eq!(payload["options"], serde_json::json!(["Postgres", "SQLite"]));
    assert_eq!(payload["tool_input"]["questions"][0]["question"], "Which database?");
}

#[test]
fn normalizes_claude_notification_question() {
    let raw = r#"{
        "hook_event_name": "Notification",
        "session_id": "claude-notify",
        "question": "Continue with the deployment?",
        "options": ["Yes", "No"]
    }"#;

    let event = normalize("claude", "Notification", raw, None).expect("normalize");
    assert_eq!(event.event_type, AgentEventType::Question);
    assert!(event.waits);
    assert_eq!(event.raw_event_name.as_deref(), Some("Notification"));
    let payload = event.payload.expect("payload");
    assert_eq!(payload["options"], serde_json::json!(["Yes", "No"]));
}

#[test]
fn normalizes_codex_payload() {
    let raw = r#"{
        "session_id": "codex-thread",
        "cwd": "/repo",
        "tool_name": "shell",
        "type": "PreToolUse",
        "model": "gpt-5"
    }"#;

    let event = normalize("codex", "PreToolUse", raw, None).expect("normalize");
    assert_eq!(event.agent, AgentKind::Codex);
    assert_eq!(event.event_type, AgentEventType::ToolStart);
    assert_eq!(event.session_id, "codex-thread");
    assert_eq!(event.cwd.as_deref(), Some("/repo"));
    assert_eq!(event.tool.as_deref(), Some("shell"));
    assert_eq!(event.model.as_deref(), Some("gpt-5"));
    assert!(!event.waits);
}

#[test]
fn normalizes_gemini_payload_with_camel_case_session_id() {
    let raw = r#"{
        "sessionId": "gemini-session",
        "cwd": "/gemini",
        "tool_name": "run_shell_command",
        "type": "BeforeTool"
    }"#;

    let event = normalize("gemini", "BeforeTool", raw, None).expect("normalize");
    assert_eq!(event.agent, AgentKind::Gemini);
    assert_eq!(event.event_type, AgentEventType::PermissionRequest);
    assert_eq!(event.session_id, "gemini-session");
    assert_eq!(event.tool.as_deref(), Some("run_shell_command"));
    assert!(event.waits);
}

#[test]
fn normalizes_cursor_payload_with_conversation_id() {
    let raw = r#"{
        "conversation_id": "cursor-conversation",
        "cwd": "/cursor",
        "tool_name": "bash",
        "hook_event_name": "beforeShellExecution",
        "tool_input": {"command": "npm test"}
    }"#;

    let event = normalize("cursor", "beforeShellExecution", raw, None).expect("normalize");
    assert_eq!(event.agent, AgentKind::Cursor);
    // Cursor gates are informational; the original never holds a Cursor hook.
    assert_eq!(event.event_type, AgentEventType::ToolStart);
    assert_eq!(event.session_id, "cursor-conversation");
    assert_eq!(event.tool.as_deref(), Some("bash"));
    assert!(!event.waits);
}

#[test]
fn uses_cwd_hint_when_payload_has_none() {
    let event = normalize("claude", "SessionStart", r#"{"session_id":"s"}"#, Some("C:\\hint"))
        .expect("normalize");
    assert_eq!(event.cwd.as_deref(), Some("C:\\hint"));
}

#[test]
fn empty_payload_still_produces_an_event() {
    let event = normalize("claude", "Stop", "", None).expect("normalize");
    assert_eq!(event.event_type, AgentEventType::TurnEnd);
    assert_eq!(event.session_id, "claude-default");
}

#[test]
fn missing_session_id_falls_back_to_the_parent_pid() {
    let event = normalize_with_ppid(
        "codex",
        "Stop",
        r#"{"_ppid": 4242}"#,
        None,
        Some(4242),
    )
    .expect("normalize");
    assert_eq!(event.session_id, "codex-ppid-4242");

    // The explicit argument wins when the payload carries no ppid.
    let event = normalize_with_ppid("gemini", "AfterAgent", "{}", None, Some(7))
        .expect("normalize");
    assert_eq!(event.session_id, "gemini-ppid-7");
}

#[test]
fn declared_and_routed_child_cards_use_the_native_parent() {
    for (fields, expected) in [
        (r#""_parent_session_id":"parent""#, Some("parent")),
        (r#""_parent_session_id":"child""#, None),
        (r#""_parent_session_id":"parent","_subsession_parent":"folded""#, Some("folded")),
        (r#""_parent_session_id":"parent","_remote_host_id":"server""#, Some("parent")),
    ] {
        let raw = format!(r#"{{"session_id":"child",{fields}}}"#);
        let event = normalize("pi", "SessionStart", &raw, None).unwrap();
        assert_eq!(event.parent_session_id.as_deref(), expected);
    }
}

#[test]
fn subagent_events_get_a_child_session_id() {
    let raw = r#"{
        "hook_event_name": "SubagentStart",
        "session_id": "parent",
        "agent_id": "abc123",
        "agent_type": "explorer"
    }"#;
    let event = normalize("claude", "SubagentStart", raw, None).expect("normalize");
    assert_eq!(event.event_type, AgentEventType::SubagentStart);
    assert_eq!(event.session_id, "parent#abc123");
    assert_eq!(event.parent_session_id.as_deref(), Some("parent"));
    assert_eq!(event.agent_id.as_deref(), Some("abc123"));
}

#[test]
fn subagent_without_identity_becomes_a_background_heartbeat() {
    let raw = r#"{"hook_event_name":"SubagentStop","session_id":"parent"}"#;
    let event = normalize("claude", "SubagentStop", raw, None).expect("normalize");
    assert_eq!(event.event_type, AgentEventType::Heartbeat);
    assert_eq!(event.tool.as_deref(), Some("background"));
    assert_eq!(event.session_id, "parent");
}

#[test]
fn subagent_stop_never_ends_the_parent_session() {
    let raw = r#"{
        "hook_event_name": "SubagentStop",
        "session_id": "parent",
        "agent_id": "child-1"
    }"#;
    let event = normalize("claude", "SubagentStop", raw, None).expect("normalize");
    assert_eq!(event.event_type, AgentEventType::SubagentEnd);
    assert_eq!(event.session_id, "parent#child-1");
    assert_eq!(event.parent_session_id.as_deref(), Some("parent"));
}

#[test]
fn interrupted_stop_reasons_end_the_turn_as_an_interrupt() {
    for reason in ["user", "interrupted", "aborted", "cancelled"] {
        let raw = format!(
            r#"{{"hook_event_name":"Stop","session_id":"s","stop_reason":"{reason}"}}"#
        );
        let event = normalize("claude", "Stop", &raw, None).expect("normalize");
        assert_eq!(
            event.event_type,
            AgentEventType::Interrupt,
            "stop_reason {reason}"
        );
        assert_eq!(event.stop_reason.as_deref(), Some(reason));
    }

    let raw = r#"{"hook_event_name":"Stop","session_id":"s","stopReason":"end_turn"}"#;
    let event = normalize("claude", "Stop", raw, None).expect("normalize");
    assert_eq!(event.event_type, AgentEventType::TurnEnd);
    assert_eq!(event.stop_reason.as_deref(), Some("end_turn"));
}

#[test]
fn invalid_json_is_the_only_normalize_error() {
    let error = normalize("claude", "Stop", "{not json", None).expect_err("must fail");
    assert!(matches!(error, codeisland_agents::AgentError::InvalidJson(_)));
}

#[test]
fn marker_source_resolves_to_canonical_agent() {
    let event = normalize("codeisland-claude", "Stop", "{}", None).expect("normalize");
    assert_eq!(event.agent, AgentKind::Claude);
}

#[test]
fn codex_desktop_namespace_keeps_cli_with_the_same_thread_id_separate() {
    let cli = normalize("codex", "UserPromptSubmit", r#"{"session_id":"thread"}"#, None).unwrap();
    let desktop = normalize(
        "codex",
        "UserPromptSubmit",
        r#"{"session_id":"thread","_term_bundle":"com.openai.codex"}"#,
        None,
    ).unwrap();
    assert_eq!(cli.session_id, "thread");
    assert_eq!(desktop.session_id, "codexapp:thread");
    assert_eq!(desktop.terminal_kind.as_deref(), Some("codex"));
}

#[test]
fn hermes_emits_one_completion_for_a_settled_reply_and_stop() {
    let mut store = SessionStore::default();
    let payload = r#"{"session_id":"hermes-turn", "last_assistant_message":"done"}"#;
    let reply = normalize("hermes", "post_llm_call", payload, None).unwrap();
    assert_eq!(reply.event_type, AgentEventType::Message);
    assert!(!store.apply(&reply).ended_turn);
    let stop = normalize("hermes", "on_session_end", payload, None).unwrap();
    assert_eq!(stop.event_type, AgentEventType::TurnEnd);
    assert!(store.apply(&stop).ended_turn);
}

#[test]
fn hermes_gateway_and_scheduled_turns_use_the_native_session_and_attention_rules() {
    use codeisland_domain::{AttentionContext, AttentionDecision, AttentionSubject, decide};
    let mut store = SessionStore::default();
    let context = AttentionContext {
        audible_cues: ["Stop".to_owned()].into(),
        push_enabled: true,
        ..Default::default()
    };
    let reply = json!({"session_id":"gateway", "extra":{"platform":"telegram",
        "user_message":"status?", "assistant_response":"finished"}});
    let event = normalize("hermes", "post_llm_call", &reply.to_string(), None).unwrap();
    assert!(!store.apply(&event).ended_turn);
    let card = store.get("gateway").unwrap();
    assert_eq!(card.last_prompt.as_deref(), Some("status?"));
    assert_eq!(card.last_message.as_deref(), Some("finished"));
    let stop = normalize("hermes", "on_session_end", &reply.to_string(), None).unwrap();
    let outcome = store.apply(&stop);
    assert!(outcome.ended_turn);
    assert_eq!(decide(&stop, &outcome, AttentionSubject::default(), &context), AttentionDecision::default());

    let job = json!({"session_id":"job", "extra":{"platform":"cron"}});
    let start = normalize("hermes", "on_session_start", &job.to_string(), None).unwrap();
    store.apply(&start);
    assert!(store.get("job").is_some());
    let end = normalize("hermes", "on_session_end", &job.to_string(), None).unwrap();
    assert!(!store.apply(&end).ended_turn);
    assert!(store.get("job").is_none());
}
