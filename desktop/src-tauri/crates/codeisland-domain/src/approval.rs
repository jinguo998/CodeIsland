use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{AgentEvent, AgentKind};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalAlwaysScope {
    #[default]
    Session,
    CodexRules { patterns: Vec<Vec<String>> },
    CodexMcpConfig,
}

/// These hooks apply the supplied rule themselves. An exception they still
/// forward after that (such as a protected path) needs a fresh decision.
pub fn source_keeps_always_rule(source: &str) -> bool {
    codeisland_swift::call(c"hook.keepsRule", serde_json::json!({
        "event": "PermissionRequest", "source": source,
    }))
}

/// Older desktop messages identify the provider only by its desktop enum.
pub fn agent_keeps_always_rule(agent: AgentKind) -> bool {
    source_keeps_always_rule(agent.hook_source())
}

pub fn command_for_event(event: &AgentEvent) -> Option<&str> {
    if event.agent != AgentKind::Codex || event.tool.as_deref() != Some("Bash") {
        return None;
    }
    let payload = event.payload.as_ref()?;
    payload.pointer("/tool_input/command")
        .or_else(|| payload.pointer("/toolInput/command"))?.as_str()
}

fn command_hook(script: &str) -> serde_json::Value {
    serde_json::json!({
        "event": "PermissionRequest", "source": "codex", "tool": "Bash",
        "payload": {"tool_input": {"command": script}},
    })
}

pub fn prefix_patterns(script: &str) -> Option<Vec<Vec<String>>> {
    codeisland_swift::call(c"codex.prefixes", command_hook(script))
}

pub fn commands_match(script: &str, patterns: &[Vec<String>]) -> bool {
    let mut args = command_hook(script);
    args["patterns"] = serde_json::json!(patterns);
    codeisland_swift::call(c"codex.matches", args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qoder_cli_and_ide_handle_persistent_rules_differently() {
        assert!(!source_keeps_always_rule("qoder"));
        assert!(source_keeps_always_rule("qoder-cli"));
        assert!(agent_keeps_always_rule(AgentKind::Qoder));
    }

    #[test]
    fn only_plain_commands_and_every_member_match() {
        let patterns = prefix_patterns("git add 'a file' &&\n cargo test | sort").unwrap();
        assert_eq!(patterns, [vec!["git", "add", "a file"], vec!["cargo", "test"], vec!["sort"]]);
        assert!(commands_match("cargo test --workspace || git add 'a file'", &patterns));
        assert!(!commands_match("cargo test && git push", &patterns));
        for script in ["a;b;c;d;e", "echo \"$HOME\"", "echo hi > out", "A=x make", "'git' add", "git status &&"] {
            assert!(prefix_patterns(script).is_none(), "{script}");
        }
    }
}
