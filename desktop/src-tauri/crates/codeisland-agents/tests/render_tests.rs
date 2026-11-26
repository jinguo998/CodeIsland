use codeisland_agents::render_response;
use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, Decision, UserAction};
use serde_json::json;

fn make_event(agent: AgentKind, event_type: AgentEventType, tool: Option<&str>) -> AgentEvent {
    let mut event = AgentEvent::new("session-1", agent, event_type);
    event.tool = tool.map(str::to_owned);
    event.raw_event_name = Some("PermissionRequest".to_owned());
    event
}

fn make_decision(agent: AgentKind, action: UserAction, text: Option<&str>) -> Decision {
    Decision {
        event_id: "event-1".to_owned(),
        session_id: "session-1".to_owned(),
        agent,
        action,
        text: text.map(str::to_owned),
    }
}

#[test]
fn claude_approve() {
    let event = make_event(AgentKind::Claude, AgentEventType::PermissionRequest, Some("Bash"));
    let decision = make_decision(AgentKind::Claude, UserAction::Approve, None);
    let (stdout, code) = render_response(AgentKind::Claude, &event, Some(&decision), "{}");
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
    );
}

#[test]
fn claude_deny() {
    let event = make_event(AgentKind::Claude, AgentEventType::PermissionRequest, Some("Bash"));
    let decision = make_decision(AgentKind::Claude, UserAction::Deny, None);
    let (stdout, code) = render_response(AgentKind::Claude, &event, Some(&decision), "{}");
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny"}}}"#
    );
}

#[test]
fn claude_always_allow_adds_a_session_rule() {
    let event = make_event(AgentKind::Claude, AgentEventType::PermissionRequest, Some("Bash"));
    let decision = make_decision(AgentKind::Claude, UserAction::AlwaysAllow, None);
    let (stdout, code) = render_response(AgentKind::Claude, &event, Some(&decision), "{}");
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"*"}],"behavior":"allow","destination":"session"}]}}}"#
    );
}

#[test]
fn claude_always_allow_mcp_uses_bare_tool_name() {
    let event = make_event(
        AgentKind::Claude,
        AgentEventType::PermissionRequest,
        Some("mcp__server__tool"),
    );
    let decision = make_decision(AgentKind::Claude, UserAction::AlwaysAllow, None);
    let (stdout, _) = render_response(AgentKind::Claude, &event, Some(&decision), "{}");
    assert_eq!(
        stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":[{"type":"addRules","rules":[{"toolName":"mcp__server__tool"}],"behavior":"allow","destination":"session"}]}}}"#
    );
}

#[test]
fn claude_question_answer_copies_tool_input() {
    let mut event = make_event(AgentKind::Claude, AgentEventType::Question, Some("AskUserQuestion"));
    event.raw_event_name = Some("PreToolUse".to_owned());
    event.payload = Some(json!({
        "tool_input": {
            "questions": [{
                "question": "Which database?",
                "options": [{"label": "Postgres"}, {"label": "SQLite"}]
            }]
        }
    }));
    let decision = make_decision(AgentKind::Claude, UserAction::Answer, Some("Postgres"));
    let (stdout, code) = render_response(AgentKind::Claude, &event, Some(&decision), "{}");
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":{"questions":[{"question":"Which database?","options":[{"label":"Postgres"},{"label":"SQLite"}]}],"answers":{"Which database?":"Postgres"},"answer":"Postgres"}}}}"#
    );
}

#[test]
fn claude_notification_question_uses_notification_schema() {
    let mut event = make_event(AgentKind::Claude, AgentEventType::Question, Some("AskUserQuestion"));
    event.raw_event_name = Some("Notification".to_owned());
    event.payload = Some(json!({ "message": "Which database?" }));
    let decision = make_decision(AgentKind::Claude, UserAction::Answer, Some("SQLite"));
    let (stdout, code) = render_response(AgentKind::Claude, &event, Some(&decision), "{}");
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"Notification","answer":"SQLite"}}"#
    );
}

#[test]
fn claude_question_deny_becomes_permission_deny() {
    let mut event = make_event(AgentKind::Claude, AgentEventType::Question, Some("AskUserQuestion"));
    event.payload = Some(json!({"tool_input": {"questions": []}}));
    let decision = make_decision(AgentKind::Claude, UserAction::Deny, None);
    let (stdout, _) = render_response(AgentKind::Claude, &event, Some(&decision), "{}");
    assert_eq!(
        stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny"}}}"#
    );
}

