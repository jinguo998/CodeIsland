use std::borrow::Cow;

use serde_json::Value;

use crate::{AgentEvent, AgentEventType, AgentTask, SessionSnapshot, TaskStatus};

#[derive(Debug, Clone)]
pub enum TranscriptMetadata {
    Recap(String),
    Checklist(Vec<ChecklistUpdate>),
}

#[derive(Debug, Clone)]
pub enum ChecklistUpdate {
    Replace(Vec<AgentTask>),
    Row {
        id: String,
        title: Option<String>,
        status: Option<TaskStatus>,
        expected_from: Option<TaskStatus>,
        remove: bool,
        insert: bool,
    },
}

pub fn claude_transcript_metadata(row: &Value) -> Option<TranscriptMetadata> {
    if row["isSidechain"] == true {
        return None;
    }
    let changes = match row["type"].as_str()? {
        "system" if row["subtype"] == "away_summary" => {
            let text = row["content"].as_str()?.trim();
            let text = text.rsplit_once('(')
                .filter(|(_, suffix)| suffix.ends_with(')') && suffix.contains("/config"))
                .map_or(text, |(summary, _)| summary.trim_end());
            return (!text.is_empty()).then(|| TranscriptMetadata::Recap(text.to_owned()));
        }
        "assistant" => {
            let message = row.get("message").unwrap_or(row);
            message["content"].as_array()?.iter()
                .filter(|block| block["type"] == "tool_use")
                .flat_map(|block| {
                    let tool = block["name"].as_str().unwrap_or_default();
                    snapshot_tool(tool, &decode_json(&block["input"]))
                })
                .collect::<Vec<_>>()
        }
        "user" => {
            let message = row.get("message").unwrap_or(row);
            let mut results = message["content"].as_array()?.iter().filter(|block| block["type"] == "tool_result");
            let result = results.next()?;
            if results.next().is_some() || result["is_error"] == true {
                return None;
            }
            let response = &row["toolUseResult"];
            if response["task"].is_object() && response["task"].get("status").is_none() {
                created_task(&Value::Null, response).into_iter().collect()
            } else if response["success"] == true && response.get("updatedFields").is_some() {
                updated_task(&Value::Null, response).into_iter().collect()
            } else {
                Vec::new()
            }
        }
        _ => return None,
    };
    (!changes.is_empty()).then_some(TranscriptMetadata::Checklist(changes))
}

pub fn codex_transcript_checklist(payload: &Value) -> Option<TranscriptMetadata> {
    let tool = payload.get("name").and_then(Value::as_str)?;
    let input = payload.get("arguments").or_else(|| payload.get("input"))?;
    let changes = snapshot_tool(tool, &decode_json(input));
    (!changes.is_empty()).then_some(TranscriptMetadata::Checklist(changes))
}

pub(crate) fn hook_checklist(event: &AgentEvent) -> Vec<ChecklistUpdate> {
    // A subagent's own list belongs to the child. Failed/denied hooks cannot
    // make an uncommitted tool call appear in a successful checklist.
    if event.agent_id.is_some()
        || event.event_type != AgentEventType::ToolEnd
        || event.raw_event_name.as_deref().is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            name.contains("fail") || name.contains("denied")
        })
    {
        return Vec::new();
    }
    let (Some(tool), Some(payload)) = (event.tool.as_deref(), event.payload.as_ref()) else {
        return Vec::new();
    };
    let input = decode_json(payload.get("tool_input").or_else(|| payload.get("toolInput"))
        .or_else(|| payload.get("arguments")).unwrap_or(&Value::Null));
    let response = decode_json(payload.get("tool_response").or_else(|| payload.get("toolResponse"))
        .unwrap_or(&Value::Null));
    if response["success"] == false {
        return Vec::new();
    }
    match tool_key(tool).as_str() {
        "taskcreate" => created_task(&input, &response).into_iter().collect(),
        "taskupdate" => updated_task(&input, &response).into_iter().collect(),
        _ => snapshot_tool(tool, &input),
    }
}

