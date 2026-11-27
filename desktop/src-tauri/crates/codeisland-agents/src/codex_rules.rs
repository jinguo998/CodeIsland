//! Bind a pending Windows permission request to its local Codex config root.
//! Swift evaluates the scope and writes the rule the user selected.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use codeisland_domain::{AgentEvent, AgentEventType, AgentKind, AppSettings, ApprovalAlwaysScope, PendingRequest};
use serde_json::{Value, json};

static RULE_WRITES: Mutex<()> = Mutex::new(());

pub fn stage_approval(
    event: &AgentEvent,
    settings: &AppSettings,
) -> (ApprovalAlwaysScope, Option<PathBuf>) {
    let session = (ApprovalAlwaysScope::Session, None);
    if event.agent != AgentKind::Codex || event.event_type != AgentEventType::PermissionRequest {
        return session;
    }
    let Some(payload) = event.payload.as_ref() else { return session };
    if payload.get("_remote_host_id").is_some() { return session; }
    let Some(config) = codex_config_for_event(event, settings) else { return session };
    let Some(home) = config.parent() else { return session };
    let scope = approved_scope(home, event.tool.as_deref(), payload);
    let config = (scope != ApprovalAlwaysScope::Session).then_some(config);
    (scope, config)
}

fn approved_scope(home: &Path, tool: Option<&str>, payload: &Value) -> ApprovalAlwaysScope {
    let Some(home) = home.to_str() else { return ApprovalAlwaysScope::Session };
    codeisland_swift::call(c"codex.scope", json!({
        "event": "PermissionRequest", "source": "codex", "home": home, "tool": tool, "payload": payload,
    }))
}

pub fn persist_approved_scope(request: &PendingRequest) {
    let Some(home) = request.approval_config.as_deref().and_then(Path::parent) else { return };
    let _write = RULE_WRITES.lock().unwrap_or_else(|error| error.into_inner());
    save_scope(home, request.tool.as_deref(), &request.always_scope);
}

fn save_scope(home: &Path, tool: Option<&str>, scope: &ApprovalAlwaysScope) -> bool {
    let Some(home) = home.to_str() else { return false };
    let patterns = match scope {
        ApprovalAlwaysScope::CodexRules { patterns } => Some(patterns),
        ApprovalAlwaysScope::CodexMcpConfig => None,
        ApprovalAlwaysScope::Session => return false,
    };
    codeisland_swift::call(c"codex.persist", json!({
        "event": "PermissionRequest", "source": "codex", "home": home, "tool": tool, "patterns": patterns,
    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::approval::prefix_patterns;
    use tempfile::tempdir;

    fn config_with_mcp_tool_approval(contents: &str, namespace: &str, tool: &str) -> Option<String> {
        let dir = tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(&config, contents).unwrap();
        let tool = format!("mcp__{namespace}__{tool}");
        let scope = approved_scope(dir.path(), Some(&tool), &serde_json::json!({}));
        assert_eq!(fs::read_to_string(&config).unwrap(), contents);
        if scope == ApprovalAlwaysScope::Session { return None; }
        assert_eq!(scope, ApprovalAlwaysScope::CodexMcpConfig);
        assert!(save_scope(dir.path(), Some(&tool), &scope));
        fs::read_to_string(config).ok()
    }

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
        let dir = tempdir().unwrap();
        let scope = ApprovalAlwaysScope::CodexRules { patterns: vec![vec!["cargo".into(), "test".into()]] };
        assert!(save_scope(dir.path(), Some("Bash"), &scope));
        assert!(save_scope(dir.path(), Some("Bash"), &scope));
        let block = fs::read_to_string(dir.path().join("rules/codeisland.rules")).unwrap();
        assert_eq!(block.matches("prefix_rule(").count(), 1);
        assert!(block.contains("prefix_rule("));
        assert!(block.contains("pattern = [\"cargo\", \"test\"]"));
        assert!(block.contains("decision = \"allow\""));
    }

    #[test]
    fn mcp_approval_uses_the_declared_server_table() {
        let config = "[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";
        let updated = config_with_mcp_tool_approval(config, "docs", "search").expect("updated");
        assert!(updated.contains(r#"# Added by CodeIsland when "Always Allow" is clicked for a Codex MCP tool."#));
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
    fn a_remote_request_stays_in_its_session_even_when_the_local_host_declares_its_tool() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("config.toml"), "[mcp_servers.docs]\ncommand = \"docs\"\n").unwrap();
        for tool in ["mcp__docs__search", "mcp__noseparator", "Bash"] {
            let raw = serde_json::json!({"_remote_host_id": "example", "tool_input":{"command":"cargo test"}});
            assert_eq!(approved_scope(dir.path(), Some(tool), &raw), ApprovalAlwaysScope::Session);
        }
    }
}
