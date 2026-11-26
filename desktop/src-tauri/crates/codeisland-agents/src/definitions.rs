use std::sync::OnceLock;

use codeisland_domain::AgentKind;
use serde::{Deserialize, Serialize};

/// How an agent stores hook configuration on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookFormat {
    /// Claude Code style `hooks: { Event: [{ matcher, hooks: [{ type, command, timeout }] }] }`.
    ClaudeJson,
    /// Codex/Gemini/Copilot style `hooks: { Event: [{ matcher, hooks: [...] }] }`.
    EventMapJson,
    /// Cursor `hooks.json`, lowercase event names, flat `{ command }` entries.
    CursorJson,
    /// Copilot CLI `hooks.json`: flat `{ type, bash, timeoutSec }` entries and
    /// a top-level `version`.
    CopilotJson,
    /// OpenCode plugin file plus a `plugin` entry in `opencode.json`.
    OpenCodePlugin,
    /// Best-effort JSON: a `hooks` map with the same entry shape as Claude.
    GenericJson,
    /// TOML `[[hooks]]` array of tables (Kimi Code CLI).
    TomlBlock,
    /// YAML managed block (Hermes, Trae CLI).
    YamlBlock,
    /// A JS/TS plugin file installed at a known path (Pi, OpenClaw, MiMo).
    PluginJs,
    /// No safe generic installer; the UI must show manual instructions.
    Manual,
}

impl HookFormat {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "claude_json" | "claude" => Self::ClaudeJson,
            "event_map_json" | "event_map" => Self::EventMapJson,
            "cursor_json" | "cursor" => Self::CursorJson,
            "copilot_json" | "copilot" => Self::CopilotJson,
            "opencode_plugin" => Self::OpenCodePlugin,
            "generic_json" | "generic" => Self::GenericJson,
            "toml_block" | "toml" => Self::TomlBlock,
            "yaml_block" | "yaml" => Self::YamlBlock,
            "plugin_js" | "plugin" => Self::PluginJs,
            "manual" | "none" => Self::Manual,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeJson => "claude_json",
            Self::EventMapJson => "event_map_json",
            Self::CursorJson => "cursor_json",
            Self::CopilotJson => "copilot_json",
            Self::OpenCodePlugin => "opencode_plugin",
            Self::GenericJson => "generic_json",
            Self::TomlBlock => "toml_block",
            Self::YamlBlock => "yaml_block",
            Self::PluginJs => "plugin_js",
            Self::Manual => "manual",
        }
    }
}

/// The response schema a hook prints on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    Claude,
    Codex,
    Gemini,
    Cursor,
    Notification,
    Plain,
}

impl ResponseFormat {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            "gemini" => Self::Gemini,
            "cursor" => Self::Cursor,
            "notification" => Self::Notification,
            "plain" | "generic" => Self::Plain,
            _ => return None,
        })
    }
}

/// A single hook event an agent understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEventDef {
    pub name: String,
    /// Timeout in milliseconds. Installers convert this to the unit the agent
    /// expects (Claude and Codex use seconds, Gemini uses milliseconds).
    pub timeout_ms: u64,
    pub blocking: bool,
}

/// Everything CodeIsland knows about one agent integration.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub agent: AgentKind,
    pub display_name: String,
    pub executables: Vec<String>,
    /// Paths relative to `%USERPROFILE%` (or absolute). The first existing
    /// path wins; the first path is used when none exist.
    pub config_paths: Vec<String>,
    pub format: HookFormat,
    pub events: Vec<HookEventDef>,
    pub source: String,
    pub response: ResponseFormat,
    pub icon: String,
    pub docs_url: Option<String>,
    /// Top-level JSON key the hooks live under. Built-ins use `hooks`; a
    /// user-defined CLI may point at another key.
    pub config_key: String,
}

impl AgentDefinition {
    /// The marker token embedded in every command we write. It is stable
    /// across releases and lets uninstall/health checks recognize our entries
    /// even when the hook executable path changes.
    pub fn marker(&self) -> String {
        if self.agent == AgentKind::Unknown {
            // Custom CLIs all share `Unknown`; their own source keeps the
            // markers distinct.
            format!("codeisland-{}", self.source)
        } else {
            format!("codeisland-{}", self.agent.tag())
        }
    }

