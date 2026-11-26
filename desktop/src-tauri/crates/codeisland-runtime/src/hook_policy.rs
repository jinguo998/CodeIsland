//! Filtering and opt-in automatic approval for live IPC hook events.

use codeisland_domain::{AgentEvent, AgentEventType, AppSettings};

pub(crate) fn excluded(settings: &AppSettings, event: &AgentEvent) -> bool {
    event.cwd.as_deref().is_some_and(|cwd| {
        settings
            .excluded_hook_cwd_substrings
            .split(',')
            .map(str::trim)
            .filter(|pattern| !pattern.is_empty())
            .any(|pattern| cwd.contains(pattern))
    })
}

pub(crate) fn auto_approves(settings: &AppSettings, source: &str, event: &AgentEvent) -> bool {
    if event.event_type != AgentEventType::PermissionRequest
        || event.tool.as_deref() == Some("AskUserQuestion")
    {
        return false;
    }

    let tool_matches = event.tool.as_deref().is_some_and(|tool| {
        settings
            .auto_approve_tools
            .split(',')
            .filter(|name| !name.is_empty())
            .any(|name| name == tool)
    });
    if tool_matches {
        return true;
    }

    let source = source.trim().to_ascii_lowercase();
    let source = source.strip_prefix("codeisland-").unwrap_or(&source);
    if source.is_empty() {
        return false;
    }
    let canonical = canonical_approval_source(source, settings);
    settings
        .auto_approve_sources
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let entry = entry.to_ascii_lowercase();
            let entry = entry.strip_prefix("codeisland-").unwrap_or(&entry);
            canonical_approval_source(entry, settings) == canonical
        })
}

/// Canonical source for approval policy. Separate products can share the
/// same agent enum without sharing permission to run tools.
fn canonical_approval_source<'a>(source: &'a str, settings: &AppSettings) -> &'a str {
    match source {
        "factory" => "droid",
        "ag" | "anti-gravity" | "anti gravity" => "antigravity",
        "googleantigravity" | "google antigravity" | "google-anti-gravity"
        | "antigravity-ide" | "antigravity-cli" | "agy" => "google-antigravity",
        "work-buddy" | "work body" | "work-body" | "workbody" => "workbuddy",
        "hermes-agent" | "hermes-agents" | "hermes agent" | "hermes agents" => "hermes",
        "grok-cli" | "grokcli" | "grok-build" | "grok build" => "grok",
        "open-claw" | "open claw" | "clawdbot" => "openclaw",
        "qwen-code" | "qwencode" => "qwen",
        "cursor-agent" | "cursoragent" | "cursorcli" => "cursor-cli",
        "qodercli" | "qoderclicn" | "qodercli-cn" | "qoder-cn" | "qodercn" | "qoder_cli"
        | "qoder_cn" => "qoder-cli",
        "dtcoder" => "aiwork",
        "dtcoder-cli" | "dtcodercli" | "aiworkcli" => "aiwork-cli",
        "qoder-work" | "qoder work" => "qoderwork",
        "kimi-cli" | "kimicli" | "kimi-code" | "kimicode" | "kimi_code" | "kimi-code-cli"
        | "kimicodecli" | "kimi_code_cli" => "kimi",
        "kiro-cli" | "kirocli" => "kiro",
        "codebuddycn" | "codybuddy-cn" => "codybuddycn",
        "step-fun" | "step fun" => "stepfun",
        "trae-cn" | "trae_cn" | "trae cn" => "traecn",
        "omp" | "oh-my-pi" | "oh my pi" => "pi",
        "z-code" | "z code" => "zcode",
        "mcode" | "minimax-code" | "minimaxcode" | "minimax-cli" | "minimaxcli"
        | "minimax-code-cli" => "minimax",
        "mimocode" | "mimo-code" | "mimo_code" | "mimo code" | "mimo-cli" | "mimocli"
        | "mimo-desktop" | "mimodesktop" | "xiaomi-mimo" | "xiaomimimo" | "xiaomi mimo"
        | "xiaomi-mimo-ai" => "mimo",
        "claude-code" | "claude_code" => "claude",
        "deepseek" => "dsh",
        custom if settings.custom_clis.iter().any(|cli| cli.source.eq_ignore_ascii_case(custom)) => custom,
        other => [
            ("google-antigravity", "google-antigravity"),
            ("googleantigravity", "google-antigravity"),
            ("antigravity", "antigravity"),
            ("workbuddy", "workbuddy"),
            ("hermes", "hermes"),
            ("grok", "grok"),
            ("openclaw", "openclaw"),
            ("qwen", "qwen"),
            ("kiro", "kiro"),
            ("kimi", "kimi"),
            ("minimax", "minimax"),
            ("codybuddycn", "codybuddycn"),
            ("codebuddycn", "codybuddycn"),
            ("stepfun", "stepfun"),
            ("traecn", "traecn"),
            ("traecli", "traecli"),
            ("trae", "trae"),
        ]
        .into_iter()
        .find_map(|(prefix, canonical)| other.starts_with(prefix).then_some(canonical))
        .unwrap_or(other),
    }
}
