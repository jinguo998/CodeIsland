//! Best-effort persistence of Codex "Always allow" rules.
//!
//! Codex reads `$CODEX_HOME/rules/*.rules` and `config.toml` when a session
//! starts, so a rule saved here stops the prompt for sessions started
//! afterwards. Two shapes are written, mirroring macOS `CodexPermissionRules`:
//!
//! * a Bash command's prefix rules in `rules/codeisland.rules`,
//! * an MCP tool's approval table in `config.toml`
//!   (`[mcp_servers.<id>.tools.<tool>]`, `approval_mode = "approve"`).
//!
//! Every failure is ignored — the hook's answer is never affected.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, AppSettings, ApprovalAlwaysScope, PendingRequest};
use codeisland_domain::approval::{command_for_event, prefix_patterns};

static RULE_WRITES: Mutex<()> = Mutex::new(());

/// The exact comment CodeIsland stamps above every approval table it writes.
const MCP_APPROVAL_COMMENT: &str =
    "# Added by CodeIsland when \"Always Allow\" is clicked for a Codex MCP tool.";

/// Freeze the same rule and target the user will see on the card.
pub fn stage_approval(
    event: &AgentEvent,
    settings: &AppSettings,
) -> (ApprovalAlwaysScope, Option<PathBuf>) {
    let session = (ApprovalAlwaysScope::Session, None);
    if event.agent != AgentKind::Codex || event.event_type != AgentEventType::PermissionRequest {
        return session;
    }
    let Some(payload) = event.payload.as_ref() else { return session };
    if payload.get("_remote_host_id").is_some() {
        return session;
    }
    let Some(config) = codex_config_for_event(event, settings) else { return session };
    if let Some((namespace, _)) = mcp_tool_target(event.tool.as_deref()) {
        let declared = fs::read_to_string(&config).ok()
            .is_some_and(|contents| declared_mcp_server_id(&contents, &namespace).is_some());
        if declared {
            return (ApprovalAlwaysScope::CodexMcpConfig, Some(config));
        }
    } else if let Some(patterns) = command_for_event(event).and_then(prefix_patterns) {
        return (ApprovalAlwaysScope::CodexRules { patterns }, Some(config));
    }
    session
}

pub fn persist_approved_scope(request: &PendingRequest) {
    let Some(config) = request.approval_config.as_deref() else { return };
    let _write = RULE_WRITES.lock().unwrap_or_else(|error| error.into_inner());
    match &request.always_scope {
        ApprovalAlwaysScope::CodexRules { patterns } => {
            let rules = config.with_file_name("rules").join("codeisland.rules");
            let _ = write_rules(&rules, patterns);
        }
        ApprovalAlwaysScope::CodexMcpConfig => {
            if let Some((namespace, tool)) = mcp_tool_target(request.tool.as_deref()) {
                let _ = write_mcp_approval(config, &namespace, &tool);
            }
        }
        ApprovalAlwaysScope::Session => {}
    }
}

/// `mcp__<server>__<tool>` -> `(server namespace, tool name)`.
fn mcp_tool_target(tool: Option<&str>) -> Option<(String, String)> {
    let rest = tool?.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server.to_owned(), tool.to_owned()))
}

fn codex_config_for_event(event: &AgentEvent, settings: &AppSettings) -> Option<PathBuf> {
    let paths = crate::paths::Paths::current();
    if let Some(transcript) = event.transcript_path.as_deref().and_then(|path| fs::canonicalize(path).ok()) {
        let root = settings.extra_config_dirs.iter()
            .filter(|raw| !raw.trim().is_empty())
            .map(|raw| paths.expand(raw))
            .flat_map(|root| [root.join(".codex"), root])
            .filter_map(|root| {
                let canonical = fs::canonicalize(&root).ok()?;
                let config = root.join("config.toml");
                (config.is_file() && transcript.starts_with(&canonical))
                    .then_some((canonical.components().count(), config))
            })
            .max_by_key(|(components, _)| *components);
        if let Some((_, config)) = root {
            return Some(config);
        }
    }
    let root = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))?;
    Some(root.join("config.toml"))
}

fn write_rules(path: &Path, patterns: &[Vec<String>]) -> std::io::Result<()> {
    let path = crate::paths::resolve_write_path(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut updated = existing.clone();
    for pattern in patterns {
        let line = pattern_line(pattern);
        if updated.contains(&line) {
            continue;
        }
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(&rule_block(pattern));
    }
    if updated != existing {
        fs::write(&path, updated)?;
    }
    Ok(())
}

/// Save `approval_mode = "approve"` under the declared MCP server's tools
/// table. Nothing is written when no `[mcp_servers.<id>]` declares a
/// transport: an implicit table would make Codex reject the whole file.
fn write_mcp_approval(path: &Path, namespace: &str, tool: &str) -> std::io::Result<()> {
    let path = crate::paths::resolve_write_path(path);
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let Some(updated) = config_with_mcp_tool_approval(&existing, namespace, tool) else {
        return Ok(());
    };
    if updated != existing {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, updated)?;
    }
    Ok(())
}

