//! Detect, install, repair and uninstall agent hooks.
//!
//! Every writer follows the same rules:
//! * preserve keys we do not own,
//! * remove only entries whose command carries our per-agent marker,
//! * be idempotent (a second install produces identical bytes),
//! * back up an existing file before mutating it,
//! * write through a temporary file and rename.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use codeisland_domain::AgentKind;
use serde_json::{Map, Value, json};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::AgentError;
use crate::definitions::{
    AgentDefinition, HookEventDef, HookFormat, definition_for_agent_in,
};
use crate::doctor::ToolStatus;
use crate::paths::Paths;

const YAML_START: &str = "# >>> codeisland managed >>>";
const YAML_END: &str = "# <<< codeisland managed <<<";

/// Public command for the Settings UI and diagnostics. The installer adds
/// `--event <name>` per hook entry; this is the base form.
pub fn hook_command(agent: AgentKind) -> String {
    let paths = Paths::current();
    definition_for_agent_in(&paths, agent)
        .map(|def| paths.hook_command(def))
        .unwrap_or_else(|| "codeisland-hook".to_owned())
}

pub fn install(agent: AgentKind) -> Result<ToolStatus, AgentError> {
    install_with(&Paths::current(), agent)
}

pub fn uninstall(agent: AgentKind) -> Result<ToolStatus, AgentError> {
    uninstall_with(&Paths::current(), agent)
}

pub fn repair(agent: AgentKind) -> Result<ToolStatus, AgentError> {
    // Installers are idempotent; a repair is an install plus a fresh status.
    install_with(&Paths::current(), agent)
}

pub fn detect(agent: AgentKind) -> ToolStatus {
    detect_with(&Paths::current(), agent)
}

/// Hermetic entry points for tests and the Settings preview.
#[doc(hidden)]
pub fn install_at(agent: AgentKind, home: &Path) -> Result<ToolStatus, AgentError> {
    install_with(&Paths::with_home(home.to_path_buf()), agent)
}

#[doc(hidden)]
pub fn uninstall_at(agent: AgentKind, home: &Path) -> Result<ToolStatus, AgentError> {
    uninstall_with(&Paths::with_home(home.to_path_buf()), agent)
}

#[doc(hidden)]
pub fn detect_at(agent: AgentKind, home: &Path) -> ToolStatus {
    detect_with(&Paths::with_home(home.to_path_buf()), agent)
}

// ── Install / uninstall dispatch ─────────────────────────────────────────────

pub fn install_with(paths: &Paths, agent: AgentKind) -> Result<ToolStatus, AgentError> {
    let selected = definition_for_agent_in(paths, agent)
        .ok_or_else(|| AgentError::Unsupported(agent.tag().to_owned()))?;
    if selected.format == HookFormat::Manual {
        return Err(AgentError::ManualInstall(manual_instructions(paths, selected)));
    }
    // Several definitions can share one AgentKind (Pi/Oh My Pi,
    // AntiGravity/Google Antigravity, …). Install every present one, not just
    // the definition the single-agent status resolves to.
    for def in crate::definitions::definitions_for_agent(agent) {
        if def.format == HookFormat::Manual || !agent_present(paths, def) {
            continue;
        }
        install_definition(paths, def)?;
    }
    Ok(detect_with(paths, agent))
}

/// Install one definition. The caller has already checked presence.
fn install_definition(paths: &Paths, def: &AgentDefinition) -> Result<(), AgentError> {
    if def.agent == AgentKind::Claude {
        let command_for = |event: &str| paths.hook_command_for_event(def, event);
        for (index, root) in paths.claude_config_dirs().into_iter().enumerate() {
            if index > 0 && !root.is_dir() {
                continue;
            }
            install_hooks_json(
                paths, def, &root.join("settings.json"), &command_for, HookJsonStyle::Claude,
            )?;
        }
        return Ok(());
    }
    let path = paths
        .primary_config(def)
        .ok_or_else(|| AgentError::Config(format!("{} has no config path", def.display_name)))?;
    let command_for = |event: &str| paths.hook_command_for_event(def, event);
    let codex_config = if def.agent == AgentKind::Codex {
        let config = path.with_file_name("config.toml");
        let current = read_to_string_opt(&config)?.unwrap_or_default();
        let updated = codex_enable_hooks(&config, &current)?;
        Some((config, updated))
    } else {
        None
    };

    match def.format {
        HookFormat::ClaudeJson => {
            let style = if def.agent == AgentKind::MiniMax {
                HookJsonStyle::MiniMax
            } else {
                HookJsonStyle::Claude
            };
            install_hooks_json(paths, def, &path, &command_for, style)?
        }
        HookFormat::EventMapJson => install_hooks_json(
            paths,
            def,
            &path,
            &command_for,
            HookJsonStyle::EventMap,
        )?,
        HookFormat::CursorJson => install_cursor_json(paths, def, &path, &command_for)?,
        HookFormat::CopilotJson => install_copilot_json(paths, def, &path, &command_for)?,
        HookFormat::OpenCodePlugin => install_opencode(paths, def, &path)?,
        HookFormat::GenericJson => install_generic_json(paths, def, &path, &command_for)?,
        HookFormat::TomlBlock => install_kimi(paths, def, &path, &command_for)?,
        HookFormat::YamlBlock => install_yaml(paths, def, &path, &command_for)?,
        HookFormat::PluginJs => install_plugin_js(paths, def, &path)?,
        HookFormat::Manual => unreachable!("manual format is rejected by the caller"),
    }

    if let Some((config, updated)) = codex_config {
        write_if_changed(paths, def.agent, &config, &updated)?;
    }
    if def.agent == AgentKind::MiniMax {
        write_minimax_manifest(paths, def, &path)?;
    }
    if def.agent == AgentKind::OpenClaw {
        install_openclaw_pack(paths, def, &path)?;
    }
    Ok(())
}

/// Enable the Codex feature while preserving the user's TOML layout.
fn codex_enable_hooks(path: &Path, existing: &str) -> Result<String, AgentError> {
    // Reject a malformed user document before the source-preserving edit.
    existing.parse::<DocumentMut>().map_err(|error| AgentError::Config(format!("{}: {error}", path.display())))?;
    codeisland_swift::call::<Option<String>>(c"codex.enableHooks", existing)
        .ok_or_else(|| AgentError::Config(format!("{}: unsupported Codex hooks feature layout", path.display())))
}

pub fn uninstall_with(paths: &Paths, agent: AgentKind) -> Result<ToolStatus, AgentError> {
    let selected = definition_for_agent_in(paths, agent)
        .ok_or_else(|| AgentError::Unsupported(agent.tag().to_owned()))?;
    if selected.format == HookFormat::Manual {
        return Ok(detect_with(paths, agent));
    }
    // Clean every definition of this kind, even one that is no longer present:
    // an uninstall must not leave a stale sibling config behind.
    for def in crate::definitions::definitions_for_agent(agent) {
        if def.format == HookFormat::Manual {
            continue;
        }
        uninstall_definition(paths, def)?;
    }
    Ok(detect_with(paths, agent))
}

