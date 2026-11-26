use std::fs;
use std::path::Path;

use codeisland_agents::{
    AgentDefinition, HookFormat, custom_status, definition, definitions, detect_all, install_at,
    install_custom, merge_claude_json, merge_copilot_json, merge_cursor_json,
    merge_event_map_json, merge_yaml_block, paths::Paths, remove_claude_json,
    remove_copilot_json, remove_cursor_json, remove_event_map_json, uninstall_at,
    uninstall_custom,
};
use codeisland_domain::AgentKind;
use serde_json::Value;

fn claude() -> &'static codeisland_agents::AgentDefinition {
    definition(AgentKind::Claude).expect("claude definition")
}

#[test]
fn claude_merge_is_idempotent_and_preserves_unknown_keys() {
    let existing = r#"{
        "theme": "dark",
        "permissions": {"allow": ["Bash(ls:*)"]},
        "hooks": {
            "Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "user-hook"}]}]
        }
    }"#;
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            claude().marker()
        )
    };

    let first = merge_claude_json(existing, claude(), &command_for).expect("merge");
    let second = merge_claude_json(&first, claude(), &command_for).expect("merge again");
    assert_eq!(first, second, "second merge must produce identical bytes");

    let value: Value = serde_json::from_str(&first).expect("json");
    assert_eq!(value["theme"], "dark");
    assert_eq!(value["permissions"]["allow"][0], "Bash(ls:*)");
    let stop = value["hooks"]["Stop"].as_array().expect("stop hooks");
    assert_eq!(stop.len(), 2, "user hook plus ours");
    assert!(
        stop.iter()
            .any(|entry| entry["hooks"][0]["command"] == "user-hook")
    );
    assert!(
        stop.iter().any(|entry| entry["hooks"][0]["command"]
            .as_str()
            .is_some_and(|command| command.contains("codeisland-claude")))
    );
    let ours = stop
        .iter()
        .find(|entry| {
            entry["hooks"][0]["command"]
                .as_str()
                .is_some_and(|command| command.contains("codeisland-claude"))
        })
        .expect("our hook");
    assert_eq!(ours["matcher"], "*", "Claude hooks use the wildcard matcher");
}

#[test]
fn claude_uninstall_removes_only_our_entries() {
    let existing = r#"{
        "theme": "dark",
        "hooks": {
            "Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "user-hook"}]}]
        }
    }"#;
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            claude().marker()
        )
    };
    let installed = merge_claude_json(existing, claude(), &command_for).expect("merge");
    let cleaned = remove_claude_json(&installed, claude()).expect("remove");

    let value: Value = serde_json::from_str(&cleaned).expect("json");
    assert_eq!(value["theme"], "dark");
    let stop = value["hooks"]["Stop"].as_array().expect("stop hooks");
    assert_eq!(stop.len(), 1);
    assert_eq!(stop[0]["hooks"][0]["command"], "user-hook");
    assert!(value["hooks"].get("SessionStart").is_none());
    assert!(value["hooks"].get("PermissionRequest").is_none());
}

