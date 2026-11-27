//! Hook payload normalization.
//!
//! Agents disagree about key names, event spellings and where the session id
//! lives. This module turns every payload into one [`AgentEvent`]. It only
//! fails for JSON that cannot be parsed; an unknown event name or source is
//! mapped best-effort so a hook never takes its agent down.

use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, codex_desktop_session_id};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::AgentError;
use crate::definitions::kind_for_source;

/// Normalize one agent hook payload.
pub fn normalize(
    source: &str,
    event_name: &str,
    raw: &str,
    cwd_hint: Option<&str>,
) -> Result<AgentEvent, AgentError> {
    normalize_with_ppid(source, event_name, raw, cwd_hint, None)
}

/// Normalize with the hook process's parent pid. When the payload carries no
/// session id, the parent pid is the only stable identity a CLI offers across
/// the separate processes each hook event spawns.
pub fn normalize_with_ppid(
    source: &str,
    event_name: &str,
    raw: &str,
    cwd_hint: Option<&str>,
    ppid_hint: Option<u32>,
) -> Result<AgentEvent, AgentError> {
    // Hooks occasionally run with empty stdin. Treat that as an empty object
    // so the event name alone can still drive the panel.
    let payload: Value = if raw.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(raw)?
    };

    Ok(project_event(source, event_name, payload, cwd_hint, ppid_hint))
}

#[derive(Deserialize)]
pub struct HookAdmission {
    pub admitted: bool,
    pub immediate: Option<String>,
}

pub fn local_hook_admission(source: &str, event: &str) -> HookAdmission {
    codeisland_swift::call(c"hook.admit", json!({
        "source": source.trim_start_matches("codeisland-"), "event": event,
    }))
}

pub struct LocalHook {
    pub event: AgentEvent,
    pub immediate: Option<String>,
    pub delay: std::time::Duration,
    pub cursor: bool,
}

/// Run in the hook process so Swift sees the firing agent's environment and
/// the original stdin bytes. Socket clients already carry their own context.
pub fn prepare_local_hook(source: &str, tag: &str, raw: &str, cwd: Option<&str>, ppid: u32) -> Result<Option<LocalHook>, AgentError> {
    #[derive(Deserialize)]
    struct Prepared {
        source: String,
        payload: Value,
        fallback: bool,
        immediate: Option<String>,
        waits: bool,
        cursor: bool,
        delay: f64,
    }
    let Some(mut hook) = codeisland_swift::call::<Option<Prepared>>(c"hook.prepare", json!({
        "source": source.trim_start_matches("codeisland-"), "event": tag, "raw": raw, "cwd": cwd,
        "parent": i32::try_from(ppid).ok(),
    })) else {
        if !raw.trim().is_empty() { serde_json::from_str::<Value>(raw)?; }
        return Ok(None);
    };
    if hook.fallback {
        // Windows plugins also send IDs in fields outside the upstream hook schema.
        if let Some(id) = serde_json::from_str(raw).ok().as_ref().and_then(pick_session_id) {
            hook.payload["session_id"] = Value::String(id);
        }
    }
    let mut event = project_event(&hook.source, tag, hook.payload, cwd, Some(ppid));
    if hook.cursor && !hook.waits && event.event_type.is_blocking() { return Ok(None); }
    event.waits = hook.waits || (!hook.cursor && event.event_type.is_blocking());
    Ok(Some(LocalHook { event, immediate: hook.immediate, delay: std::time::Duration::from_secs_f64(hook.delay),
        cursor: hook.cursor }))
}