fn uninstall_definition(paths: &Paths, def: &AgentDefinition) -> Result<(), AgentError> {
    if def.agent == AgentKind::Claude {
        for root in paths.claude_config_dirs() {
            uninstall_hooks_json(paths, def, &root.join("settings.json"), HookJsonStyle::Claude)?;
        }
        return Ok(());
    }
    let Some(path) = paths.primary_config(def) else {
        return Ok(());
    };

    match def.format {
        HookFormat::ClaudeJson => uninstall_hooks_json(paths, def, &path, HookJsonStyle::Claude)?,
        HookFormat::EventMapJson => {
            uninstall_hooks_json(paths, def, &path, HookJsonStyle::EventMap)?
        }
        HookFormat::CursorJson => uninstall_cursor_json(paths, def, &path)?,
        HookFormat::CopilotJson => uninstall_copilot_json(paths, def, &path)?,
        HookFormat::OpenCodePlugin => uninstall_opencode(paths, def, &path)?,
        HookFormat::GenericJson => uninstall_generic_json(paths, def, &path)?,
        HookFormat::TomlBlock => uninstall_kimi(paths, def, &path)?,
        HookFormat::YamlBlock => uninstall_yaml(paths, def, &path)?,
        HookFormat::PluginJs => uninstall_plugin_js(paths, def, &path)?,
        HookFormat::Manual => {}
    }
    if def.agent == AgentKind::MiniMax {
        remove_minimax_manifest(paths, def, &path)?;
    }
    if def.agent == AgentKind::OpenClaw {
        uninstall_openclaw_pack(paths, def, &path)?;
    }
    Ok(())
}

// ── JSON: Claude and event-map formats ──────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookJsonStyle {
    Claude,
    EventMap,
    /// Best-effort JSON for agents that accept a Claude-like wrapper but not
    /// Claude's `"*"` matcher.
    Generic,
    /// MiniMax's mcode accepts an omitted matcher but validates one when it is
    /// present, so its plugin pack gets a matcher-less entry.
    MiniMax,
}

fn install_hooks_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
    command_for: &dyn Fn(&str) -> String,
    style: HookJsonStyle,
) -> Result<(), AgentError> {
    let existing = read_to_string_opt(path)?.unwrap_or_default();
    let merged = merge_hooks_json(&existing, def, command_for, style)?;
    write_if_changed(paths, def.agent, path, &merged)?;
    Ok(())
}

fn uninstall_hooks_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
    style: HookJsonStyle,
) -> Result<(), AgentError> {
    let Some(existing) = read_to_string_opt(path)? else {
        return Ok(());
    };
    let cleaned = remove_hooks_json(&existing, def, style)?;
    write_if_changed(paths, def.agent, path, &cleaned)?;
    Ok(())
}

/// Merge our entries into a `hooks` object.
fn merge_hooks_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
    style: HookJsonStyle,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    let key = def.config_key.clone();
    {
        let hooks = root
            .entry(key.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| AgentError::Config(format!("{key} is not an object")))?;
        clean_hooks_map(hooks, def, false);
        for event in &def.events {
            let entry = make_hook_entry(style, def, event, command_for(&event.name));
            let array = hooks
                .entry(event.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            array
                .as_array_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("{key}.{} is not an array", event.name))
                })?
                .push(entry);
        }
    }
    if root
        .get(&key)
        .and_then(|value| value.as_object())
        .is_some_and(|hooks| hooks.is_empty())
    {
        root.remove(&key);
    }
    to_json_text(&Value::Object(root))
}

/// Remove our entries from a `hooks` object.
fn remove_hooks_json(
    existing: &str,
    def: &AgentDefinition,
    _style: HookJsonStyle,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    let key = def.config_key.clone();
    if let Some(hooks) = root.get_mut(&key).and_then(|value| value.as_object_mut()) {
        clean_hooks_map(hooks, def, true);
        let empty = hooks.is_empty();
        if empty {
            root.remove(&key);
        }
    }
    to_json_text(&Value::Object(root))
}

pub fn merge_claude_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    merge_hooks_json(existing, def, command_for, HookJsonStyle::Claude)
}

pub fn remove_claude_json(
    existing: &str,
    def: &AgentDefinition,
) -> Result<String, AgentError> {
    remove_hooks_json(existing, def, HookJsonStyle::Claude)
}

pub fn merge_event_map_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    merge_hooks_json(existing, def, command_for, HookJsonStyle::EventMap)
}

pub fn remove_event_map_json(
    existing: &str,
    def: &AgentDefinition,
) -> Result<String, AgentError> {
    remove_hooks_json(existing, def, HookJsonStyle::EventMap)
}

fn make_hook_entry(
    style: HookJsonStyle,
    def: &AgentDefinition,
    event: &HookEventDef,
    command: String,
) -> Value {
    match style {
        HookJsonStyle::Claude => json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                // Qwen Code declares milliseconds in its Claude-shaped file;
                // every other Claude fork uses seconds.
                "timeout": timeout_value(def, event)
            }]
        }),
        HookJsonStyle::EventMap => json!({
            // Codex/Gemini reject a matcher on nested entries.
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": timeout_value(def, event)
            }]
        }),
        HookJsonStyle::Generic => json!({
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": timeout_value(def, event)
            }]
        }),
        HookJsonStyle::MiniMax => json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": timeout_secs(event)
            }]
        }),
    }
}

// ── Cursor ──────────────────────────────────────────────────────────────────

fn install_cursor_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
    command_for: &dyn Fn(&str) -> String,
) -> Result<(), AgentError> {
    let existing = read_to_string_opt(path)?.unwrap_or_default();
    let merged = merge_cursor_json(&existing, def, command_for)?;
    write_if_changed(paths, def.agent, path, &merged)?;
    Ok(())
}

fn uninstall_cursor_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
) -> Result<(), AgentError> {
    let Some(existing) = read_to_string_opt(path)? else {
        return Ok(());
    };
    let cleaned = remove_cursor_json(&existing, def)?;
    write_if_changed(paths, def.agent, path, &cleaned)?;
    Ok(())
}

pub fn merge_cursor_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    root.entry("version").or_insert(json!(1));
    let key = def.config_key.clone();
    {
        let hooks = root
            .entry(key.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| AgentError::Config(format!("{key} is not an object")))?;
        clean_hooks_map(hooks, def, false);
        for event in &def.events {
            let entry = json!({
                "command": command_for(&event.name),
                "timeout": timeout_secs(event)
            });
            hooks
                .entry(event.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("{key}.{} is not an array", event.name))
                })?
                .push(entry);
        }
    }
    to_json_text(&Value::Object(root))
}

pub fn remove_cursor_json(
    existing: &str,
    def: &AgentDefinition,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    let key = def.config_key.clone();
    if let Some(hooks) = root.get_mut(&key).and_then(|value| value.as_object_mut()) {
        clean_hooks_map(hooks, def, true);
        let empty = hooks.is_empty();
        if empty {
            root.remove(&key);
        }
    }
    to_json_text(&Value::Object(root))
}

// ── Copilot ─────────────────────────────────────────────────────────────────

fn install_copilot_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
    command_for: &dyn Fn(&str) -> String,
) -> Result<(), AgentError> {
    let existing = read_to_string_opt(path)?.unwrap_or_default();
    let merged = merge_copilot_json(&existing, def, command_for)?;
    write_if_changed(paths, def.agent, path, &merged)?;
    Ok(())
}