#[test]
fn event_map_merge_preserves_version_and_user_entries() {
    let codex = definition(AgentKind::Codex).expect("codex definition");
    let existing = r#"{
        "version": 2,
        "unknown": true,
        "hooks": {
            "PreToolUse": [{"hooks": [{"type": "command", "command": "other"}]}]
        }
    }"#;
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            codex.marker()
        )
    };

    let first = merge_event_map_json(existing, codex, &command_for).expect("merge");
    let second = merge_event_map_json(&first, codex, &command_for).expect("merge again");
    assert_eq!(first, second);

    let value: Value = serde_json::from_str(&first).expect("json");
    assert_eq!(value["version"], 2);
    assert_eq!(value["unknown"], true);
    let pretool = value["hooks"]["PreToolUse"].as_array().expect("array");
    assert_eq!(pretool.len(), 2);
    assert!(pretool.iter().any(|entry| entry["hooks"][0]["command"] == "other"));
    let ours = pretool
        .iter()
        .find(|entry| entry["hooks"][0]["command"] != "other")
        .expect("our entry");
    assert!(
        ours.get("matcher").is_none(),
        "Codex/Gemini nested entries carry no matcher"
    );

    let cleaned = remove_event_map_json(&first, codex).expect("remove");
    let value: Value = serde_json::from_str(&cleaned).expect("json");
    assert_eq!(value["version"], 2);
    assert_eq!(value["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert!(value["hooks"].get("SessionStart").is_none());
}

#[test]
fn cursor_merge_is_idempotent_and_removable() {
    let cursor = definition(AgentKind::Cursor).expect("cursor definition");
    let existing = r#"{
        "version": 1,
        "other": "kept",
        "hooks": {"stop": [{"command": "user-stop"}]}
    }"#;
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            cursor.marker()
        )
    };

    let first = merge_cursor_json(existing, cursor, &command_for).expect("merge");
    let second = merge_cursor_json(&first, cursor, &command_for).expect("merge again");
    assert_eq!(first, second);

    let value: Value = serde_json::from_str(&first).expect("json");
    assert_eq!(value["other"], "kept");
    let stop = value["hooks"]["stop"].as_array().expect("stop");
    assert_eq!(stop.len(), 2);

    let cleaned = remove_cursor_json(&first, cursor).expect("remove");
    let value: Value = serde_json::from_str(&cleaned).expect("json");
    assert_eq!(value["other"], "kept");
    assert_eq!(value["hooks"]["stop"].as_array().unwrap().len(), 1);
    assert!(value["hooks"].get("beforeSubmitPrompt").is_none());
}

