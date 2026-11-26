use serde_json::Value;

use crate::{AgentEvent, project_name};

/// The Claude hook's display description for the current tool invocation.
pub(crate) fn description(event: &AgentEvent) -> Option<String> {
    let payload = event.payload.as_ref()?;
    input(payload)
        .and_then(|input| describe_input(event.tool.as_deref(), input))
        .or_else(|| {
            let fields = ["message", "text", "summary", "status", "detail", "content"];
            string(payload, &fields)
                .or_else(|| {
                    ["payload", "data"].iter().find_map(|key| string(&payload[key], &fields))
                })
                .map(ToOwned::to_owned)
        })
        .or_else(|| payload["agent_type"].as_str().map(ToOwned::to_owned))
        .or_else(|| payload["prompt"].as_str().map(|prompt| prefix(prompt, 40)))
}

fn input(payload: &Value) -> Option<&Value> {
    let fields = ["tool_input", "toolInput", "input", "arguments", "args", "params"];
    fields.into_iter().find_map(|key| payload.get(key).filter(|value| value.is_object()))
        .or_else(|| {
            let nested_fields = ["input", "tool_input", "toolInput", "arguments", "args", "params"];
            ["tool", "payload", "data"].into_iter().find_map(|container| {
                nested_fields.into_iter().find_map(|key| payload[container].get(key).filter(|value| value.is_object()))
            })
        })
        .or_else(|| payload.pointer("/toolCall/args").filter(|value| value.is_object()))
}

fn describe_input(tool: Option<&str>, input: &Value) -> Option<String> {
    match tool.unwrap_or_default() {
        "Bash" | "execute_command" | "run_command" => {
            let description = string(input, &["description"]);
            let command = string(input, &["CommandLine", "command"]);
            match (description, command) {
                (Some(description), Some(command)) if !description.contains(command) => {
                    Some(format!("{description}\nCommand:\n{command}"))
                }
                _ => description.or(command).map(ToOwned::to_owned),
            }
        }
        "Read" | "read_file" | "view_file" => {
            let name = project_name(string(input, &["AbsolutePath", "file_path"]))?;
            if let (Some(start), Some(end)) = (input["StartLine"].as_i64(), input["EndLine"].as_i64()) {
                Some(format!("{name}:{start}-{end}"))
            } else if let Some(offset) = input["offset"].as_i64() {
                Some(format!("{name}:{offset}"))
            } else {
                Some(name)
            }
        }
        "Edit" | "apply_diff" | "replace_file_content" | "multi_replace_file_content"
        | "Write" | "write_to_file" => project_name(string(input, &["TargetFile", "file_path"])),
        "Grep" | "search_files" | "grep_search" => {
            let pattern = string(input, &["Query", "pattern"])?;
            let directory = project_name(string(input, &["SearchPath", "path"]));
            Some(match directory {
                Some(directory) => format!("{pattern} in {directory}"),
                None => pattern.to_owned(),
            })
        }
        "Glob" => string(input, &["pattern"]).map(ToOwned::to_owned),
        "WebSearch" => string(input, &["query"]).map(ToOwned::to_owned),
        "WebFetch" => string(input, &["url"]).map(|url| {
            url_host(url).map(ToOwned::to_owned).unwrap_or_else(|| prefix(url, 40))
        }),
        "Task" | "Agent" => string(input, &["description"]).map(ToOwned::to_owned)
            .or_else(|| string(input, &["prompt"]).map(|prompt| prefix(prompt, 40))),
        "TodoWrite" => Some("Updating tasks".to_owned()),
        _ => project_name(string(input, &["file_path"]))
            .or_else(|| string(input, &["pattern"]).map(ToOwned::to_owned))
            .or_else(|| string(input, &["command"]).map(|command| prefix(command, 60)))
            .or_else(|| string(input, &["prompt"]).map(|prompt| prefix(prompt, 40))),
    }
}

fn string<'a>(object: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().copied().find_map(|key| object[key].as_str().map(str::trim).filter(|value| !value.is_empty()))
}

fn prefix(value: &str, length: usize) -> String {
    value.chars().take(length).collect()
}

fn url_host(url: &str) -> Option<&str> {
    let authority = url.split_once("://").map(|(_, authority)| authority)
        .or_else(|| url.strip_prefix("//"))?
        .split(['/', '?', '#']).next()?;
    let host_and_port = authority.rsplit('@').next()?;
    let host = if host_and_port.starts_with('[') {
        &host_and_port[..=host_and_port.find(']')?]
    } else {
        host_and_port.split(':').next()?
    };
    (!host.is_empty() && !host.chars().any(char::is_whitespace)).then_some(host)
}