fn uninstall_copilot_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
) -> Result<(), AgentError> {
    let Some(existing) = read_to_string_opt(path)? else {
        return Ok(());
    };
    let cleaned = remove_copilot_json(&existing, def)?;
    write_if_changed(paths, def.agent, path, &cleaned)?;
    Ok(())
}

/// Copilot CLI expects a top-level `version` and flat
/// `{ type, bash, timeoutSec }` entries — not the nested
/// `{ matcher, hooks: [...] }` shape Claude uses.
pub fn merge_copilot_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    root.entry("version").or_insert(json!(1));
    let key = def.config_key.clone();
    {
        let hooks = root
            .entry(key.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| AgentError::Config(format!("{key} is not an object")))?;
        clean_hooks_map(hooks, def, false);
        for event in &def.events {
            let entry = json!({
                "type": "command",
                "bash": command_for(&event.name),
                "timeoutSec": timeout_secs(event)
            });
            hooks
                .entry(event.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("{key}.{} is not an array", event.name))
                })?
                .push(entry);
        }
    }
    to_json_text(&Value::Object(root))
}

pub fn remove_copilot_json(
    existing: &str,
    def: &AgentDefinition,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    let key = def.config_key.clone();
    if let Some(hooks) = root.get_mut(&key).and_then(|value| value.as_object_mut()) {
        clean_hooks_map(hooks, def, true);
        let empty = hooks.is_empty();
        if empty {
            root.remove(&key);
        }
    }
    to_json_text(&Value::Object(root))
}

// ── Generic JSON (ZCode, Antigravity, Kiro, Trae) ───────────────────────────

fn install_generic_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
    command_for: &dyn Fn(&str) -> String,
) -> Result<(), AgentError> {
    let existing = read_to_string_opt(path)?.unwrap_or_default();
    let merged = merge_generic_json(&existing, def, command_for)?;
    write_if_changed(paths, def.agent, path, &merged)?;
    Ok(())
}

fn uninstall_generic_json(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
) -> Result<(), AgentError> {
    let Some(existing) = read_to_string_opt(path)? else {
        return Ok(());
    };
    let cleaned = remove_generic_json(&existing, def)?;
    write_if_changed(paths, def.agent, path, &cleaned)?;
    Ok(())
}

pub fn merge_generic_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    match def.agent {
        AgentKind::ZCode => merge_zcode_json(existing, def, command_for),
        AgentKind::Antigravity => merge_antigravity_json(existing, def, command_for),
        AgentKind::Kiro => merge_kiro_json(existing, def, command_for),
        AgentKind::Trae | AgentKind::TraeCn => merge_trae_json(existing, def, command_for),
        _ => merge_hooks_json(existing, def, command_for, HookJsonStyle::Generic),
    }
}

pub fn remove_generic_json(
    existing: &str,
    def: &AgentDefinition,
) -> Result<String, AgentError> {
    match def.agent {
        AgentKind::ZCode => remove_zcode_json(existing, def),
        AgentKind::Antigravity => remove_antigravity_json(existing, def),
        AgentKind::Kiro => remove_kiro_json(existing, def),
        AgentKind::Trae | AgentKind::TraeCn => remove_trae_json(existing, def),
        _ => remove_hooks_json(existing, def, HookJsonStyle::Claude),
    }
}

fn merge_zcode_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    {
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| AgentError::Config("hooks is not an object".to_owned()))?;
        hooks.entry("enabled").or_insert(json!(true));
        let events = hooks
            .entry("events")
            .or_insert_with(|| Value::Object(Map::new()));
        let events = events
            .as_object_mut()
            .ok_or_else(|| AgentError::Config("hooks.events is not an object".to_owned()))?;
        clean_hooks_map(events, def, false);
        for event in &def.events {
            let mut hook = Map::new();
            hook.insert("type".to_owned(), json!("command"));
            hook.insert("command".to_owned(), json!(command_for(&event.name)));
            // ZCode's per-hook default is 60s. Only write a timeout when the
            // event needs longer (the blocking approval).
            if timeout_secs(event) > 60 {
                hook.insert("timeout".to_owned(), json!(timeout_secs(event)));
            }
            events
                .entry(event.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("hooks.events.{} is not an array", event.name))
                })?
                .push(json!({ "hooks": [Value::Object(hook)] }));
        }
    }
    to_json_text(&Value::Object(root))
}

fn remove_zcode_json(existing: &str, def: &AgentDefinition) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    if let Some(hooks) = root.get_mut("hooks").and_then(|value| value.as_object_mut()) {
        if let Some(events) = hooks.get_mut("events").and_then(|value| value.as_object_mut()) {
            clean_hooks_map(events, def, true);
            let empty = events.is_empty();
            if empty {
                hooks.remove("events");
            }
        }
    }
    to_json_text(&Value::Object(root))
}

fn merge_antigravity_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    {
        let wrapper = root
            .entry("codeisland")
            .or_insert_with(|| Value::Object(Map::new()));
        let wrapper = wrapper
            .as_object_mut()
            .ok_or_else(|| AgentError::Config("codeisland is not an object".to_owned()))?;
        clean_hooks_map(wrapper, def, false);
        for event in &def.events {
            // Antigravity tool events take `{matcher, hooks:[…]}`; model
            // events (Stop) take a flat `{type, command}` handler.
            let entry = if matches!(event.name.as_str(), "PreToolUse" | "PostToolUse") {
                json!({
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": command_for(&event.name),
                        "timeout": timeout_secs(event)
                    }]
                })
            } else {
                json!({
                    "type": "command",
                    "command": command_for(&event.name),
                    "timeout": timeout_secs(event)
                })
            };
            wrapper
                .entry(event.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("codeisland.{} is not an array", event.name))
                })?
                .push(entry);
        }
    }
    to_json_text(&Value::Object(root))
}

fn remove_antigravity_json(existing: &str, def: &AgentDefinition) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    if let Some(wrapper) = root.get_mut("codeisland").and_then(|value| value.as_object_mut()) {
        clean_hooks_map(wrapper, def, true);
        let empty = wrapper.is_empty();
        if empty {
            root.remove("codeisland");
        }
    }
    to_json_text(&Value::Object(root))
}

fn merge_kiro_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    root.entry("name").or_insert(json!("codeisland"));
    {
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| AgentError::Config("hooks is not an object".to_owned()))?;
        clean_hooks_map(hooks, def, false);
        for event in &def.events {
            let entry = json!({
                "command": command_for(&event.name),
                "matcher": "*",
                "timeout_ms": event.timeout_ms
            });
            hooks
                .entry(event.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("hooks.{} is not an array", event.name))
                })?
                .push(entry);
        }
    }
    to_json_text(&Value::Object(root))
}

fn remove_kiro_json(existing: &str, def: &AgentDefinition) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    if let Some(hooks) = root.get_mut("hooks").and_then(|value| value.as_object_mut()) {
        clean_hooks_map(hooks, def, true);
        let empty = hooks.is_empty();
        if empty {
            root.remove("hooks");
        }
    }
    to_json_text(&Value::Object(root))
}