fn project_event(
    source: &str, event_name: &str, mut payload: Value, cwd_hint: Option<&str>, ppid_hint: Option<u32>,
) -> AgentEvent {
    let agent = kind_for_source(source);

    // Prefer the event name carried in the payload (Claude and forks send
    // `hook_event_name`); fall back to the installer-provided `--event` value.
    let wire_event = payload
        .get("hook_event_name")
        .or_else(|| payload.get("hookEventName"))
        .or_else(|| payload.get("event_name"))
        .or_else(|| payload.get("type"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| event_name.to_owned());

    let mut tool = pick_tool(&payload);
    let question_event = is_question(tool.as_deref(), &payload);
    let mut event_type = if question_event {
        AgentEventType::Question
    } else {
        map_event_type(event_name, &wire_event)
    };

    let agent_id = pick_str(&payload, &["agent_id", "agentId"]);
    let parent_from_payload = pick_str(
        &payload,
        &[
            "parent_session_id",
            "parentSessionId",
            "parent_id",
            "parentId",
            "_parent_session_id",
        ],
    );
    let ppid = pick_ppid(&payload).or(ppid_hint);
    let mut session_id = pick_session_id(&payload)
        .unwrap_or_else(|| session_fallback(&agent, source, ppid));
    let mut parent_session_id = parent_from_payload.clone();

    // A subagent's events must never land on the parent's card. Give the
    // child its own session id (`<parent>#<agent id>`) and record the link.
    if matches!(
        event_type,
        AgentEventType::SubagentStart | AgentEventType::SubagentEnd
    ) {
        match (parent_from_payload.as_deref(), agent_id.as_deref()) {
            (None, None) => {
                // No identity to file the child under. Keep the parent's
                // state untouched and report a lightweight heartbeat.
                event_type = AgentEventType::Heartbeat;
                tool = Some("background".to_owned());
            }
            (parent, id) => {
                let parent = parent
                    .map(str::to_owned)
                    .unwrap_or_else(|| session_id.clone());
                session_id = match id {
                    Some(id) if !id.is_empty() => format!("{parent}#{id}"),
                    _ => format!("{parent}#sub"),
                };
                parent_session_id = Some(parent);
            }
        }
    }

    let mut event = AgentEvent::new(session_id, agent, event_type);
    event.agent_id = agent_id;
    event.parent_session_id = parent_session_id;
    event.raw_event_name = Some(wire_event);
    event.cwd = pick_cwd(&payload).or_else(|| cwd_hint.map(str::to_owned));
    event.tool = tool;
    event.text = pick_text(&payload).or_else(|| question_text(&payload));
    event.title = pick_title(&payload).or_else(|| question_text(&payload));
    event.model = pick_str(&payload, &["model", "model_name", "modelName"])
        .or_else(|| pick_pointer_str(&payload, "/extra/model"));
    event.pid = pick_pid(&payload);
    event.transcript_path = pick_str(&payload, &["transcript_path", "transcriptPath"])
        .or_else(|| pick_pointer_str(&payload, "/extra/transcript_path"));
    event.terminal_kind = pick_str(
        &payload,
        &["terminal_kind", "terminalKind", "_term_app", "term_app", "TERM_PROGRAM"],
    );
    event.wezterm_pane = pick_str(&payload, &["wezterm_pane", "_wezterm_pane", "WEZTERM_PANE"])
        .and_then(|value| value.parse().ok());

    if agent == AgentKind::Codex
        && (payload["_term_bundle"].as_str() == Some("com.openai.codex")
            || event.terminal_kind.as_deref().is_some_and(|kind| {
                kind.eq_ignore_ascii_case("codex") || kind.eq_ignore_ascii_case("codex desktop")
            }))
    {
        event.session_id = codex_desktop_session_id(&event.session_id);
        event.parent_session_id = event.parent_session_id.as_deref().map(codex_desktop_session_id);
        event.terminal_kind = Some("codex".to_owned());
    }

    // A turn that ended because the user stopped it is an interruption, not a
    // completion. Agents report the reason on the turn-ending event.
    if matches!(
        event.event_type,
        AgentEventType::TurnEnd | AgentEventType::TurnFailed
    ) {
        if let Some(reason) = pick_str(
            &payload,
            &["stop_reason", "stopReason", "reason"],
        ) {
            if is_interrupted_reason(&reason) {
                event.event_type = AgentEventType::Interrupt;
            }
            event.stop_reason = Some(reason);
        }
    }
    event.waits = event.event_type.is_blocking();

    // Expose the parsed options where the runtime/UI can read them without
    // having to know each agent's nested `tool_input` shape.
    if event.event_type == AgentEventType::Question {
        let options = question_options(&payload);
        if !options.is_empty() {
            if let Some(object) = payload.as_object_mut() {
                object.entry("options").or_insert_with(|| json!(options));
            }
        }
    }
    // Cursor replays can carry a digest; use it as the event id so the store
    // drops the duplicate instead of queueing a second request.
    if agent == AgentKind::Cursor {
        if let Some(digest) = pick_str(
            &payload,
            &["digest", "event_id", "eventId", "call_id", "callId"],
        ) {
            event.id = format!("cursor:{digest}");
        }
    }
    event.payload = Some(payload);
    event
}

/// A session id that is stable for one agent process without a wire id:
/// `<source>-ppid-<ppid>`, or `<source>-default` when even the parent pid is
/// unavailable. Never the shared "default", which merges unrelated agents.
fn session_fallback(agent: &codeisland_domain::AgentKind, source: &str, ppid: Option<u32>) -> String {
    let tag = if *agent == codeisland_domain::AgentKind::Unknown {
        source.trim_start_matches("codeisland-")
    } else {
        agent.tag()
    };
    match ppid {
        Some(ppid) => format!("{tag}-ppid-{ppid}"),
        None => format!("{tag}-default"),
    }
}

fn is_interrupted_reason(reason: &str) -> bool {
    matches!(
        reason.to_ascii_lowercase().as_str(),
        "user" | "interrupted" | "aborted" | "cancelled"
    )
}

/// Map the installed event name and the payload event name to a domain type.
fn map_event_type(installed: &str, wire: &str) -> AgentEventType {
    map_one(installed)
        .or_else(|| map_one(wire))
        .or_else(|| best_effort(wire))
        .or_else(|| best_effort(installed))
        .unwrap_or(AgentEventType::Message)
}

fn map_one(name: &str) -> Option<AgentEventType> {
    // Failures finish a turn in the original normalizer, while their raw wire
    // name still drives the error presentation.
    let raw = name.trim();
    if matches!(raw, "StopFailure" | "stop_failure" | "stopFailure") {
        return Some(AgentEventType::TurnFailed);
    }
    let canonical: String = codeisland_swift::call(c"normalizeEvent", raw);
    Some(match canonical.as_str() {
        "SessionStart" => AgentEventType::SessionStart,
        "SessionEnd" => AgentEventType::SessionEnd,
        "UserPromptSubmit" => AgentEventType::UserPrompt,
        "PreToolUse" => AgentEventType::ToolStart,
        "PostToolUse" | "PostToolUseFailure" | "postToolUseFailure" | "PermissionDenied" => AgentEventType::ToolEnd,
        "PermissionRequest" | "permissionRequest" => AgentEventType::PermissionRequest,
        "Notification" => AgentEventType::Notification,
        "Stop" | "AfterAgentResponse" | "TaskRoundComplete" => AgentEventType::TurnEnd,
        "AgentTurnSettled" => AgentEventType::Message,
        "PreCompact" | "PostCompact" | "preCompact" | "postCompact" => AgentEventType::Heartbeat,
        "SubagentStart" | "subagentStart" => AgentEventType::SubagentStart,
        "SubagentStop" | "subagentStop" => AgentEventType::SubagentEnd,
        "Interrupt" | "interrupt" => AgentEventType::Interrupt,
        _ => return None,
    })
}

fn best_effort(name: &str) -> Option<AgentEventType> {
    let lower = name.to_ascii_lowercase();
    if lower.contains("permission") {
        Some(AgentEventType::PermissionRequest)
    } else if lower.contains("question") {
        Some(AgentEventType::Question)
    } else if lower.contains("fail") || lower.contains("error") {
        Some(AgentEventType::TurnFailed)
    } else if lower.contains("session") && lower.contains("start") {
        Some(AgentEventType::SessionStart)
    } else if lower.contains("session") && lower.contains("end") {
        Some(AgentEventType::SessionEnd)
    } else if lower.contains("prompt") {
        Some(AgentEventType::UserPrompt)
    } else if lower.contains("pre") && lower.contains("tool") {
        Some(AgentEventType::ToolStart)
    } else if lower.contains("post") && lower.contains("tool") {
        Some(AgentEventType::ToolEnd)
    } else if lower.contains("notification") {
        Some(AgentEventType::Notification)
    } else if lower.contains("stop") {
        Some(AgentEventType::TurnEnd)
    } else {
        None
    }
}

fn is_question(tool: Option<&str>, payload: &Value) -> bool {
    if let Some(tool) = tool {
        let tool = tool.to_ascii_lowercase();
        if tool == "askuserquestion" || tool == "ask_user_question" {
            return true;
        }
    }
    for pointer in ["/tool_input/questions", "/toolInput/questions"] {
        if let Some(questions) = payload.pointer(pointer).and_then(Value::as_array) {
            if !questions.is_empty() {
                return true;
            }
        }
    }
    // Claude's Notification question carries a top-level `question` string.
    // Do not use a "?" heuristic: ordinary status text often ends in one and
    // must not stall the hook.
    payload
        .get("question")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
}

fn question_text(payload: &Value) -> Option<String> {
    for pointer in [
        "/tool_input/questions/0/question",
        "/toolInput/questions/0/question",
        "/tool_input/question",
        "/toolInput/question",
    ] {
        if let Some(text) = payload.pointer(pointer).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_owned());
            }
        }
    }
    payload
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn question_options(payload: &Value) -> Vec<String> {
    let mut options = Vec::new();
    for pointer in ["/tool_input/questions", "/toolInput/questions"] {
        let Some(questions) = payload.pointer(pointer).and_then(Value::as_array) else {
            continue;
        };
        for question in questions {
            let Some(choices) = question.get("options").and_then(Value::as_array) else {
                continue;
            };
            for choice in choices {
                if let Some(label) = choice.get("label").and_then(Value::as_str) {
                    options.push(label.to_owned());
                } else if let Some(label) = choice.as_str() {
                    options.push(label.to_owned());
                }
            }
        }
    }
    if options.is_empty() {
        for pointer in ["/tool_input/options", "/toolInput/options", "/options"] {
            let Some(choices) = payload.pointer(pointer).and_then(Value::as_array) else {
                continue;
            };
            for choice in choices {
                if let Some(label) = choice.get("label").and_then(Value::as_str) {
                    options.push(label.to_owned());
                } else if let Some(label) = choice.as_str() {
                    options.push(label.to_owned());
                }
            }
        }
    }
    options
}

