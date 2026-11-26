//! Claude Desktop's local Cowork/Chat metadata and append-only audit stream.

use chrono::{DateTime, Utc};
use serde_json::Value;

pub const PREFIX: &str = "cowork:";
pub const STORE_DIRECTORY: &str = "local-agent-mode-sessions";
const MAX_PARSED_LINE: usize = 512 * 1024;

pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("local_").is_some_and(|body| {
        !body.is_empty() && body.len() <= 128
            && body.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

pub fn deep_link(session_key: &str) -> Option<String> {
    let id = session_key.strip_prefix(PREFIX)?;
    valid_id(id).then(|| format!("claude://claude.ai/cowork/{id}"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub id: String,
    pub cli_id: Option<String>,
    pub title: Option<String>,
    pub initial_prompt: Option<String>,
    pub host_cwd: Option<String>,
    pub model: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub last_activity: Option<DateTime<Utc>>,
    pub archived: bool,
    pub session_type: Option<String>,
}

impl Metadata {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let data: Value = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).ok()?;
        let id = non_empty(&data["sessionId"])?;
        if !valid_id(&id) {
            return None;
        }
        let first_folder = data["userSelectedFolders"].as_array().and_then(|folders| {
            folders.iter().filter_map(non_empty).find(|path| is_host_path(path))
        });
        let host_cwd = first_folder.or_else(|| non_empty(&data["cwd"]).filter(|path| is_host_path(path)));
        let initial_prompt = non_empty(&data["initialMessage"]);
        let title = non_empty(&data["title"]).or_else(|| initial_prompt.as_deref().map(|text| clip(text, 80)));
        Some(Self {
            id,
            cli_id: non_empty(&data["cliSessionId"]),
            title,
            initial_prompt,
            host_cwd,
            model: non_empty(&data["model"]),
            created_at: epoch(&data["createdAt"]),
            last_activity: epoch(&data["lastActivityAt"]),
            archived: data["isArchived"].as_bool().unwrap_or(false),
            session_type: non_empty(&data["sessionType"]),
        })
    }

    pub fn trackable(&self) -> bool {
        !self.archived && !matches!(self.session_type.as_deref(), Some("agent" | "dispatch_child" | "radar"))
    }
}

fn is_host_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    (path.starts_with('/') && !path.starts_with("/sessions/"))
        || path.starts_with("\\\\")
        || matches!(bytes, [drive, b':', b'\\' | b'/', ..] if drive.is_ascii_alphabetic())
}

fn epoch(value: &Value) -> Option<DateTime<Utc>> {
    let millis = value.as_i64().or_else(|| {
        value.as_f64().filter(|value| value.is_finite() && *value <= i64::MAX as f64)
            .map(|value| value as i64)
    })?;
    (millis > 0).then(|| DateTime::from_timestamp_millis(millis)).flatten()
}

fn non_empty(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn clip(text: &str, max: usize) -> String {
    let mut chars = text.chars().map(|ch| if matches!(ch, '\n' | '\r') { ' ' } else { ch });
    let mut line: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        line.push('…');
    }
    line
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub name: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Permission {
    id: Option<String>,
    tool: Tool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Prompt(Option<String>),
    Activity,
    Assistant { subagent: bool, tool: Option<Tool> },
    Request { id: Option<String>, tool: Tool },
    Resolve(Option<String>),
    End { failed: bool, interrupted: bool, text: Option<String> },
    Ignored,
}

pub fn parse_audit_line(line: &[u8]) -> Event {
    let line = line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(line);
    if line.len() > MAX_PARSED_LINE {
        return parse_large_line(line);
    }
    let Ok(value) = serde_json::from_slice::<Value>(line) else { return Event::Ignored };
    let content = &value["message"]["content"];
    match value["type"].as_str() {
        Some("user") => {
            if value["tool_use_id"].as_str().is_some()
                || content.as_array().is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"))
            {
                Event::Activity
            } else {
                let text = (!value["isSynthetic"].as_bool().unwrap_or(false))
                    .then(|| prompt_text(content)).flatten();
                Event::Prompt(text)
            }
        }
        Some("assistant") => Event::Assistant {
            subagent: value.get("parent_tool_use_id").is_none_or(|parent| !parent.is_null()),
            tool: content.as_array().and_then(|blocks| {
                let block = blocks.iter().rev().find(|block| block["type"] == "tool_use")?;
                let name = non_empty(&block["name"])?;
                Some(Tool { detail: tool_detail(&name, &block["input"]), name })
            }),
        },
        Some("result") => {
            let interrupted = value["terminal_reason"].as_str().is_some_and(|reason| reason.starts_with("aborted_"));
            Event::End {
                failed: !interrupted && (value["is_error"].as_bool().unwrap_or(false)
                    || value["subtype"].as_str().is_some_and(|subtype| subtype.starts_with("error"))),
                interrupted,
                text: non_empty(&value["result"]),
            }
        }
        Some("system") => match value["subtype"].as_str() {
            Some("init") => Event::Activity,
            Some("permission_request") => {
                let name = non_empty(&value["tool_name"]).unwrap_or_else(|| "tool".to_owned());
                Event::Request {
                    id: non_empty(&value["uuid"]),
                    tool: Tool { detail: tool_detail(&name, &value["tool_input"]), name },
                }
            }
            Some("permission_response") => Event::Resolve(non_empty(&value["uuid"])),
            _ => Event::Ignored,
        },
        _ => Event::Ignored,
    }
}

fn parse_large_line(line: &[u8]) -> Event {
    let has = |marker: &[u8]| line.windows(marker.len()).any(|chunk| chunk == marker);
    if line.starts_with(br#"{"type":"assistant""#) {
        Event::Assistant { subagent: !has(br#""parent_tool_use_id":null"#), tool: None }
    } else if line.starts_with(br#"{"type":"user""#) {
        if has(br#""tool_use_id":""#) || has(br#""type":"tool_result""#) {
            Event::Activity
        } else {
            Event::Prompt(None)
        }
    } else if line.starts_with(br#"{"type":"result""#) {
        let interrupted = has(br#""terminal_reason":"aborted_"#);
        Event::End { failed: !interrupted && has(br#""is_error":true"#), interrupted, text: None }
    } else if line.starts_with(br#"{"type":"system","subtype":"init""#) {
        Event::Activity
    } else {
        Event::Ignored
    }
}

fn prompt_text(content: &Value) -> Option<String> {
    if let Some(text) = non_empty(content) {
        return Some(text);
    }
    let parts: Vec<_> = content.as_array()?.iter()
        .filter(|block| block["type"] == "text").filter_map(|block| non_empty(&block["text"])).collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn tool_detail(name: &str, input: &Value) -> Option<String> {
    if name == "AskUserQuestion" {
        let questions: Vec<_> = input["questions"].as_array().into_iter().flatten()
            .filter_map(|question| non_empty(&question["question"])).collect();
        let text = match questions.as_slice() {
            [] => non_empty(&input["question"])?,
            [first] => first.clone(),
            [first, rest @ ..] => format!("{first} (+{})", rest.len()),
        };
        return Some(clip(&text, 160));
    }
    let text = ["command", "file_path", "path", "url", "pattern", "query", "description"]
        .into_iter().find_map(|key| non_empty(&input[key]))?;
    Some(clip(&host_readable_paths(&text), 160))
}

fn host_readable_paths(mut text: &str) -> String {
    let mut output = String::new();
    while let Some((prefix, rest)) = text.split_once("/sessions/") {
        output.push_str(prefix);
        if let Some((name, path)) = rest.split_once("/mnt/")
            && !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            text = path;
        } else {
            output.push_str("/sessions/");
            text = rest;
        }
    }
    output.push_str(text);
    output
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Running,
    WaitingApproval,
    WaitingQuestion,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Audit {
    pub phase: Phase,
    pub prompt_count: u64,
    pub end_count: u64,
    pub request_count: u64,
    pub prompt: Option<String>,
    pub result: Option<String>,
    pub failed: bool,
    pub interrupted: bool,
    pub current_tool: Option<Tool>,
    permissions: Vec<Permission>,
}

impl Audit {
    pub fn active_permission(&self) -> Option<&Tool> {
        self.permissions.last().map(|permission| &permission.tool)
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Prompt(text) => {
                self.phase = Phase::Running;
                self.permissions.clear();
                self.current_tool = None;
                self.prompt_count += 1;
                self.result = None;
                self.failed = false;
                self.interrupted = false;
                if text.is_some() { self.prompt = text; }
            }
            Event::Activity => {
                if self.phase == Phase::Idle { self.phase = Phase::Running; }
            }
            Event::Assistant { subagent, tool } => {
                if !subagent {
                    self.permissions.clear();
                }
                if tool.is_some() || !subagent {
                    self.current_tool = tool;
                }
                self.refresh_phase();
            }
            Event::Request { id, tool } => {
                if id.is_some() { self.permissions.retain(|pending| pending.id != id); }
                self.permissions.push(Permission { id, tool });
                self.request_count += 1;
                self.refresh_phase();
            }
            Event::Resolve(id) => {
                if !self.permissions.is_empty() {
                    let matched = self.permissions.iter().position(|pending| id.is_some() && pending.id == id);
                    self.permissions.remove(matched.unwrap_or(0));
                }
                self.refresh_phase();
            }
            Event::End { failed, interrupted, text } => {
                self.phase = Phase::Idle;
                self.permissions.clear();
                self.current_tool = None;
                self.end_count += 1;
                self.failed = failed;
                self.interrupted = interrupted;
                self.result = text;
            }
            Event::Ignored => {}
        }
    }

    fn refresh_phase(&mut self) {
        self.phase = match self.active_permission().map(|tool| tool.name.as_str()) {
            Some("AskUserQuestion") => Phase::WaitingQuestion,
            Some(_) => Phase::WaitingApproval,
            None => Phase::Running,
        };
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conversation {
    pub prompt: Option<String>,
    pub response: Option<String>,
    pub model: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Update {
    pub metadata: Metadata,
    pub audit: Audit,
    pub conversation: Conversation,
    pub transcript_path: Option<String>,
    pub last_activity: DateTime<Utc>,
    pub restore: bool,
    pub live: bool,
    pub settle: bool,
    pub prompts_started: bool,
    pub turns_completed: bool,
    pub permissions_requested: bool,
}