fn merge_trae_json(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    root.entry("version").or_insert(json!(1));
    {
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| AgentError::Config("hooks is not an object".to_owned()))?;
        clean_hooks_map(hooks, def, false);
        for event in &def.events {
            let entry = json!({
                "matcher": "*",
                "loop_limit": 5,
                "hooks": [{
                    "type": "command",
                    "command": command_for(&event.name),
                    "timeout": timeout_secs(event)
                }]
            });
            hooks
                .entry(event.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("hooks.{} is not an array", event.name))
                })?
                .push(entry);
        }
    }
    to_json_text(&Value::Object(root))
}

fn remove_trae_json(existing: &str, def: &AgentDefinition) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    if let Some(hooks) = root.get_mut("hooks").and_then(|value| value.as_object_mut()) {
        clean_hooks_map(hooks, def, true);
        let empty = hooks.is_empty();
        if empty {
            root.remove("hooks");
        }
    }
    to_json_text(&Value::Object(root))
}

// ── OpenCode plugin ─────────────────────────────────────────────────────────

fn install_opencode(
    paths: &Paths,
    def: &AgentDefinition,
    config_path: &Path,
) -> Result<(), AgentError> {
    let Some(dir) = config_path.parent() else {
        return Err(AgentError::Config("opencode config has no parent".to_owned()));
    };
    let plugin_path = dir.join("plugins").join("codeisland.js");
    let source = crate::plugins::plugin_source(def, &paths.hook_program_path());
    write_if_changed(paths, def.agent, &plugin_path, &source)?;

    let existing = read_to_string_opt(config_path)?.unwrap_or_default();
    let merged = merge_opencode_config(&existing, &plugin_path)?;
    write_if_changed(paths, def.agent, config_path, &merged)?;
    Ok(())
}

fn uninstall_opencode(
    paths: &Paths,
    def: &AgentDefinition,
    config_path: &Path,
) -> Result<(), AgentError> {
    if let Some(dir) = config_path.parent() {
        let plugin_path = dir.join("plugins").join("codeisland.js");
        remove_plugin_file(def, &plugin_path)?;
    }
    if let Some(existing) = read_to_string_opt(config_path)? {
        let cleaned = remove_opencode_config(&existing)?;
        write_if_changed(paths, def.agent, config_path, &cleaned)?;
    }
    Ok(())
}

fn merge_opencode_config(
    existing: &str,
    plugin_path: &Path,
) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    let reference = file_url(plugin_path);
    let plugins = root
        .entry("plugin")
        .or_insert_with(|| Value::Array(Vec::new()));
    let plugins = plugins
        .as_array_mut()
        .ok_or_else(|| AgentError::Config("plugin is not an array".to_owned()))?;
    plugins.retain(|entry| {
        !entry
            .as_str()
            .is_some_and(|text| text.contains("codeisland"))
    });
    plugins.push(json!(reference));
    to_json_text(&Value::Object(root))
}

fn remove_opencode_config(existing: &str) -> Result<String, AgentError> {
    let mut root = parse_object(existing)?;
    if let Some(plugins) = root.get_mut("plugin").and_then(|value| value.as_array_mut()) {
        plugins.retain(|entry| {
            !entry
                .as_str()
                .is_some_and(|text| text.contains("codeisland"))
        });
        let empty = plugins.is_empty();
        if empty {
            root.remove("plugin");
        }
    }
    to_json_text(&Value::Object(root))
}

// ── MiniMax plugin manifest ─────────────────────────────────────────────────

/// Marker written into the manifest so uninstall only removes OUR pack.
const MINIMAX_MANIFEST_MARKER: &str = "CodeIsland MiniMax Code plugin";

/// `<plugin>/hooks/hooks.json` -> `<plugin>/.claude-plugin/plugin.json`.
fn minimax_manifest_path(hooks_path: &Path) -> Option<PathBuf> {
    let hooks_dir = hooks_path.parent()?;
    let plugin_dir = hooks_dir.parent()?;
    Some(plugin_dir.join(".claude-plugin").join("plugin.json"))
}

/// mcode discovers local plugins by scanning `~/.minimax/plugins` for a
/// directory with a manifest; the hooks file alone is never loaded.
fn write_minimax_manifest(
    paths: &Paths,
    def: &AgentDefinition,
    hooks_path: &Path,
) -> Result<(), AgentError> {
    let Some(manifest) = minimax_manifest_path(hooks_path) else {
        return Ok(());
    };
    if let Some(existing) = read_to_string_opt(&manifest)? {
        // A plugin someone else put at plugins/codeisland is not ours to touch.
        if !existing.contains(MINIMAX_MANIFEST_MARKER) {
            return Ok(());
        }
    }
    let document = json!({
        "name": "codeisland",
        "displayName": "CodeIsland",
        "version": "1.0.0",
        "description": MINIMAX_MANIFEST_MARKER,
        "hooks": ["hooks/hooks.json"]
    });
    let text = to_json_text(&document)?;
    write_if_changed(paths, def.agent, &manifest, &text)?;
    Ok(())
}

fn remove_minimax_manifest(
    _paths: &Paths,
    _def: &AgentDefinition,
    hooks_path: &Path,
) -> Result<(), AgentError> {
    let Some(manifest) = minimax_manifest_path(hooks_path) else {
        return Ok(());
    };
    let Some(existing) = read_to_string_opt(&manifest)? else {
        return Ok(());
    };
    if !existing.contains(MINIMAX_MANIFEST_MARKER) {
        return Ok(());
    }
    if manifest.exists() {
        fs::remove_file(&manifest)?;
    }
    // Leave no empty .claude-plugin directory behind.
    if let Some(dir) = manifest.parent() {
        if dir.read_dir().is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(dir);
        }
    }
    Ok(())
}

// ── OpenClaw plugin pack ────────────────────────────────────────────────────

/// Marker written into the pack so uninstall only removes OUR files.
const OPENCLAW_PACK_MARKER: &str = "CodeIsland OpenClaw plugin";

/// OpenClaw loads a plugin pack (index.ts + manifest) and only activates it
/// when `openclaw.json` registers the pack directory. Mirrors macOS
/// `installOpenclawPlugin`: the files are written first; a JSON5 config we
/// cannot parse is left untouched rather than clobbered.
fn install_openclaw_pack(
    paths: &Paths,
    def: &AgentDefinition,
    index_path: &Path,
) -> Result<(), AgentError> {
    let Some(plugin_dir) = index_path.parent() else {
        return Ok(());
    };
    let manifest = json!({
        "id": "codeisland",
        "name": "CodeIsland",
        "description": OPENCLAW_PACK_MARKER,
        "version": "1.0.0",
        "configSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        },
        "activation": { "onStartup": true }
    });
    let package = json!({
        "name": "codeisland",
        "version": "1.0.0",
        "description": OPENCLAW_PACK_MARKER,
        "private": true,
        "type": "module",
        "openclaw": { "extensions": ["./index.ts"] }
    });
    for (name, document) in [("openclaw.plugin.json", manifest), ("package.json", package)] {
        let path = plugin_dir.join(name);
        if let Some(existing) = read_to_string_opt(&path)? {
            if !existing.trim().is_empty() && !existing.contains(OPENCLAW_PACK_MARKER) {
                // Someone else's pack file: never overwrite it.
                continue;
            }
        }
        let text = to_json_text(&document)?;
        write_if_changed(paths, def.agent, &path, &text)?;
    }
    register_openclaw_plugin(paths, def, plugin_dir)
}