fn pick_session_id(payload: &Value) -> Option<String> {
    for key in [
        "session_id",
        "sessionId",
        "conversation_id",
        "conversationId",
        "thread_id",
        "threadId",
    ] {
        if let Some(value) = payload.get(key) {
            if let Some(text) = value.as_str() {
                if !text.trim().is_empty() {
                    return Some(text.trim().to_owned());
                }
            } else if let Some(number) = value.as_u64() {
                return Some(number.to_string());
            } else if let Some(number) = value.as_i64() {
                return Some(number.to_string());
            }
        }
    }
    for pointer in ["/session/id", "/sessionId", "/conversation/id"] {
        if let Some(text) = payload.pointer(pointer).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.trim().to_owned());
            }
        }
    }
    None
}

fn pick_cwd(payload: &Value) -> Option<String> {
    pick_str(
        payload,
        &[
            "cwd",
            "working_directory",
            "workingDirectory",
            "workspace",
            "project_dir",
            "project_path",
            "projectPath",
        ],
    )
    .or_else(|| pick_pointer_str(payload, "/extra/cwd"))
    .or_else(|| pick_pointer_str(payload, "/extra/working_directory"))
}

fn pick_tool(payload: &Value) -> Option<String> {
    if let Some(tool) = pick_str(payload, &["tool_name", "toolName"]) {
        return Some(tool);
    }
    if let Some(tool) = payload.get("tool") {
        if let Some(name) = tool.as_str() {
            if !name.trim().is_empty() {
                return Some(name.trim().to_owned());
            }
        }
        if let Some(name) = tool.get("name").and_then(Value::as_str) {
            if !name.trim().is_empty() {
                return Some(name.trim().to_owned());
            }
        }
    }
    pick_pointer_str(payload, "/tool_use/name")
        .or_else(|| pick_pointer_str(payload, "/toolUse/name"))
}