#[test]
fn install_writes_backs_up_and_uninstalls_in_a_hermetic_home() {
    let home = std::env::temp_dir().join(format!("codeisland-agents-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".claude")).expect("create config dir");
    let config = home.join(".claude").join("settings.json");
    fs::write(&config, "{\n  \"theme\": \"dark\"\n}\n").expect("seed config");

    let status = install_at(AgentKind::Claude, &home).expect("install");
    assert!(status.installed);
    assert!(status.hook_installed);
    let expected_path = config.display().to_string();
    assert_eq!(status.config_path.as_deref(), Some(expected_path.as_str()));

    let installed = fs::read_to_string(&config).expect("read installed");
    assert!(installed.contains("codeisland-claude"));
    assert!(installed.contains("\"theme\": \"dark\""));

    let backup_root = home
        .join("AppData")
        .join("Local")
        .join("CodeIsland")
        .join("backups")
        .join("claude");
    let backups: Vec<_> = fs::read_dir(&backup_root)
        .expect("backup dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("backup entries");
    assert_eq!(backups.len(), 1, "one backup before the first mutation");

    install_at(AgentKind::Claude, &home).expect("install again");
    let reinstalled = fs::read_to_string(&config).expect("read reinstalled");
    assert_eq!(installed, reinstalled, "install is idempotent");

    uninstall_at(AgentKind::Claude, &home).expect("uninstall");
    let cleaned = fs::read_to_string(&config).expect("read cleaned");
    let value: Value = serde_json::from_str(&cleaned).expect("json");
    assert_eq!(value["theme"], "dark");
    assert!(value.get("hooks").is_none(), "our hooks key is gone");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn detect_all_is_sorted_and_covers_every_agent() {
    let statuses = detect_all();
    let mut agents: Vec<&str> = definitions().iter().map(|def| def.agent.tag()).collect();
    agents.sort_unstable();
    agents.dedup();
    assert_eq!(
        statuses.len(),
        agents.len(),
        "one row per AgentKind (several definitions share a kind)"
    );
    let mut names: Vec<String> = statuses
        .iter()
        .map(|status| status.display_name.to_lowercase())
        .collect();
    let sorted = {
        let mut copy = names.clone();
        copy.sort();
        copy
    };
    assert_eq!(names, sorted);
    names.dedup();
    assert!(names.len() >= 31);
}

#[test]
fn copilot_uses_its_own_flat_schema_and_keeps_version() {
    let copilot = definition(AgentKind::Copilot).expect("copilot definition");
    let existing = r#"{
        "version": 7,
        "other": true,
        "hooks": {
            "preToolUse": [{"type": "command", "bash": "user-hook", "timeoutSec": 9}]
        }
    }"#;
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            copilot.marker()
        )
    };

    let first = merge_copilot_json(existing, copilot, &command_for).expect("merge");
    let second = merge_copilot_json(&first, copilot, &command_for).expect("merge again");
    assert_eq!(first, second, "copilot merge must be idempotent");

    let value: Value = serde_json::from_str(&first).expect("json");
    assert_eq!(value["version"], 7, "user version is preserved");
    assert_eq!(value["other"], true);
    let pre_tool = value["hooks"]["preToolUse"].as_array().expect("array");
    assert_eq!(pre_tool.len(), 2, "user entry plus ours");
    let ours = pre_tool
        .iter()
        .find(|entry| entry["bash"].as_str().is_some_and(|text| text.contains("codeisland-copilot")))
        .expect("our entry");
    assert_eq!(ours["type"], "command");
    assert!(ours["timeoutSec"].as_u64().is_some());
    assert!(ours.get("matcher").is_none(), "Copilot has no matcher");
    assert!(ours.get("hooks").is_none(), "Copilot has no nested hooks array");

    let cleaned = remove_copilot_json(&first, copilot).expect("remove");
    let value: Value = serde_json::from_str(&cleaned).expect("json");
    assert_eq!(value["version"], 7);
    let pre_tool = value["hooks"]["preToolUse"].as_array().expect("array");
    assert_eq!(pre_tool.len(), 1);
    assert_eq!(pre_tool[0]["bash"], "user-hook");
    assert!(value["hooks"].get("sessionStart").is_none());
}

#[test]
fn qwen_timeouts_stay_in_milliseconds() {
    let qwen = definition(AgentKind::Qwen).expect("qwen definition");
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            qwen.marker()
        )
    };
    let merged = merge_claude_json("{}", qwen, &command_for).expect("merge");
    let value: Value = serde_json::from_str(&merged).expect("json");
    assert_eq!(
        value["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 5000,
        "Qwen declares milliseconds"
    );
    assert_eq!(
        value["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 86400000
    );
}

#[test]
fn minimax_install_writes_a_discoverable_plugin_manifest() {
    let home = std::env::temp_dir().join(format!("codeisland-minimax-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".minimax")).expect("create minimax root");

    let status = install_at(AgentKind::MiniMax, &home).expect("install");
    assert!(status.installed);
    assert!(status.hook_installed);

    let plugin_dir = home
        .join(".minimax")
        .join("plugins")
        .join("codeisland");
    let manifest = plugin_dir.join(".claude-plugin").join("plugin.json");
    let text = fs::read_to_string(&manifest).expect("manifest");
    let value: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(value["name"], "codeisland");
    assert_eq!(value["hooks"][0], "hooks/hooks.json");
    assert!(value["description"]
        .as_str()
        .is_some_and(|text| text.contains("CodeIsland MiniMax Code plugin")));
    assert!(plugin_dir.join("hooks").join("hooks.json").is_file());

    // Idempotent: a second install produces identical bytes.
    let again = fs::read_to_string(&manifest).expect("manifest again");
    install_at(AgentKind::MiniMax, &home).expect("install again");
    assert_eq!(text, fs::read_to_string(&manifest).expect("manifest third"));

    uninstall_at(AgentKind::MiniMax, &home).expect("uninstall");
    assert!(!manifest.exists(), "our manifest is removed");
    assert!(
        fs::read_to_string(plugin_dir.join("hooks").join("hooks.json"))
            .map(|text| !text.contains("codeisland-minimax"))
            .unwrap_or(true),
        "our hooks entries are removed"
    );

    let _ = fs::remove_dir_all(&home);
    let _ = again;
}

#[test]
fn minimax_leaves_a_foreign_manifest_alone() {
    let home = std::env::temp_dir().join(format!("codeisland-minimax-{}", uuid::Uuid::new_v4()));
    let plugin_dir = home.join(".minimax").join("plugins").join("codeisland");
    let manifest = plugin_dir.join(".claude-plugin").join("plugin.json");
    fs::create_dir_all(manifest.parent().expect("parent")).expect("dirs");
    fs::write(&manifest, r#"{"name":"someone-else"}"#).expect("seed");

    install_at(AgentKind::MiniMax, &home).expect("install");
    let text = fs::read_to_string(&manifest).expect("manifest");
    assert_eq!(text, r#"{"name":"someone-else"}"#);

    uninstall_at(AgentKind::MiniMax, &home).expect("uninstall");
    assert!(manifest.exists(), "not ours to delete");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn hermes_yaml_merges_into_an_existing_hooks_map() {
    let hermes = definition(AgentKind::Hermes).expect("hermes definition");
    let existing = "model: gpt-5\n\
                    hooks:\n\
                    \x20 post_tool_call:\n\
                    \x20   - command: 'user-hook'\n\
                    \x20     timeout: 9\n\
                    \n\
                    other: kept\n";
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            hermes.marker()
        )
    };

    let merged = merge_yaml_block(existing, hermes, &command_for).expect("merge");
    let second = merge_yaml_block(&merged, hermes, &command_for).expect("merge again");
    assert_eq!(merged, second, "yaml merge must be idempotent");
    assert_eq!(merged.matches("hooks:").count(), 1, "no duplicate hooks key");
    assert!(merged.contains("model: gpt-5"));
    assert!(merged.contains("other: kept"));
    assert!(merged.contains("user-hook"), "user entry preserved");
    assert!(merged.contains("post_tool_call:"), "existing event kept");
    assert!(merged.contains("pre_tool_call:"), "missing event added");
    assert!(merged.contains("codeisland-hermes"));

    let cleaned = codeisland_agents::remove_yaml_block_text(&merged);
    assert!(!cleaned.contains("codeisland managed"));
    assert!(!cleaned.contains("codeisland-hermes"));
    assert!(cleaned.contains("user-hook"));
    assert!(cleaned.contains("model: gpt-5"));
    assert_eq!(cleaned.matches("hooks:").count(), 1);
}

#[test]
fn traecli_yaml_merges_into_an_existing_flat_list() {
    let traecli = codeisland_agents::definition_for_source("traecli").expect("traecli definition");
    let existing = "hooks: []\n# trailing comment\n";
    let command_for = |event: &str| {
        format!(
            "codeisland-hook send --source {} --event {event} --stdin",
            traecli.marker()
        )
    };

    let merged = merge_yaml_block(existing, traecli, &command_for).expect("merge");
    assert!(merged.contains("hooks:\n"), "empty flow list becomes a block");
    assert!(merged.contains("- type: command"));
    assert!(merged.contains("codeisland-traecli"));
    assert!(merged.contains("# trailing comment"));
    let second = merge_yaml_block(&merged, traecli, &command_for).expect("merge again");
    assert_eq!(merged, second);
}

#[test]
fn codex_config_keeps_comments_and_other_keys() {
    let home = std::env::temp_dir().join(format!("codeisland-codex-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".codex")).expect("create codex dir");
    let config = home.join(".codex").join("config.toml");
    fs::write(
        &config,
        "# my settings\nmodel = \"gpt-5\"\n\n[profiles.fast]\nmodel = \"o4\"\n",
    )
    .expect("seed config");

    install_at(AgentKind::Codex, &home).expect("install");
    let text = fs::read_to_string(&config).expect("config");
    assert!(text.contains("# my settings"));
    assert!(text.contains("model = \"gpt-5\""));
    assert!(text.contains("[profiles.fast]"));
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(parsed["features"]["hooks"].as_bool(), Some(true));
    assert!(parsed.get("hooks").is_none());

    let once = text;
    install_at(AgentKind::Codex, &home).expect("repair");
    assert_eq!(once, fs::read_to_string(&config).expect("config again"));

    uninstall_at(AgentKind::Codex, &home).expect("uninstall");
    let text = fs::read_to_string(&config).expect("config after uninstall");
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(parsed["features"]["hooks"].as_bool(), Some(true), "the master switch stays");
    assert!(text.contains("# my settings"));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn codex_config_with_a_hooks_table_is_not_clobbered() {
    let home = std::env::temp_dir().join(format!("codeisland-codex-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".codex")).expect("create codex dir");
    let config = home.join(".codex").join("config.toml");
    let original = "[features.hooks]\nuser = true\n";
    fs::write(&config, original).expect("seed config");

    let result = install_at(AgentKind::Codex, &home);
    assert!(result.is_err(), "a non-boolean hooks key is left alone");
    assert_eq!(fs::read_to_string(&config).expect("config"), original);
    assert!(!home.join(".codex/hooks.json").exists());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn codex_migrates_legacy_flag_inside_features_and_keeps_unrelated_tables() {
    let home = std::env::temp_dir().join(format!("codeisland-codex-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".codex")).expect("create codex dir");
    let config = home.join(".codex/config.toml");
    fs::write(
        &config,
        "[hooks]\nuser = true\n\n[features]\ncodex_hooks = false # feature comment\nother = true\n",
    ).expect("seed config");
    install_at(AgentKind::Codex, &home).expect("install");
    let text = fs::read_to_string(&config).expect("config");
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(parsed["features"]["hooks"].as_bool(), Some(true));
    assert_eq!(parsed["features"]["other"].as_bool(), Some(true));
    assert_eq!(parsed["hooks"]["user"].as_bool(), Some(true));
    assert!(parsed["features"].get("codex_hooks").is_none());
    assert!(text.contains("# feature comment"));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn plugin_installs_generate_real_sources() {
    let home = std::env::temp_dir().join(format!("codeisland-plugins-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".pi").join("agent")).expect("pi dir");
    // Make the Pi definition the one this hermetic home selects even when the
    // test machine happens to have an `omp` binary on PATH.
    fs::create_dir_all(home.join(".omp").join("agent")).expect("omp dir");
    fs::create_dir_all(home.join(".config").join("opencode")).expect("opencode dir");

    install_at(AgentKind::Pi, &home).expect("pi install");
    let pi = fs::read_to_string(home.join(".pi/agent/extensions/codeisland.ts")).expect("pi plugin");
    assert!(pi.contains("runHook"));
    assert!(!pi.contains("export default {}"));

    install_at(AgentKind::OpenCode, &home).expect("opencode install");
    let opencode = fs::read_to_string(
        home.join(".config/opencode/plugins/codeisland.js"),
    )
    .expect("opencode plugin");
    assert!(opencode.contains("permissionReplyFromDecision"));
    assert!(opencode.contains("AskUserQuestion"));
    assert!(!opencode.contains("export default {}"));

    uninstall_at(AgentKind::Pi, &home).expect("pi uninstall");
    assert!(!home.join(".pi/agent/extensions/codeisland.ts").exists());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn openclaw_install_writes_the_pack_and_registers_it() {
    let home = std::env::temp_dir().join(format!("codeisland-openclaw-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".openclaw")).expect("openclaw dir");

    install_at(AgentKind::OpenClaw, &home).expect("install");
    let plugin_dir = home.join(".openclaw").join("codeisland-plugin");
    assert!(plugin_dir.join("index.ts").is_file());
    let manifest = fs::read_to_string(plugin_dir.join("openclaw.plugin.json")).expect("manifest");
    assert!(manifest.contains("CodeIsland OpenClaw plugin"));
    assert!(plugin_dir.join("package.json").is_file());

    let config_path = home.join(".openclaw").join("openclaw.json");
    let config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("config")).expect("json");
    let paths = config["plugins"]["load"]["paths"].as_array().expect("paths");
    assert_eq!(paths.len(), 1);
    let registered_dir = Path::new(paths[0].as_str().expect("plugin directory path"));
    assert_eq!(registered_dir, plugin_dir);
    assert_eq!(config["plugins"]["entries"]["codeisland"]["enabled"], true);

    // A second install is a no-op, including the registration list.
    let once = fs::read_to_string(&config_path).expect("config again");
    install_at(AgentKind::OpenClaw, &home).expect("install again");
    assert_eq!(once, fs::read_to_string(&config_path).expect("config third"));

    uninstall_at(AgentKind::OpenClaw, &home).expect("uninstall");
    assert!(!plugin_dir.exists(), "our pack is removed");
    let config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("config")).expect("json");
    assert!(config["plugins"].get("load").is_none());
    assert!(config["plugins"].get("entries").is_none());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn openclaw_leaves_an_unparseable_config_alone() {
    let home = std::env::temp_dir().join(format!("codeisland-openclaw-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".openclaw")).expect("openclaw dir");
    let config_path = home.join(".openclaw").join("openclaw.json");
    let json5 = "{\n  // user comment\n  plugins: {},\n}\n";
    fs::write(&config_path, json5).expect("seed config");

    install_at(AgentKind::OpenClaw, &home).expect("install");
    assert_eq!(fs::read_to_string(&config_path).expect("config"), json5);
    assert!(home
        .join(".openclaw/codeisland-plugin/openclaw.plugin.json")
        .is_file());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn same_kind_products_install_every_present_config() {
    let home = std::env::temp_dir().join(format!("codeisland-pi-omp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".pi").join("agent")).expect("pi dir");
    fs::create_dir_all(home.join(".omp").join("agent")).expect("omp dir");

    install_at(AgentKind::Pi, &home).expect("install");
    assert!(
        home.join(".pi/agent/extensions/codeisland.ts").is_file(),
        "Pi extension installed"
    );
    assert!(
        home.join(".omp/agent/extensions/codeisland.ts").is_file(),
        "Oh My Pi extension installed too"
    );

    uninstall_at(AgentKind::Pi, &home).expect("uninstall");
    assert!(!home.join(".pi/agent/extensions/codeisland.ts").exists());
    assert!(!home.join(".omp/agent/extensions/codeisland.ts").exists());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn custom_cli_installs_under_its_own_marker_and_config_key() {
    let home = std::env::temp_dir().join(format!("codeisland-custom-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".mytool")).expect("tool dir");
    let paths = Paths::with_home(home.clone());

    let def = AgentDefinition::custom(
        "MyTool",
        "mytool",
        ".mytool/settings.json",
        HookFormat::ClaudeJson,
        "hooks",
    );
    let status = install_custom(&paths, &def).expect("install");
    assert!(status.installed);
    assert!(status.hook_installed);
    let config = home.join(".mytool").join("settings.json");
    let installed = fs::read_to_string(&config).expect("read config");
    assert!(installed.contains("codeisland-mytool"));

    install_custom(&paths, &def).expect("install again");
    assert_eq!(
        installed,
        fs::read_to_string(&config).expect("read again"),
        "custom install is idempotent"
    );
    assert!(custom_status(&paths, &def).hook_installed);

    // A second custom CLI with a custom top-level key must not collide with
    // the first one's marker or key.
    let keyed = AgentDefinition::custom(
        "Keyed",
        "keyed",
        ".keyed/config.json",
        HookFormat::EventMapJson,
        "myHooks",
    );
    install_custom(&paths, &keyed).expect("install keyed");
    let value: Value = serde_json::from_str(
        &fs::read_to_string(home.join(".keyed").join("config.json")).expect("read keyed"),
    )
    .expect("json");
    assert!(value["myHooks"]["SessionStart"].is_array());
    assert!(value.get("hooks").is_none());

    uninstall_custom(&paths, &def).expect("uninstall");
    let cleaned = fs::read_to_string(&config).expect("read cleaned");
    assert!(!cleaned.contains("codeisland-mytool"));
    assert!(!cleaned.contains("codeisland-keyed"));

    let _ = fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn install_writes_through_a_symlinked_config() {
    let home = std::env::temp_dir().join(format!("codeisland-link-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join(".claude")).expect("config dir");
    fs::create_dir_all(home.join("dotfiles")).expect("dotfiles dir");
    let target = home.join("dotfiles").join("claude-settings.json");
    fs::write(&target, "{\n  \"theme\": \"dark\"\n}\n").expect("seed target");
    std::os::unix::fs::symlink(&target, home.join(".claude").join("settings.json"))
        .expect("symlink");

    install_at(AgentKind::Claude, &home).expect("install");
    assert!(
        home.join(".claude").join("settings.json").is_symlink(),
        "the link itself must survive"
    );
    let written = fs::read_to_string(&target).expect("read target");
    assert!(written.contains("codeisland-claude"));
    assert!(written.contains("\"theme\": \"dark\""));

    let _ = fs::remove_dir_all(&home);
}