fn openclaw_config_path(paths: &Paths) -> PathBuf {
    paths.home().join(".openclaw").join("openclaw.json")
}

/// Add `plugins.load.paths += [plugin_dir]` and
/// `plugins.entries.codeisland.enabled = true`.
fn register_openclaw_plugin(
    paths: &Paths,
    def: &AgentDefinition,
    plugin_dir: &Path,
) -> Result<(), AgentError> {
    let config_path = openclaw_config_path(paths);
    let existing = read_to_string_opt(&config_path)?.unwrap_or_default();
    let mut root = if existing.trim().is_empty() {
        Map::new()
    } else {
        match serde_json::from_str::<Value>(&existing) {
            Ok(Value::Object(map)) => map,
            _ => {
                // openclaw.json is JSON5; comments/trailing commas make it
                // unparseable here. Stage the pack and leave the user's file.
                tracing::warn!(
                    path = %config_path.display(),
                    "openclaw.json is not plain JSON; register the plugin manually"
                );
                return Ok(());
            }
        }
    };

    let plugins = root
        .entry("plugins")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(plugins) = plugins.as_object_mut() else {
        return Ok(());
    };

    let load = plugins
        .entry("load")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(load) = load.as_object_mut() {
        let entries = load
            .entry("paths")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(entries) = entries.as_array_mut() {
            let dir = plugin_dir.display().to_string();
            if !entries
                .iter()
                .any(|entry| entry.as_str() == Some(dir.as_str()))
            {
                entries.push(json!(dir));
            }
        }
    }

    let entries = plugins
        .entry("entries")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(entries) = entries.as_object_mut() {
        let entry = entries
            .entry("codeisland")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(entry) = entry.as_object_mut() {
            entry.insert("enabled".to_owned(), json!(true));
        }
    }

    let text = to_json_text(&Value::Object(root))?;
    write_if_changed(paths, def.agent, &config_path, &text)?;
    Ok(())
}

fn uninstall_openclaw_pack(
    paths: &Paths,
    def: &AgentDefinition,
    index_path: &Path,
) -> Result<(), AgentError> {
    let Some(plugin_dir) = index_path.parent() else {
        return Ok(());
    };
    // Remove only our pack, never other extensions in ~/.openclaw. The index
    // may already have been removed by the plugin-file uninstaller, so the
    // manifest is the fallback identity check.
    let ours = read_to_string_opt(index_path)?
        .is_some_and(|text| text.contains(OPENCLAW_PACK_MARKER) || text.contains(&def.marker()))
        || read_to_string_opt(&plugin_dir.join("openclaw.plugin.json"))?
            .is_some_and(|text| text.contains(OPENCLAW_PACK_MARKER));
    if ours {
        let _ = fs::remove_dir_all(plugin_dir);
    }
    unregister_openclaw_plugin(paths, def, plugin_dir)
}

fn unregister_openclaw_plugin(
    paths: &Paths,
    def: &AgentDefinition,
    plugin_dir: &Path,
) -> Result<(), AgentError> {
    let config_path = openclaw_config_path(paths);
    let Some(existing) = read_to_string_opt(&config_path)? else {
        return Ok(());
    };
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(&existing) else {
        return Ok(());
    };
    let Some(plugins) = root.get_mut("plugins").and_then(Value::as_object_mut) else {
        return Ok(());
    };

    let mut remove_load = false;
    if let Some(load) = plugins.get_mut("load").and_then(Value::as_object_mut) {
        let dir = plugin_dir.display().to_string();
        let mut remove_paths = false;
        if let Some(entries) = load.get_mut("paths").and_then(Value::as_array_mut) {
            entries.retain(|entry| entry.as_str() != Some(dir.as_str()));
            remove_paths = entries.is_empty();
        }
        if remove_paths {
            load.remove("paths");
        }
        remove_load = load.is_empty();
    }
    if remove_load {
        plugins.remove("load");
    }

    let mut remove_entries = false;
    if let Some(entries) = plugins.get_mut("entries").and_then(Value::as_object_mut) {
        entries.remove("codeisland");
        remove_entries = entries.is_empty();
    }
    if remove_entries {
        plugins.remove("entries");
    }
    if plugins.is_empty() {
        root.remove("plugins");
    }

    let text = to_json_text(&Value::Object(root))?;
    write_if_changed(paths, def.agent, &config_path, &text)?;
    Ok(())
}

// ── Kimi TOML ───────────────────────────────────────────────────────────────

fn install_kimi(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
    command_for: &dyn Fn(&str) -> String,
) -> Result<(), AgentError> {
    let existing = read_to_string_opt(path)?.unwrap_or_default();
    let merged = merge_kimi_toml(&existing, def, command_for)?;
    write_if_changed(paths, def.agent, path, &merged)?;
    Ok(())
}

fn uninstall_kimi(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
) -> Result<(), AgentError> {
    let Some(existing) = read_to_string_opt(path)? else {
        return Ok(());
    };
    let cleaned = remove_kimi_toml(&existing, def)?;
    write_if_changed(paths, def.agent, path, &cleaned)?;
    Ok(())
}

pub fn merge_kimi_toml(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let mut doc = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .parse::<DocumentMut>()
            .map_err(|error| AgentError::Config(format!("invalid TOML: {error}")))?
    };

    let mut tables = ArrayOfTables::new();
    if let Some(item) = doc.get("hooks") {
        let existing_tables = item
            .as_array_of_tables()
            .ok_or_else(|| AgentError::Config("hooks is not an array of tables".to_owned()))?;
        for table in existing_tables.iter() {
            if !toml_table_is_ours(table, def) {
                tables.push(table.clone());
            }
        }
    }
    for event in &def.events {
        let mut table = Table::new();
        table.insert("event", value(event.name.clone()));
        table.insert("command", value(command_for(&event.name)));
        table.insert("timeout", value(timeout_secs(event) as i64));
        if matches!(
            event.name.as_str(),
            "PreToolUse" | "PostToolUse" | "PostToolUseFailure"
        ) {
            table.insert("matcher", value(".*"));
        }
        tables.push(table);
    }
    doc.insert("hooks", Item::ArrayOfTables(tables));
    Ok(ensure_newline(doc.to_string()))
}

pub fn remove_kimi_toml(
    existing: &str,
    def: &AgentDefinition,
) -> Result<String, AgentError> {
    if existing.trim().is_empty() {
        return Ok(existing.to_owned());
    }
    let mut doc = existing
        .parse::<DocumentMut>()
        .map_err(|error| AgentError::Config(format!("invalid TOML: {error}")))?;
    let Some(item) = doc.get("hooks") else {
        return Ok(existing.to_owned());
    };
    let Some(existing_tables) = item.as_array_of_tables() else {
        return Ok(existing.to_owned());
    };

    let mut kept = ArrayOfTables::new();
    let mut removed = false;
    for table in existing_tables.iter() {
        if toml_table_is_ours(table, def) {
            removed = true;
        } else {
            kept.push(table.clone());
        }
    }
    if !removed {
        return Ok(existing.to_owned());
    }
    if kept.is_empty() {
        doc.remove("hooks");
    } else {
        doc.insert("hooks", Item::ArrayOfTables(kept));
    }
    Ok(ensure_newline(doc.to_string()))
}

