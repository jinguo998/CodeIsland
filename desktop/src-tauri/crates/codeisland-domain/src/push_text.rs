use std::sync::LazyLock;

use regex::Regex;

/// Credential and home-path patterns from Swift HookEvent.sanitizedSummary.
static REDACTIONS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r#"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/@:"']+:[^\s/@"']+@"#, "$1[REDACTED]@"),
        (r#"(?i)(authorization\s*[:=]\s*(?:(?:bearer|basic|token|digest|negotiate|bot)\s+)?)([\"']?)[^\"'\s]+"#, "$1[REDACTED]"),
        (r#"(?i)((?:aws[-_]?secret[-_]?access[-_]?key|aws[-_]?(?:session|security)[-_]?token|x[-_]amz[-_]security[-_]token)\s*[:=]\s*)([\"']?)[^\"'\s]+"#, "$1[REDACTED]"),
        (r#"(?i)((?:x[-_])?(?:api[-_]?key|(?:auth|access|private|refresh|id|bearer|bot|deploy|session)[-_]?token|client[-_]?secret|secret|password|passwd)\s*:\s*)([\"']?)[^\"'\s]+"#, "$1[REDACTED]"),
        (r#"(?i)(["'][a-z0-9_.-]*?(?:api[-_]?key|apikey|token|secret|password|passwd|private[-_]?key|access[-_]?key|credentials?)["']\s*:\s*["'])[^"']*(["'])"#, "$1[REDACTED]$2"),
        (r#"(?i)((?:set-)?cookie\s*:\s*)[^"'\n]+"#, "$1[REDACTED]"),
        (r#"(?i)(\bcurl\b[^|;&]*?\s(?:-u\s*|--user(?:\s+|=)))(["']?)([^\s:"']*):[^\s"']+"#, "$1$2$3:[REDACTED]"),
        (r#"(?i)(\bcurl\b[^|;&]*?\s(?:-b|--cookie)(?:\s+|=))(["']?)[^\s"']+"#, "$1$2[REDACTED]"),
        (r#"(\bmysql\w*\b[^|;&]*?\s-p)([^\s"'-][^\s"']*)"#, "$1[REDACTED]"),
        (r#"(\bsshpass\b[^|;&]*?\s-p\s*)([^\s"']+)"#, "$1[REDACTED]"),
        (r#"(?i)(--(?:api[-_]?key|token|secret|password|passwd|auth|auth[-_]?token|access[-_]?token)(?:\s+|=))([\"']?)[^\s\"']+"#, "$1$2[REDACTED]"),
        (r#"(?i)((?:api[-_]?key|apikey|token|secret|password|passwd)=)([\"']?)[^\s\"'&]+"#, "$1$2[REDACTED]"),
        (r#"(?i)([?&](?:api[-_]?key|apikey|key|access[-_]?key|secret[-_]?key|token|access[-_]?token|auth[-_]?token|refresh[-_]?token|id[-_]?token|private[-_]?token|secret|client[-_]?secret|signature|sig|sign|password|passwd|auth)=)[^&\s\"'#]+"#, "$1[REDACTED]"),
        (r#"\b(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{20,}|(?:AKIA|ASIA)[A-Z0-9]{16})\b"#, "[REDACTED]"),
        (r#"\b(?:glpat-[A-Za-z0-9_-]{20,}|xox[abposre]-[A-Za-z0-9-]{10,}|xapp-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{35}|ya29\.[A-Za-z0-9_-]{20,}|npm_[A-Za-z0-9]{36}|hf_[A-Za-z0-9]{30,}|[rs]k_(?:live|test)_[A-Za-z0-9]{16,}|SG\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}|pypi-[A-Za-z0-9_-]{20,}|eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,})"#, "[REDACTED]"),
        (r#"\b(?:bot)?\d{6,12}:[A-Za-z0-9_-]{30,}"#, "[REDACTED]"),
        (r#"/(?:Users|home)/[^/\s]+"#, "~"),
        (r#"\b[A-Za-z0-9_\-+/=]{48,}\b"#, "[REDACTED]"),
        (r"(?i)\b[a-z]:[\\/](?:Users|Documents and Settings)[\\/][^\\/\s]+", "~"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| (Regex::new(pattern).expect("valid push redaction"), replacement))
    .collect()
});

pub fn clean_push_text(text: &str, limit: usize, markdown: bool) -> String {
    let budget = limit.saturating_mul(2).saturating_add(256);
    let prefix = text.char_indices().nth(budget).map_or(text, |(index, _)| {
        let prefix = &text[..index];
        prefix
            .rsplit_once('\n')
            .or_else(|| prefix.rsplit_once(char::is_whitespace))
            .map_or(
                prefix,
                |(head, _)| if head.is_empty() { prefix } else { head },
            )
    });
    let mut lines = Vec::new();
    for original in prefix.lines() {
        let original = original.trim();
        if markdown && original.starts_with("```") {
            continue;
        }
        let line = if markdown {
            let trimmed = original.trim_start_matches('#').trim_start();
            trimmed
                .strip_prefix("> ")
                .unwrap_or(trimmed)
                .replace("**", "")
                .replace('`', "")
        } else {
            original.to_owned()
        };
        let mut line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            continue;
        }
        for (pattern, replacement) in REDACTIONS.iter() {
            if let std::borrow::Cow::Owned(redacted) = pattern.replace_all(&line, *replacement) {
                line = redacted;
            }
        }
        lines.push(line);
    }
    let result = lines.join("\n");
    if result.chars().count() > limit {
        result
            .chars()
            .take(limit.saturating_sub(1))
            .chain(['…'])
            .collect()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_prose_and_redacts_credentials_from_push_text() {
        let input = "**The token refresh**\nAuthorization: Bearer abc123\nuser=https://name:password@service.invalid";
        let text = clean_push_text(input, 200, true);
        assert!(text.contains("The token refresh"));
        assert!(text.contains("Authorization: Bearer [REDACTED]"));
        assert!(text.contains("https://[REDACTED]@service.invalid"));
        assert!(!text.contains("abc123") && !text.contains("name:password"));
    }
}