#[test]
fn codex_behaviors() {
    let event = make_event(AgentKind::Codex, AgentEventType::PermissionRequest, Some("shell"));

    let (allow, _) = render_response(
        AgentKind::Codex,
        &event,
        Some(&make_decision(AgentKind::Codex, UserAction::Approve, None)),
        "{}",
    );
    assert_eq!(
        allow,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
    );

    let (always, _) = render_response(
        AgentKind::Codex,
        &event,
        Some(&make_decision(AgentKind::Codex, UserAction::AlwaysAllow, None)),
        "{}",
    );
    assert_eq!(
        always,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
    );

    let (deny, _) = render_response(
        AgentKind::Codex,
        &event,
        Some(&make_decision(AgentKind::Codex, UserAction::Deny, None)),
        "{}",
    );
    assert_eq!(
        deny,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny"}}}"#
    );
}

#[test]
fn gemini_and_cursor_use_plain_decision_json() {
    let event = make_event(AgentKind::Gemini, AgentEventType::PermissionRequest, Some("shell"));
    let (allow, _) = render_response(
        AgentKind::Gemini,
        &event,
        Some(&make_decision(AgentKind::Gemini, UserAction::Approve, None)),
        "{}",
    );
    assert_eq!(allow, r#"{"decision":"allow"}"#);

    let (deny, _) = render_response(
        AgentKind::Gemini,
        &event,
        Some(&make_decision(AgentKind::Gemini, UserAction::Deny, None)),
        "{}",
    );
    assert_eq!(deny, r#"{"decision":"deny"}"#);

    let cursor = make_event(AgentKind::Cursor, AgentEventType::PermissionRequest, Some("bash"));
    let (cursor_allow, _) = render_response(
        AgentKind::Cursor,
        &cursor,
        Some(&make_decision(AgentKind::Cursor, UserAction::AlwaysAllow, None)),
        "{}",
    );
    assert_eq!(cursor_allow, r#"{"decision":"allow"}"#);
}

#[test]
fn no_decision_returns_the_fallback() {
    let event = make_event(AgentKind::Claude, AgentEventType::PermissionRequest, Some("Bash"));
    let (stdout, code) = render_response(AgentKind::Claude, &event, None, "{}");
    assert_eq!(code, 0);
    assert_eq!(stdout, "{}");
}

#[test]
fn unknown_agent_falls_back_but_still_renders_json() {
    let event = make_event(AgentKind::Unknown, AgentEventType::PermissionRequest, Some("Bash"));
    let decision = make_decision(AgentKind::Unknown, UserAction::Approve, None);
    let (stdout, _) = render_response(AgentKind::Unknown, &event, Some(&decision), "{}");
    assert_eq!(stdout, r#"{"decision":"allow"}"#);
}

#[test]
fn zcode_uses_the_claude_schema_without_permissions() {
    let event = make_event(AgentKind::ZCode, AgentEventType::PermissionRequest, Some("Bash"));
    let (allow, _) = render_response(
        AgentKind::ZCode,
        &event,
        Some(&make_decision(AgentKind::ZCode, UserAction::Approve, None)),
        "{}",
    );
    assert_eq!(
        allow,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
    );

    let (deny, _) = render_response(
        AgentKind::ZCode,
        &event,
        Some(&make_decision(AgentKind::ZCode, UserAction::Deny, None)),
        "{}",
    );
    assert_eq!(
        deny,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny"}}}"#
    );

    let (always, _) = render_response(
        AgentKind::ZCode,
        &event,
        Some(&make_decision(AgentKind::ZCode, UserAction::AlwaysAllow, None)),
        "{}",
    );
    assert_eq!(
        always,
        r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#,
        "ZCode must never receive updatedPermissions"
    );
    assert!(!always.contains("updatedPermissions"));
}

#[test]
fn minimax_always_allow_uses_a_bare_tool_name() {
    let event = make_event(AgentKind::MiniMax, AgentEventType::PermissionRequest, Some("Bash"));
    let decision = make_decision(AgentKind::MiniMax, UserAction::AlwaysAllow, None);
    let (stdout, _) = render_response(AgentKind::MiniMax, &event, Some(&decision), "{}");
    assert!(stdout.contains("\"toolName\":\"Bash\""));
    assert!(!stdout.contains("ruleContent"), "MiniMax reads bare tool names");
}

#[test]
fn qoder_keeps_the_always_allow_rule() {
    let event = make_event(AgentKind::Qoder, AgentEventType::PermissionRequest, Some("Bash"));
    let decision = make_decision(AgentKind::Qoder, UserAction::AlwaysAllow, None);
    let (stdout, _) = render_response(AgentKind::Qoder, &event, Some(&decision), "{}");
    assert!(stdout.contains("updatedPermissions"));
}
