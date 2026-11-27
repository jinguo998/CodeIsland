pub fn clean_push_text(text: &str, limit: usize, markdown: bool) -> String {
    codeisland_swift::call(c"cleanPushText", serde_json::json!({
        "text": text, "limit": limit.min(text.len()), "markdown": markdown,
    }))
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