fn pick_text(payload: &Value) -> Option<String> {
    for key in [
        "message",
        "text",
        "prompt",
        "user_prompt",
        "userPrompt",
        "input",
        "last_assistant_message",
        "lastAssistantMessage",
        "last_user_message",
        "lastUserMessage",
    ] {
        if let Some(value) = payload.get(key) {
            if let Some(text) = value.as_str() {
                let text = text.trim();
                if !text.is_empty() {
                    return Some(text.to_owned());
                }
            }
            if value.is_object() {
                if let Some(text) = pick_str(value, &["text", "content", "message"]) {
                    return Some(text);
                }
            }
        }
    }
    pick_pointer_str(payload, "/extra/user_message")
        .or_else(|| pick_pointer_str(payload, "/extra/message"))
}

fn pick_title(payload: &Value) -> Option<String> {
    pick_str(
        payload,
        &["title", "name", "session_title", "sessionTitle", "summary"],
    )
    .or_else(|| pick_pointer_str(payload, "/extra/title"))
}

fn pick_pid(payload: &Value) -> Option<u32> {
    for key in ["pid", "process_id", "processId", "_ppid", "ppid"] {
        if let Some(value) = payload.get(key) {
            if let Some(number) = value.as_u64() {
                return u32::try_from(number).ok();
            }
            if let Some(text) = value.as_str() {
                if let Ok(number) = text.trim().parse::<u32>() {
                    return Some(number);
                }
            }
        }
    }
    None
}

fn pick_ppid(payload: &Value) -> Option<u32> {
    for key in ["_ppid", "ppid", "parent_pid", "parentPid"] {
        if let Some(value) = payload.get(key) {
            if let Some(number) = value.as_u64() {
                return u32::try_from(number).ok();
            }
            if let Some(text) = value.as_str() {
                if let Ok(number) = text.trim().parse::<u32>() {
                    return Some(number);
                }
            }
        }
    }
    None
}

fn pick_str(payload: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = payload.get(*key) else {
            continue;
        };
        if let Some(text) = value.as_str() {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_owned());
            }
        } else if let Some(number) = value.as_u64() {
            return Some(number.to_string());
        } else if let Some(number) = value.as_i64() {
            return Some(number.to_string());
        }
    }
    None
}

fn pick_pointer_str(payload: &Value, pointer: &str) -> Option<String> {
    payload
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}