fn toml_table_is_ours(table: &Table, def: &AgentDefinition) -> bool {
    table
        .get("command")
        .and_then(Item::as_str)
        .is_some_and(|command| command_is_ours(command, def))
}

// ── YAML managed block (Hermes, Trae CLI) ───────────────────────────────────

fn install_yaml(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
    command_for: &dyn Fn(&str) -> String,
) -> Result<(), AgentError> {
    let existing = read_to_string_opt(path)?.unwrap_or_default();
    let merged = merge_yaml_block(&existing, def, command_for)?;
    write_if_changed(paths, def.agent, path, &merged)?;
    Ok(())
}

fn uninstall_yaml(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
) -> Result<(), AgentError> {
    let Some(existing) = read_to_string_opt(path)? else {
        return Ok(());
    };
    let cleaned = remove_yaml_block_text(&existing);
    write_if_changed(paths, def.agent, path, &cleaned)?;
    Ok(())
}

pub fn merge_yaml_block(
    existing: &str,
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
) -> Result<String, AgentError> {
    let cleaned = remove_yaml_block_text(existing);
    let mut lines: Vec<String> = cleaned.lines().map(str::to_owned).collect();

    // A YAML parser is not a dependency, so the merge is line-surgical: find
    // the user's top-level `hooks:` key and add our marker-wrapped entries
    // under it, leaving every other line (comments, key order) untouched.
    let Some(hooks_index) = find_top_level_hooks(&lines) else {
        let block = render_yaml_block(def, command_for);
        // Removing a previous managed block leaves the blank separator line
        // behind; collapse it so a second install produces identical bytes.
        let mut out = cleaned;
        while out.ends_with("\n\n") {
            out.pop();
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&block);
        out.push('\n');
        return Ok(out);
    };

    let rest = lines[hooks_index]
        .trim_start()
        .strip_prefix("hooks:")
        .unwrap_or("")
        .trim();
    let rest = strip_yaml_comment(rest);
    if !rest.is_empty() && !is_empty_yaml_collection(rest) {
        return Err(AgentError::ManualInstall(format!(
            "{} has a non-empty inline `hooks:` value; merge the CodeIsland block manually.",
            def.display_name
        )));
    }
    if !rest.is_empty() {
        // `hooks: []` / `hooks: {}` / `hooks: null` -> a block we can extend.
        lines[hooks_index] = "hooks:".to_owned();
    }

    let section_end = hooks_section_end(&lines, hooks_index);
    let indent = hooks_child_indent(&lines, hooks_index, section_end);
    if def.agent == AgentKind::TraeCli {
        // Trae CLI's hooks list is flat: our items go at the top of the list.
        let block = render_traecli_managed_items(def, command_for, indent);
        let insert_at = hooks_index + 1;
        lines.splice(insert_at..insert_at, block);
    } else {
        // Hermes's hooks value is a map of event -> list; the helper inserts
        // each marker-wrapped item (or event key) in place.
        render_hermes_managed_events(def, command_for, &mut lines, hooks_index, indent)?;
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn find_top_level_hooks(lines: &[String]) -> Option<usize> {
    lines.iter().position(|line| {
        let trimmed = line.trim_start();
        line.len() == trimmed.len()
            && (trimmed == "hooks:" || trimmed.starts_with("hooks:"))
            && !trimmed.starts_with('#')
    })
}

/// Exclusive end of the `hooks:` block: the next top-level key or EOF.
fn hooks_section_end(lines: &[String], hooks_index: usize) -> usize {
    for (offset, line) in lines.iter().enumerate().skip(hooks_index + 1) {
        let trimmed = line.trim_start();
        if line.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.len() == trimmed.len() && trimmed.contains(':') {
            return offset;
        }
    }
    lines.len()
}

fn hooks_child_indent(lines: &[String], hooks_index: usize, section_end: usize) -> usize {
    lines[hooks_index + 1..section_end]
        .iter()
        .find(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .map(|line| line.len() - line.trim_start().len())
        .unwrap_or(2)
}

fn strip_yaml_comment(text: &str) -> &str {
    if text.starts_with('#') {
        return "";
    }
    match text.find(" #") {
        Some(index) => text[..index].trim_end(),
        None => text,
    }
}

fn is_empty_yaml_collection(text: &str) -> bool {
    matches!(text, "[]" | "{}" | "null" | "~")
}

/// Hermes: `hooks:` is a map of event -> list. Add our item under each event
/// key, creating the key when the user has none.
fn render_hermes_managed_events(
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
    lines: &mut Vec<String>,
    hooks_index: usize,
    child_indent: usize,
) -> Result<(), AgentError> {
    for event in &def.events {
        let section_end = hooks_section_end(lines, hooks_index);
        let key_index = lines[hooks_index + 1..section_end]
            .iter()
            .position(|line| {
                let indent = line.len() - line.trim_start().len();
                indent == child_indent && line.trim_start().starts_with(&format!("{}:", event.name))
            })
            .map(|offset| hooks_index + 1 + offset);

        let item_indent = child_indent + 2;
        let marker_indent = " ".repeat(item_indent);
        if let Some(key_index) = key_index {
            let rest = lines[key_index]
                .trim_start()
                .split_once(':')
                .map(|(_, rest)| strip_yaml_comment(rest.trim()))
                .unwrap_or("");
            if !rest.is_empty() && !is_empty_yaml_collection(rest) {
                return Err(AgentError::ManualInstall(format!(
                    "{} already defines hooks.{} with an inline value; merge the CodeIsland block manually.",
                    def.display_name, event.name
                )));
            }
            if !rest.is_empty() {
                lines[key_index] = format!("{}{}:", " ".repeat(child_indent), event.name);
            }
            let mut block = vec![format!("{marker_indent}{YAML_START}")];
            block.push(format!(
                "{marker_indent}- command: {}",
                yaml_scalar(&command_for(&event.name))
            ));
            block.push(format!(
                "{marker_indent}  timeout: {}",
                timeout_secs(event)
            ));
            block.push(format!("{marker_indent}{YAML_END}"));
            lines.splice(key_index + 1..key_index + 1, block);
        } else {
            let indent = " ".repeat(child_indent);
            let mut block = vec![format!("{indent}{YAML_START}")];
            block.push(format!("{indent}{}:", event.name));
            block.push(format!(
                "{indent}  - command: {}",
                yaml_scalar(&command_for(&event.name))
            ));
            block.push(format!("{indent}    timeout: {}", timeout_secs(event)));
            block.push(format!("{indent}{YAML_END}"));
            // Inserting shifts the section; splice immediately and let the
            // next event rescan from the (updated) hooks line.
            lines.splice(hooks_index + 1..hooks_index + 1, block);
        }
    }
    Ok(())
}

/// Trae CLI: `hooks:` is a flat list of command items. Our items go at the
/// top of that list, inside one marker span.
fn render_traecli_managed_items(
    def: &AgentDefinition,
    command_for: &dyn Fn(&str) -> String,
    indent: usize,
) -> Vec<String> {
    let pad = " ".repeat(indent);
    let mut block = vec![format!("{pad}{YAML_START}")];
    for event in &def.events {
        block.push(format!("{pad}- type: command"));
        block.push(format!(
            "{pad}  command: {}",
            yaml_scalar(&command_for(&event.name))
        ));
        block.push(format!("{pad}  timeout: '{}s'", timeout_secs(event)));
    }
    block.push(format!("{pad}{YAML_END}"));
    block
}

pub fn remove_yaml_block_text(existing: &str) -> String {
    if !existing.contains(YAML_START) {
        return existing.to_owned();
    }
    let mut out = Vec::new();
    let mut skipping = false;
    for line in existing.lines() {
        if line.trim() == YAML_START {
            skipping = true;
            continue;
        }
        if skipping {
            if line.trim() == YAML_END {
                skipping = false;
            }
            continue;
        }
        out.push(line);
    }
    let mut text = out.join("\n");
    if existing.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text
}

fn render_yaml_block(def: &AgentDefinition, command_for: &dyn Fn(&str) -> String) -> String {
    let mut lines = vec![YAML_START.to_owned(), "hooks:".to_owned()];
    if def.agent == AgentKind::TraeCli {
        // Trae CLI's hooks list is a flat array of command items.
        for event in &def.events {
            lines.push("  - type: command".to_owned());
            lines.push(format!(
                "    command: {}",
                yaml_scalar(&command_for(&event.name))
            ));
            lines.push(format!("    timeout: '{}s'", timeout_secs(event)));
        }
    } else {
        // Hermes uses a map of event name -> command list.
        for event in &def.events {
            lines.push(format!("  {}:", event.name));
            lines.push(format!(
                "    - command: {}",
                yaml_scalar(&command_for(&event.name))
            ));
            lines.push(format!("      timeout: {}", timeout_secs(event)));
        }
    }
    lines.push(YAML_END.to_owned());
    lines.join("\n")
}

fn yaml_scalar(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

// ── Plugin files (Pi, OpenClaw, MiMo) ───────────────────────────────────────

fn install_plugin_js(
    paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
) -> Result<(), AgentError> {
    if path.extension().is_none() {
        return Err(AgentError::ManualInstall(manual_instructions(paths, def)));
    }
    let source = crate::plugins::plugin_source(def, &paths.hook_program_path());
    write_if_changed(paths, def.agent, path, &source)?;
    Ok(())
}

fn uninstall_plugin_js(
    _paths: &Paths,
    def: &AgentDefinition,
    path: &Path,
) -> Result<(), AgentError> {
    remove_plugin_file(def, path)
}

fn remove_plugin_file(def: &AgentDefinition, path: &Path) -> Result<(), AgentError> {
    let Some(text) = read_to_string_opt(path)? else {
        return Ok(());
    };
    if text.contains("codeisland-plugin") || text.contains(&def.marker()) {
        fs::remove_file(path)?;
    }
    Ok(())
}

// ── Detection ───────────────────────────────────────────────────────────────

pub(crate) fn detect_with(paths: &Paths, agent: AgentKind) -> ToolStatus {
    let Some(def) = definition_for_agent_in(paths, agent) else {
        return ToolStatus {
            agent,
            display_name: agent.display_name().to_owned(),
            installed: false,
            hook_installed: false,
            healthy: false,
            config_path: None,
            command: None,
            problem: Some("unsupported agent".to_owned()),
        };
    };

    let config_path = paths.primary_config(def);
    let installed = agent_present(paths, def);
    let hook_installed = config_path
        .as_ref()
        .is_some_and(|path| hook_marker_present(paths, def, path));
    let hook_exe_present = paths.hook_exe().is_some();
    let healthy = hook_installed && hook_exe_present;

    let problem = if !installed {
        Some("agent not detected".to_owned())
    } else if !hook_installed {
        Some("hooks are not installed".to_owned())
    } else if !hook_exe_present {
        Some("codeisland-hook executable not found".to_owned())
    } else {
        None
    };

    ToolStatus {
        agent,
        display_name: def.display_name.clone(),
        installed,
        hook_installed,
        healthy,
        config_path: config_path.map(|path| path.display().to_string()),
        command: Some(paths.hook_command(def)),
        problem,
    }
}

/// Install a user-defined CLI. `def` is built by the caller from the settings
/// entry (see [`AgentDefinition::custom`]); it carries `AgentKind::Unknown`
/// and a source-based marker so several custom CLIs never collide.
pub fn install_custom(paths: &Paths, def: &AgentDefinition) -> Result<ToolStatus, AgentError> {
    if !crate::definitions::custom_format_supported(def.format) {
        return Err(AgentError::Unsupported(format!(
            "{}: unsupported custom hook format",
            def.display_name
        )));
    }
    install_definition(paths, def)?;
    Ok(custom_status(paths, def))
}

pub fn uninstall_custom(paths: &Paths, def: &AgentDefinition) -> Result<ToolStatus, AgentError> {
    if !crate::definitions::custom_format_supported(def.format) {
        return Ok(custom_status(paths, def));
    }
    uninstall_definition(paths, def)?;
    Ok(custom_status(paths, def))
}

pub fn custom_status(paths: &Paths, def: &AgentDefinition) -> ToolStatus {
    let config_path = paths.primary_config(def);
    let installed = agent_present(paths, def);
    let hook_installed = config_path
        .as_ref()
        .is_some_and(|path| hook_marker_present(paths, def, path));
    let hook_exe_present = paths.hook_exe().is_some();
    let healthy = hook_installed && hook_exe_present;
    let problem = if !installed {
        Some("agent not detected".to_owned())
    } else if !hook_installed {
        Some("hooks are not installed".to_owned())
    } else if !hook_exe_present {
        Some("codeisland-hook executable not found".to_owned())
    } else {
        None
    };
    ToolStatus {
        agent: def.agent,
        display_name: def.display_name.clone(),
        installed,
        hook_installed,
        healthy,
        config_path: config_path.map(|path| path.display().to_string()),
        command: Some(paths.hook_command(def)),
        problem,
    }
}

fn hook_marker_present(_paths: &Paths, def: &AgentDefinition, path: &Path) -> bool {
    // MiniMax only discovers the pack when our manifest is beside the hooks;
    // a deleted manifest is drift that a repair rewrites.
    if def.agent == AgentKind::MiniMax {
        let manifest_ok = minimax_manifest_path(path)
            .and_then(|manifest| read_to_string_opt(&manifest).ok().flatten())
            .is_some_and(|text| text.contains(MINIMAX_MANIFEST_MARKER));
        if !manifest_ok {
            return false;
        }
    }

    if def.format == HookFormat::OpenCodePlugin {
        let Some(dir) = path.parent() else {
            return false;
        };
        let plugin_path = dir.join("plugins").join("codeisland.js");
        let plugin_present = read_to_string_opt(&plugin_path)
            .ok()
            .flatten()
            .is_some_and(|text| text.contains("codeisland-plugin"));
        let config_references = read_to_string_opt(path)
            .ok()
            .flatten()
            .is_some_and(|text| text.contains("codeisland"));
        return plugin_present && config_references;
    }

    if def.format == HookFormat::PluginJs {
        if def.agent == AgentKind::OpenClaw {
            let manifest_ok = path
                .parent()
                .map(|dir| dir.join("openclaw.plugin.json"))
                .and_then(|manifest| read_to_string_opt(&manifest).ok().flatten())
                .is_some_and(|text| text.contains(OPENCLAW_PACK_MARKER));
            if !manifest_ok {
                return false;
            }
        }
        return read_to_string_opt(path).ok().flatten().is_some_and(|text| {
            text.contains("codeisland-plugin") || text.contains(&def.marker())
        });
    }

    if matches!(
        def.format,
        HookFormat::TomlBlock | HookFormat::YamlBlock | HookFormat::Manual
    ) {
        return read_to_string_opt(path).ok().flatten().is_some_and(|text| {
            text.contains(&def.marker())
                || text.contains("codeisland-hook")
                || text.contains(YAML_START)
        });
    }

    let Some(text) = read_to_string_opt(path).ok().flatten() else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    json_contains_marker(&value, def)
}

fn json_contains_marker(value: &Value, def: &AgentDefinition) -> bool {
    match value {
        Value::String(text) => command_is_ours(text, def),
        Value::Array(items) => items.iter().any(|item| json_contains_marker(item, def)),
        Value::Object(map) => map.values().any(|item| json_contains_marker(item, def)),
        _ => false,
    }
}

fn agent_present(paths: &Paths, def: &AgentDefinition) -> bool {
    if def
        .executables
        .iter()
        .any(|executable| crate::paths::executable_on_path(executable))
    {
        return true;
    }

    // Google Antigravity shares ~/.gemini with the plain Gemini CLI, so a bare
    // ~/.gemini is not evidence that Antigravity is installed. The Claude-fork
    // AntiGravity lives under ~/.antigravity and needs no special case.
    if def.agent == AgentKind::Antigravity && def.source == "google-antigravity" {
        return paths.home().join(".gemini").join("config").exists()
            || paths.home().join(".gemini").join("antigravity-cli").exists();
    }

    paths.config_paths(def).iter().any(|path| {
        path.exists()
            || path.parent().is_some_and(|parent| parent.exists())
            || tool_root(paths, path).is_some_and(|root| root.exists())
    }) || paths
        .extra_config_paths(def)
        .iter()
        .any(|path| path.exists() || path.parent().is_some_and(|parent| parent.exists()))
}

/// The directory that means "this tool is installed", derived from a config
/// path: `.claude/settings.json` -> `~/.claude`, `.config/opencode/opencode.json`
/// -> `~/.config/opencode`, `.pi/agent/extensions/codeisland.ts` -> `~/.pi`.
fn tool_root(paths: &Paths, path: &Path) -> Option<PathBuf> {
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

// ── Shared JSON helpers ─────────────────────────────────────────────────────

fn parse_object(existing: &str) -> Result<Map<String, Value>, AgentError> {
    if existing.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(existing)
        .map_err(|error| AgentError::Config(format!("invalid JSON: {error}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| AgentError::Config("config root is not a JSON object".to_owned()))
}

fn to_json_text(value: &Value) -> Result<String, AgentError> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

fn clean_hooks_map(hooks: &mut Map<String, Value>, def: &AgentDefinition, drop_empty: bool) {
    let keys: Vec<String> = hooks.keys().cloned().collect();
    for key in keys {
        let Some(value) = hooks.get_mut(&key) else {
            continue;
        };
        let Some(entries) = value.as_array_mut() else {
            continue;
        };
        entries.retain(|entry| !entry_is_ours(entry, def));
        let empty = entries.is_empty();
        if empty && drop_empty {
            hooks.remove(&key);
        }
    }
}

fn entry_is_ours(entry: &Value, def: &AgentDefinition) -> bool {
    if let Some(hooks) = entry.get("hooks").and_then(Value::as_array) {
        return hooks.iter().any(|hook| {
            hook.get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| command_is_ours(command, def))
        });
    }
    for key in ["command", "bash"] {
        if let Some(command) = entry.get(key).and_then(Value::as_str) {
            return command_is_ours(command, def);
        }
    }
    false
}

fn command_is_ours(command: &str, def: &AgentDefinition) -> bool {
    let marker = def.marker();
    command.contains(&marker)
        || (command.contains("codeisland-hook")
            && (command.contains(&format!("--source {marker}"))
                || command.contains(&format!("--source {}", def.source))))
        || (command.contains("codeisland-bridge")
            && command.contains(&format!("--source {}", def.source)))
        || command.contains("codeisland.js")
        || command.contains("vibe-island")
        || command.contains("vibenotch")
}

fn timeout_secs(event: &HookEventDef) -> u64 {
    (event.timeout_ms + 999) / 1000
}

fn timeout_value(def: &AgentDefinition, event: &HookEventDef) -> u64 {
    // Gemini and Qwen Code document hook timeouts in milliseconds; Claude,
    // Codex, Copilot and the rest use seconds.
    if matches!(def.agent, AgentKind::Gemini | AgentKind::Qwen) {
        event.timeout_ms
    } else {
        timeout_secs(event)
    }
}

fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

fn ensure_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

// ── File I/O ────────────────────────────────────────────────────────────────

fn read_to_string_opt(path: &Path) -> Result<Option<String>, AgentError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_if_changed(
    paths: &Paths,
    agent: AgentKind,
    path: &Path,
    new_text: &str,
) -> Result<bool, AgentError> {
    let path = &crate::paths::resolve_write_path(path);
    let existing = read_to_string_opt(path)?;
    if existing.as_deref() == Some(new_text) {
        return Ok(false);
    }
    if existing.is_some() {
        backup(paths, agent, path)?;
    }
    write_atomic(path, new_text)?;
    Ok(true)
}

fn backup(paths: &Paths, agent: AgentKind, path: &Path) -> Result<(), AgentError> {
    let dir = paths.backup_dir(agent);
    fs::create_dir_all(&dir)?;
    let stamp = chrono::Utc::now()
        .format("%Y%m%dT%H%M%S%.3fZ")
        .to_string()
        .replace(':', "-")
        .replace('.', "-");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let backup_path = dir.join(format!("{stamp}-{nanos}.json"));
    fs::copy(path, backup_path)?;
    Ok(())
}

fn write_atomic(path: &Path, text: &str) -> Result<(), AgentError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp = path.with_file_name(format!(
        ".{file_name}.codeisland-{}-{nanos}.tmp",
        std::process::id()
    ));
    fs::write(&temp, text)?;

    // Unix rename replaces atomically. Windows refuses to replace, so remove
    // the destination first; the temp file still guarantees a complete write.
    #[cfg(windows)]
    {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error.into())
        }
    }
}

fn manual_instructions(paths: &Paths, def: &AgentDefinition) -> String {
    let docs = def
        .docs_url
        .as_deref()
        .unwrap_or("the agent's hook documentation");
    format!(
        "{} does not expose a hook file CodeIsland can safely edit. \
         Configure the agent to run `{} --event <Event>` for its events and forward the event JSON on stdin. See {docs}.",
        def.display_name,
        paths.hook_command(def)
    )
}