fn config_with_mcp_tool_approval(contents: &str, namespace: &str, tool: &str) -> Option<String> {
    let server = declared_mcp_server_id(contents, namespace)?;
    let table = ["mcp_servers", server.as_str(), "tools", tool];
    Some(insert_or_update_approval_table(contents, &table))
}

fn insert_or_update_approval_table(contents: &str, table_path: &[&str]) -> String {
    let mut lines: Vec<String> = contents.lines().map(str::to_owned).collect();

    let header = lines.iter().position(|line| {
        toml_table_segments(line).is_some_and(|segments| {
            segments.len() == table_path.len()
                && segments
                    .iter()
                    .zip(table_path)
                    .all(|(left, right)| left.as_str() == *right)
        })
    });
    if let Some(header) = header {
        let end = lines[header + 1..]
            .iter()
            .position(|line| toml_table_segments(line).is_some())
            .map(|offset| header + 1 + offset)
            .unwrap_or(lines.len());
        if let Some(approval) = lines[header + 1..end]
            .iter()
            .position(|line| toml_key(line) == Some("approval_mode"))
            .map(|offset| header + 1 + offset)
        {
            lines[approval] = "approval_mode = \"approve\"".to_owned();
        } else {
            lines.insert(header + 1, "approval_mode = \"approve\"".to_owned());
        }
    } else {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(MCP_APPROVAL_COMMENT.to_owned());
        let header = table_path
            .iter()
            .map(|segment| toml_key_segment(segment))
            .collect::<Vec<_>>()
            .join(".");
        lines.push(format!("[{header}]"));
        lines.push("approval_mode = \"approve\"".to_owned());
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The config.toml table key to write a tool approval under, for the server
/// namespace in an `mcp__<namespace>__<tool>` name. Codex sanitises server
/// names inside tool names (`davinci-resolve` -> `davinci_resolve`), so an
/// exact match is preferred and a unique sanitised match is recovered; an
/// ambiguous or undeclared server yields nothing.
fn declared_mcp_server_id(contents: &str, namespace: &str) -> Option<String> {
    if config_declares_mcp_transport(contents, namespace) {
        return Some(namespace.to_owned());
    }
    let candidates: Vec<String> = declared_mcp_server_ids(contents)
        .into_iter()
        .filter(|server| sanitized_tool_namespace(server) == namespace)
        .collect();
    if candidates.len() == 1 {
        candidates.into_iter().next()
    } else {
        None
    }
}

/// Every `[mcp_servers.<id>]` that declares a transport.
fn declared_mcp_server_ids(contents: &str) -> Vec<String> {
    let lines: Vec<&str> = contents.lines().collect();
    let mut ids: Vec<String> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(segments) = toml_table_segments(line) else {
            continue;
        };
        if segments.len() != 2 || segments[0] != "mcp_servers" {
            continue;
        }
        let end = lines[index + 1..]
            .iter()
            .position(|line| toml_table_segments(line).is_some())
            .map(|offset| index + 1 + offset)
            .unwrap_or(lines.len());
        let declares_transport = lines[index + 1..end]
            .iter()
            .any(|body| matches!(toml_key(body), Some("command" | "url")));
        if declares_transport && !ids.contains(&segments[1]) {
            ids.push(segments[1].clone());
        }
    }
    ids
}

fn config_declares_mcp_transport(contents: &str, server_id: &str) -> bool {
    let lines: Vec<&str> = contents.lines().collect();
    let Some(header) = lines.iter().position(|line| {
        toml_table_segments(line).is_some_and(|segments| {
            segments.len() == 2
                && segments[0] == "mcp_servers"
                && segments[1] == server_id
        })
    }) else {
        return false;
    };
    let end = lines[header + 1..]
        .iter()
        .position(|line| toml_table_segments(line).is_some())
        .map(|offset| header + 1 + offset)
        .unwrap_or(lines.len());
    lines[header + 1..end]
        .iter()
        .any(|line| matches!(toml_key(line), Some("command" | "url")))
}

/// How a server name reaches us inside a tool name: anything that is not a
/// letter, number or underscore becomes `_` (`__` is the field separator).
fn sanitized_tool_namespace(server_id: &str) -> String {
    server_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn toml_key(line: &str) -> Option<&str> {
    let stripped = strip_toml_comment(line).trim();
    let (key, _) = stripped.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        None
    } else {
        Some(key)
    }
}

fn toml_table_segments(line: &str) -> Option<Vec<String>> {
    let trimmed = strip_toml_comment(line).trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') || trimmed.starts_with("[[") {
        return None;
    }
    let body = &trimmed[1..trimmed.len() - 1];
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaping = false;
    for ch in body.chars() {
        if let Some(active) = quote {
            if escaping {
                current.push(ch);
                escaping = false;
            } else if active == '"' && ch == '\\' {
                escaping = true;
            } else if ch == active {
                quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '.' {
            let segment = current.trim();
            if segment.is_empty() {
                return None;
            }
            segments.push(segment.to_owned());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    let segment = current.trim();
    if !segment.is_empty() {
        segments.push(segment.to_owned());
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaping = false;
    for (index, ch) in line.char_indices() {
        if let Some(active) = quote {
            if escaping {
                escaping = false;
            } else if active == '"' && ch == '\\' {
                escaping = true;
            } else if ch == active {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '#' {
            return &line[..index];
        }
    }
    line
}

fn toml_key_segment(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn pattern_line(pattern: &[String]) -> String {
    let quoted: Vec<String> = pattern.iter().map(|word| quoted(word)).collect();
    format!("pattern = [{}]", quoted.join(", "))
}

fn rule_block(pattern: &[String]) -> String {
    format!(
        "# Added by CodeIsland when \"Always Allow\" is clicked for Codex.\n\
         prefix_rule(\n    {},\n    decision = \"allow\",\n    justification = \"Allowed from CodeIsland Always Allow\",\n)\n\n",
        pattern_line(pattern)
    )
}

fn quoted(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_commands_become_prefix_patterns() {
        let patterns = prefix_patterns("cargo test --all && git status").expect("patterns");
        assert_eq!(
            patterns,
            vec![
                vec!["cargo".to_owned(), "test".to_owned(), "--all".to_owned()],
                vec!["git".to_owned(), "status".to_owned()],
            ]
        );
    }

    #[test]
    fn redirected_or_expanded_commands_are_not_ruled() {
        assert!(prefix_patterns("echo hi > out.txt").is_none());
        assert!(prefix_patterns("ls $(pwd)").is_none());
        assert!(prefix_patterns("cat *").is_none());
    }

    #[test]
    fn rule_block_matches_the_macos_shape() {
        let block = rule_block(&["cargo".to_owned(), "test".to_owned()]);
        assert!(block.contains("prefix_rule("));
        assert!(block.contains("pattern = [\"cargo\", \"test\"]"));
        assert!(block.contains("decision = \"allow\""));
    }

    #[test]
    fn mcp_approval_uses_the_declared_server_table() {
        let config = "[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";
        let updated = config_with_mcp_tool_approval(config, "docs", "search").expect("updated");
        assert!(updated.contains(MCP_APPROVAL_COMMENT));
        assert!(updated.contains("[mcp_servers.docs.tools.search]"));
        assert!(updated.contains("approval_mode = \"approve\""));
    }

    #[test]
    fn mcp_approval_recovers_a_sanitised_server_name() {
        let config = "[mcp_servers.\"davinci-resolve\"]\ncommand = \"resolve-mcp\"\n";
        let updated =
            config_with_mcp_tool_approval(config, "davinci_resolve", "render").expect("updated");
        assert!(updated.contains("[mcp_servers.davinci-resolve.tools.render]"));
    }

    #[test]
    fn mcp_approval_refuses_an_undeclared_server() {
        let config = "[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";
        assert!(config_with_mcp_tool_approval(config, "other", "search").is_none());
        assert!(config_with_mcp_tool_approval("[mcp_servers.empty]\n", "empty", "x").is_none());
    }

    #[test]
    fn mcp_approval_updates_an_existing_table() {
        let config = "[mcp_servers.docs]\ncommand = \"docs-mcp\"\n\n\
                      [mcp_servers.docs.tools.search]\napproval_mode = \"prompt\"\n";
        let updated = config_with_mcp_tool_approval(config, "docs", "search").expect("updated");
        assert!(updated.contains("approval_mode = \"approve\""));
        assert!(!updated.contains("\"prompt\""));
        assert_eq!(updated.matches("approval_mode").count(), 1);
    }

    #[test]
    fn mcp_tool_names_split_on_the_double_underscore() {
        assert_eq!(
            mcp_tool_target(Some("mcp__docs__search")),
            Some(("docs".to_owned(), "search".to_owned()))
        );
        assert_eq!(mcp_tool_target(Some("Bash")), None);
        assert_eq!(mcp_tool_target(Some("mcp__noseparator")), None);
    }
}
