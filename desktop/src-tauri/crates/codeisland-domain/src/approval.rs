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
pub fn agent_keeps_always_rule(agent: AgentKind) -> bool {
    matches!(agent, AgentKind::Claude | AgentKind::Qoder | AgentKind::QoderCn | AgentKind::MiniMax)
}

pub fn command_for_event(event: &AgentEvent) -> Option<&str> {
    if event.agent != AgentKind::Codex || event.tool.as_deref() != Some("Bash") {
        return None;
    }
    let payload = event.payload.as_ref()?;
    payload.pointer("/tool_input/command")
        .or_else(|| payload.pointer("/toolInput/command"))?.as_str()
}

/// Only save a rule when all commands can be described by up to four literal
/// prefixes. Each prefix is the first three words of a plain command.
pub fn prefix_patterns(script: &str) -> Option<Vec<Vec<String>>> {
    let mut patterns = Vec::new();
    for mut command in plain_commands(script)? {
        command.truncate(3);
        if !patterns.contains(&command) {
            patterns.push(command);
        }
    }
    (patterns.len() <= 4).then_some(patterns)
}

pub fn commands_match(script: &str, patterns: &[Vec<String>]) -> bool {
    !patterns.is_empty() && plain_commands(script).is_some_and(|commands| {
        commands.iter().all(|command| {
            patterns.iter().any(|prefix| !prefix.is_empty() && command.starts_with(prefix))
        })
    })
}

#[derive(Default)]
struct PlainScript {
    commands: Vec<Vec<String>>,
    words: Vec<String>,
    word: String,
    word_parts: usize,
    unquoted_run: bool,
    name_is_quoted: bool,
    awaiting_command: bool,
    empty_concatenation: bool,
}

impl PlainScript {
    fn end_word(&mut self) {
        if self.word_parts == 0 {
            return;
        }
        self.empty_concatenation |= self.word_parts > 1 && self.word.is_empty();
        self.words.push(std::mem::take(&mut self.word));
        self.word_parts = 0;
        self.unquoted_run = false;
    }

    fn end_command(&mut self) -> Option<()> {
        self.end_word();
        let name = self.words.first()?;
        if self.name_is_quoted || name.is_empty() || name.contains('=') || matches!(
            name.as_str(),
            "if" | "then" | "else" | "elif" | "fi" | "case" | "esac" | "for" | "select"
                | "while" | "until" | "do" | "done" | "in" | "function" | "time" | "coproc"
        ) {
            return None;
        }
        self.commands.push(std::mem::take(&mut self.words));
        self.name_is_quoted = false;
        Some(())
    }
}

fn plain_commands(script: &str) -> Option<Vec<Vec<String>>> {
    let mut input = script.chars().peekable();
    let mut parsed = PlainScript::default();
    while let Some(ch) = input.next() {
        match ch {
            ' ' | '\t' => parsed.end_word(),
            '\n' => {
                if !parsed.awaiting_command && (parsed.word_parts > 0 || !parsed.words.is_empty()) {
                    parsed.end_command()?;
                }
            }
            ';' => {
                if parsed.awaiting_command || input.peek() == Some(&';') {
                    return None;
                }
                parsed.end_command()?;
            }
            '&' | '|' => {
                if parsed.awaiting_command || (ch == '|' && input.peek() == Some(&'&'))
                    || (ch == '&' && input.peek() != Some(&'&'))
                {
                    return None;
                }
                if input.peek() == Some(&ch) {
                    input.next();
                }
                parsed.end_command()?;
                parsed.awaiting_command = true;
            }
            '\'' | '"' => {
                parsed.name_is_quoted |= parsed.words.is_empty();
                parsed.word_parts += 1;
                parsed.unquoted_run = false;
                parsed.awaiting_command = false;
                loop {
                    let inner = input.next()?;
                    if inner == ch {
                        break;
                    }
                    if ch == '"' && (matches!(inner, '$' | '`') || (inner == '\\' && input.peek()
                        .is_some_and(|next| matches!(*next, '$' | '`' | '"' | '\\' | '\n'))))
                    {
                        return None;
                    }
                    parsed.word.push(inner);
                }
            }
            _ => {
                if ch.is_ascii_control() || "{}*?[]\\~^#$`()<>!".contains(ch) {
                    return None;
                }
                if !parsed.unquoted_run {
                    if ch == '=' {
                        return None;
                    }
                    parsed.word_parts += 1;
                }
                parsed.word.push(ch);
                parsed.unquoted_run = true;
                parsed.awaiting_command = false;
            }
        }
    }
    if parsed.awaiting_command {
        return None;
    }
    if parsed.word_parts > 0 || !parsed.words.is_empty() {
        parsed.end_command()?;
    }
    (!parsed.commands.is_empty() && !parsed.empty_concatenation).then_some(parsed.commands)
}

#[cfg(test)]
mod tests {
    use super::*;

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