    /// Build a definition for a user-defined CLI from Settings.
    pub fn custom(
        name: impl Into<String>,
        source: impl Into<String>,
        config_path: impl Into<String>,
        format: HookFormat,
        config_key: impl Into<String>,
    ) -> Self {
        let source = source.into();
        Self {
            agent: AgentKind::Unknown,
            display_name: name.into(),
            executables: vec![source.clone()],
            config_paths: vec![config_path.into()],
            format,
            events: custom_events(format),
            source: source.clone(),
            response: ResponseFormat::Claude,
            icon: source,
            docs_url: None,
            config_key: config_key.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DefinitionsFile {
    agents: Vec<RawAgent>,
}

#[derive(Debug, Deserialize)]
struct RawAgent {
    id: String,
    display: String,
    source: String,
    #[serde(default)]
    executables: Vec<String>,
    #[serde(default)]
    config: Vec<String>,
    format: String,
    #[serde(default)]
    events: Vec<RawEvent>,
    response: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    docs: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawEvent {
    name: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    blocking: bool,
}

fn default_timeout_ms() -> u64 {
    5_000
}

/// Formats a user-defined CLI can be installed with, mirroring the macOS
/// Custom CLIs templates (Claude, Codex/Gemini, Cursor, Copilot).
pub fn custom_format_supported(format: HookFormat) -> bool {
    matches!(
        format,
        HookFormat::ClaudeJson
            | HookFormat::EventMapJson
            | HookFormat::CursorJson
            | HookFormat::CopilotJson
    )
}

/// The event set installed for a user-defined CLI, matching macOS
/// `ConfigInstaller.defaultEvents(for:)`.
fn custom_events(format: HookFormat) -> Vec<HookEventDef> {
    let table: &[(&str, u64, bool)] = match format {
        HookFormat::ClaudeJson => &[
            ("UserPromptSubmit", 5_000, true),
            ("PreToolUse", 5_000, false),
            ("PostToolUse", 5_000, true),
            ("SessionStart", 5_000, false),
            ("SessionEnd", 5_000, true),
            ("Stop", 5_000, true),
            ("SubagentStart", 5_000, true),
            ("SubagentStop", 5_000, true),
            ("Notification", 86_400_000, false),
            ("PreCompact", 5_000, true),
        ],
        HookFormat::EventMapJson => &[
            ("SessionStart", 5_000, false),
            ("SessionEnd", 5_000, true),
            ("UserPromptSubmit", 5_000, false),
            ("PreToolUse", 5_000, false),
            ("PostToolUse", 5_000, false),
            ("Stop", 5_000, false),
        ],
        HookFormat::CursorJson => &[
            ("beforeSubmitPrompt", 5_000, false),
            ("beforeShellExecution", 5_000, false),
            ("afterShellExecution", 5_000, false),
            ("beforeReadFile", 5_000, false),
            ("afterFileEdit", 5_000, false),
            ("beforeMCPExecution", 5_000, false),
            ("afterMCPExecution", 5_000, false),
            ("afterAgentThought", 5_000, false),
            ("afterAgentResponse", 5_000, false),
            ("stop", 5_000, false),
        ],
        HookFormat::CopilotJson => &[
            ("sessionStart", 5_000, false),
            ("sessionEnd", 5_000, true),
            ("userPromptSubmitted", 5_000, false),
            ("preToolUse", 5_000, false),
            ("postToolUse", 5_000, true),
            ("errorOccurred", 5_000, true),
        ],
        _ => &[],
    };
    table
        .iter()
        .map(|(name, timeout_ms, blocking)| HookEventDef {
            name: (*name).to_owned(),
            timeout_ms: *timeout_ms,
            blocking: *blocking,
        })
        .collect()
}

impl RawEvent {
    fn into_def(self) -> HookEventDef {
        let name = self.name;
        // Permission gates block the agent's tool call. Treat the common
        // spellings as blocking even when the table omits the flag.
        let blocking = self.blocking
            || matches!(
                name.as_str(),
                "PermissionRequest"
                    | "permission_request"
                    | "BeforeTool"
                    | "beforeShellExecution"
                    | "beforeMCPExecution"
            );
        HookEventDef {
            name,
            timeout_ms: self.timeout_ms,
            blocking,
        }
    }
}

/// All built-in definitions, loaded once from `definitions/agents.toml`.
pub fn definitions() -> &'static [AgentDefinition] {
    static DEFINITIONS: OnceLock<Vec<AgentDefinition>> = OnceLock::new();
    DEFINITIONS.get_or_init(load_definitions).as_slice()
}

fn load_definitions() -> Vec<AgentDefinition> {
    let raw: DefinitionsFile = toml::from_str(include_str!("../definitions/agents.toml"))
        .expect("definitions/agents.toml must be valid TOML");

    let mut out = Vec::with_capacity(raw.agents.len());
    for item in raw.agents {
        let agent = kind_for_source(&item.id);
        if agent == AgentKind::Unknown {
            tracing::warn!(id = %item.id, "ignoring unknown agent definition");
            continue;
        }
        let format = HookFormat::parse(&item.format).unwrap_or(HookFormat::Manual);
        let response = ResponseFormat::parse(&item.response).unwrap_or(ResponseFormat::Plain);
        let events = item.events.into_iter().map(RawEvent::into_def).collect();
        out.push(AgentDefinition {
            agent,
            display_name: item.display,
            executables: item.executables,
            config_paths: item.config,
            format,
            events,
            source: item.source,
            response,
            icon: item.icon.unwrap_or_else(|| agent.tag().to_owned()),
            docs_url: item.docs,
            config_key: "hooks".to_owned(),
        });
    }
    out
}

/// Find the definition for an agent kind.
///
/// Several products can share one [`AgentKind`] when the domain enum has no
/// variant of its own (Pi/Oh My Pi, AntiGravity/Google Antigravity, AiWork/AiWork
/// CLI, Trae CLI/Trae CLI Next). The first definition whose config path — or
/// the tool root that owns it — exists on this machine wins; otherwise the
/// first entry in the table is used.
pub fn definition(agent: AgentKind) -> Option<&'static AgentDefinition> {
    definition_for_agent_in(&crate::paths::Paths::current(), agent)
}

/// [`definition`] resolved against an explicit (possibly hermetic) home.
pub fn definition_for_agent_in(
    paths: &crate::paths::Paths,
    agent: AgentKind,
) -> Option<&'static AgentDefinition> {
    if agent == AgentKind::Unknown {
        return None;
    }
    let candidates: Vec<&'static AgentDefinition> = definitions()
        .iter()
        .filter(|def| def.agent == agent)
        .collect();
    let first = *candidates.first()?;
    if candidates.len() == 1 {
        return Some(first);
    }
    candidates
        .iter()
        .copied()
        .find(|def| definition_present(paths, def))
        .or(Some(first))
}

/// Every definition that maps to `agent` (more than one when the domain enum
/// cannot distinguish two products).
pub fn definitions_for_agent(agent: AgentKind) -> Vec<&'static AgentDefinition> {
    definitions()
        .iter()
        .filter(|def| def.agent == agent)
        .collect()
}

