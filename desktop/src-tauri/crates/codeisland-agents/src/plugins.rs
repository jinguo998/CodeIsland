//! The provider callbacks come from the shipped macOS extension resources.
use codeisland_domain::AgentKind;
use crate::definitions::AgentDefinition;

pub fn plugin_source(def: &AgentDefinition, hook_program: &str) -> String {
    let source = match def.agent {
        AgentKind::OpenCode | AgentKind::MiMo => include_str!(concat!(env!("OUT_DIR"), "/opencode.js")),
        AgentKind::Pi if def.source == "omp" => include_str!(concat!(env!("OUT_DIR"), "/omp.ts")),
        AgentKind::Pi => include_str!(concat!(env!("OUT_DIR"), "/pi.ts")),
        AgentKind::OpenClaw => include_str!(concat!(env!("OUT_DIR"), "/openclaw.ts")),
        _ => return stub(&def.marker()),
    };
    let template = if def.agent == AgentKind::MiMo {
        codeisland_swift::call(c"install.mimoPlugin", source)
    } else {
        source.to_owned()
    };
    // Substitute literal arguments only. The executable text is inserted last
    // so even a path containing one of the placeholder names is unchanged.
    let marker = def.marker();
    let source_arg = serde_json::to_string(&marker).expect("encode plugin source");
    let program = serde_json::to_string(hook_program).expect("encode sidecar path");
    let body = template.replace("__CODEISLAND_SOURCE__", &source_arg)
        .replace("__CODEISLAND_EXECUTABLE__", &program);
    format!("// codeisland-plugin: {marker}\n{body}")
}

pub fn stub(marker: &str) -> String {
    format!("// codeisland-plugin: {marker}\nexport default {{}};\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::definition_for_source;

    #[test]
    fn opencode_plugin_embeds_the_hook_command_and_source() {
        let def = definition_for_source("opencode").expect("opencode definition");
        let text = plugin_source(def, "C:\\Program Files\\CodeIsland\\codeisland-hook.exe");
        assert!(text.contains("codeisland-plugin: codeisland-opencode"));
        assert!(text.contains("C:\\\\Program Files\\\\CodeIsland\\\\codeisland-hook.exe"));
        assert!(text.contains("__SOURCE__") == false);
        assert!(text.contains("\"opencode\""));
        assert!(text.contains("permission"));
        assert!(!text.contains("export default {}"));
    }

    #[test]
    fn mimo_reuses_the_opencode_plugin_with_its_own_source() {
        let def = definition_for_source("mimo").expect("mimo definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("const SOURCE = \"mimo\";"));
        assert!(text.contains("codeisland-plugin: codeisland-mimo"));
    }

    #[test]
    fn opencode_plugin_supports_the_v2_setup_service() {
        let def = definition_for_source("opencode").expect("opencode definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("setup: setupV2"));
        assert!(text.contains("createV2Runtime"));
        assert!(text.contains("session.tool.called"));
        assert!(text.contains("form.created"));
        assert!(text.contains("ctx.event.subscribe"));
    }

    #[test]
    fn pi_extension_routes_the_ask_tool_to_the_question_card() {
        let def = definition_for_source("pi").expect("pi definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("forwardAsk"));
        assert!(text.contains("event.toolName === \"ask\""));
        assert!(text.contains("AskUserQuestion"));
    }

    #[test]
    fn pi_and_openclaw_generate_real_extensions() {
        for source in ["pi", "omp", "openclaw"] {
            let def = definition_for_source(source).expect("definition");
            let text = plugin_source(def, "codeisland-hook");
            assert!(text.contains("runHook"), "{source} has no hook bridge");
            assert!(!text.contains("export default {}"), "{source} is still a stub");
        }
    }

    #[test]
    fn openclaw_gates_dangerous_shell_calls_on_the_island() {
        let def = definition_for_source("openclaw").expect("definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("before_tool_call"));
        assert!(text.contains("PermissionRequest"));
        assert!(text.contains("blockReason"));
        assert!(text.contains("DANGEROUS_PATTERNS"));
        assert!(text.contains("pendingPermissionLanes"));
    }
}