fn snapshot_tool(tool: &str, input: &Value) -> Vec<ChecklistUpdate> {
    let key = match tool_key(tool).as_str() {
        "todowrite" | "writetodos" => "todos",
        "updateplan" => "plan",
        _ => return Vec::new(),
    };
    let rows = decode_json(&input[key]);
    let Some(rows) = rows.as_array() else { return Vec::new() };
    let merging = key == "todos" && (input["merge"] == true || input["merge"] == "true");
    let mut replacement = Vec::new();
    let mut changes = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let title = text(row, &["content", "subject", "step", "title", "description", "text", "task"]);
        let (status, remove) = task_status(&row["status"]);
        let provider_id = id(&row["id"]);
        if merging {
            if let Some(id) = provider_id.or_else(|| title.as_ref().map(|title| format!("title:{title}"))) {
                changes.push(ChecklistUpdate::Row { id, title, status, expected_from: None, remove, insert: true });
            }
        } else if !remove && let Some(title) = title {
            replacement.push(AgentTask {
                id: provider_id.unwrap_or_else(|| format!("row:{index}")),
                title,
                status: status.unwrap_or(TaskStatus::Pending),
            });
        }
    }
    if merging { changes } else { vec![ChecklistUpdate::Replace(replacement)] }
}

fn created_task(input: &Value, response: &Value) -> Option<ChecklistUpdate> {
    let task = &response["task"];
    let id = id(&task["id"]).or_else(|| {
        let (_, suffix) = response.as_str()?.split_once("Task #")?;
        let (id, _) = suffix.split_once(" created successfully")?;
        (!id.is_empty() && !id.contains(char::is_whitespace)).then(|| id.to_owned())
    })?;
    let title = text(task, &["subject", "title"])
        .or_else(|| text(input, &["subject", "title", "content"]))?;
    Some(ChecklistUpdate::Row { id, title: Some(title), status: None, expected_from: None, remove: false, insert: true })
}

fn updated_task(input: &Value, response: &Value) -> Option<ChecklistUpdate> {
    let id = ["taskId", "task_id", "id"].into_iter().find_map(|key| id(&input[key]))
        .or_else(|| id(&response["taskId"]))?;
    let change = &response["statusChange"];
    let (status, remove) = task_status(input.get("status").unwrap_or(&change["to"]));
    let (expected_from, _) = task_status(&change["from"]);
    let title = text(input, &["subject", "title"]);
    (status.is_some() || remove || title.is_some()).then_some(ChecklistUpdate::Row {
        id, title, status, expected_from, remove, insert: false,
    })
}

pub(crate) fn apply_checklist(session: &mut SessionSnapshot, updates: Vec<ChecklistUpdate>, at: chrono::DateTime<chrono::Utc>) -> bool {
    if updates.is_empty() || session.tasks_updated_at.is_some_and(|previous| previous > at) {
        return false;
    }
    let mut changed = false;
    for update in updates {
        match update {
            ChecklistUpdate::Replace(rows) if session.tasks != rows => {
                session.tasks = rows;
                changed = true;
            }
            ChecklistUpdate::Replace(_) => {}
            ChecklistUpdate::Row { id, title, status, expected_from, remove, insert } => {
                let index = session.tasks.iter().position(|row| {
                    row.id == id || id.strip_prefix("title:") == Some(row.title.as_str())
                });
                if let Some(index) = index {
                    let row = &mut session.tasks[index];
                    if expected_from.is_some_and(|previous| row.status != previous) {
                        continue;
                    }
                    if remove {
                        session.tasks.remove(index);
                        changed = true;
                    } else {
                        if let Some(title) = title.filter(|title| row.title != *title) {
                            row.title = title;
                            changed = true;
                        }
                        if let Some(status) = status.filter(|status| row.status != *status) {
                            row.status = status;
                            changed = true;
                        }
                    }
                } else if insert && !remove && let Some(title) = title {
                    session.tasks.push(AgentTask { id, title, status: status.unwrap_or(TaskStatus::Pending) });
                    changed = true;
                }
            }
        }
    }
    session.tasks_updated_at = Some(at);
    changed
}

fn tool_key(tool: &str) -> String {
    tool.to_ascii_lowercase().replace(['_', '-', ' '], "")
}

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| value.get(key)?.as_str().map(str::trim)
        .filter(|text| !text.is_empty()).map(ToOwned::to_owned))
}

fn id(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.trim().is_empty()).then(|| text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn decode_json(value: &Value) -> Cow<'_, Value> {
    value.as_str().and_then(|text| serde_json::from_str::<Value>(text).ok())
        .map_or(Cow::Borrowed(value), Cow::Owned)
}

fn task_status(value: &Value) -> (Option<TaskStatus>, bool) {
    let status = value.as_str().unwrap_or_default().trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match status.as_str() {
        "pending" | "todo" | "not_started" | "open" | "queued" => (Some(TaskStatus::Pending), false),
        "in_progress" | "inprogress" | "active" | "running" | "doing" | "started" => (Some(TaskStatus::InProgress), false),
        "completed" | "complete" | "done" | "finished" => (Some(TaskStatus::Completed), false),
        "deleted" | "removed" | "cancelled" | "canceled" => (None, true),
        _ => (None, false),
    }
}