/// Whether the product a definition describes looks installed: an existing
/// config file, an existing config parent, or its tool root.
fn definition_present(paths: &crate::paths::Paths, def: &AgentDefinition) -> bool {
    // Trae CLI and Trae CLI Next share the `traecli` binary and the ~/.trae
    // root, so their config layout — not the tool root — tells them apart.
    if def.agent == AgentKind::TraeCli {
        let trae = paths.home().join(".trae");
        return if def.source == "traecli-next" {
            trae.join("cli").exists()
        } else {
            trae.join("traecli.yaml").exists() || (trae.exists() && !trae.join("cli").exists())
        };
    }
    if def
        .executables
        .iter()
        .any(|executable| crate::paths::executable_on_path(executable))
    {
        return true;
    }
    let configs = paths.config_paths(def);
    if configs.iter().any(|path| path.exists()) {
        return true;
    }
    if def.agent == AgentKind::Antigravity {
        // AntiGravity (Claude fork) and Google Antigravity share ~/.gemini with
        // Gemini CLI, so check each product's own root rather than a parent.
        return if def.source == "google-antigravity" {
            paths.home().join(".gemini").join("config").exists()
                || paths.home().join(".gemini").join("antigravity-cli").exists()
        } else {
            paths.home().join(".antigravity").exists()
        };
    }
    configs.iter().any(|path| {
        path.parent().is_some_and(|parent| parent.exists())
            || tool_root(paths, path).is_some_and(|root| root.exists())
    })
}

/// The directory that means "this tool is installed", derived from a config
/// path: `.claude/settings.json` -> `~/.claude`, `.config/opencode/opencode.json`
/// -> `~/.config/opencode`, `.omp/agent/extensions/codeisland.ts` -> `~/.omp`.
fn tool_root(paths: &crate::paths::Paths, path: &std::path::Path) -> Option<std::path::PathBuf> {
    let relative = path.strip_prefix(paths.home()).ok()?;
    let mut components = relative.components();
    let first = components.next()?;
    let mut root = paths.home().join(first.as_os_str());
    if matches!(
        first.as_os_str().to_str(),
        Some(".config" | ".local" | "AppData" | "Documents")
    ) {
        if let Some(second) = components.next() {
            root = root.join(second.as_os_str());
        }
    }
    Some(root)
}

/// Find the definition that owns a wire/source name.
///
/// Accepts both the exact `source` from the table and the aliases understood
/// by [`kind_for_source`], so `--source qoder-cli` and `--source factory`
/// resolve to the same definitions as `qoder` and `droid`.
pub fn definition_for_source(source: &str) -> Option<&'static AgentDefinition> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    if let Some(def) = definitions()
        .iter()
        .find(|def| def.source.eq_ignore_ascii_case(source))
    {
        return Some(def);
    }
    definition(kind_for_source(source))
}

/// Installed commands pass the per-agent marker as `--source` so the command
/// string itself is stable and recognizable. Strip it before matching.
fn without_marker(source: &str) -> &str {
    source
        .strip_prefix("codeisland-")
        .filter(|rest| !rest.is_empty())
        .unwrap_or(source)
}

/// Map a source/id spelling to an [`AgentKind`].
///
/// `AgentKind::from_tag` already covers the common aliases; this adds the
/// wire spellings used by the hook `--source` flag and by older macOS builds.
pub fn kind_for_source(source: &str) -> AgentKind {
    let source = without_marker(source.trim());
    match source.to_ascii_lowercase().as_str() {
        "claude-code" | "claude_code" => AgentKind::Claude,
        "qoder-cli" | "qoder_cli" => AgentKind::Qoder,
        "qoder-cn" | "qoder_cn" => AgentKind::QoderCn,
        "trae-cli-next" | "traecli-next" => AgentKind::TraeCli,
        "google-antigravity" | "agy" => AgentKind::Antigravity,
        "omp" | "oh-my-pi" => AgentKind::Pi,
        "dsh" => AgentKind::DeepSeek,
        "factory" => AgentKind::Factory,
        "aiwork-cli" => AgentKind::AiWork,
        other => AgentKind::from_tag(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_cover_the_required_agents() {
        let required = [
            "claude", "codex", "gemini", "cursor", "opencode", "qoder", "droid",
            "codebuddy", "copilot", "grok", "kimi", "cline", "pi", "omp", "qwen", "hermes",
            "antigravity", "google-antigravity", "kiro", "stepfun", "workbuddy", "openclaw",
            "zcode", "minimax", "deepseek", "aiwork", "aiwork-cli", "mimo", "trae", "traecn",
            "traecli", "traecli-next", "codybuddycn", "qodercn", "qoderwork",
        ];
        for source in required {
            let def = definition_for_source(source)
                .unwrap_or_else(|| panic!("missing definition for {source}"));
            assert!(!def.display_name.is_empty(), "{source} has no display name");
            assert!(!def.config_paths.is_empty(), "{source} has no config path");
        }
        assert!(definitions().len() >= 34);
    }

    #[test]
    fn source_aliases_resolve() {
        assert_eq!(
            definition_for_source("qoder-cli").map(|d| d.agent),
            Some(AgentKind::Qoder)
        );
        assert_eq!(
            definition_for_source("google-antigravity").map(|d| d.source.as_str()),
            Some("google-antigravity")
        );
        assert_eq!(
            definition_for_source("omp").map(|d| d.agent),
            Some(AgentKind::Pi)
        );
        assert_eq!(
            definition_for_source("aiwork-cli").map(|d| d.agent),
            Some(AgentKind::AiWork)
        );
        assert_eq!(
            definition_for_source("traecli-next").map(|d| d.agent),
            Some(AgentKind::TraeCli)
        );
        assert_eq!(
            definition_for_source("dsh").map(|d| d.agent),
            Some(AgentKind::DeepSeek)
        );
    }

    #[test]
    fn unknown_agent_has_no_definition() {
        assert!(definition(AgentKind::Unknown).is_none());
    }
}
