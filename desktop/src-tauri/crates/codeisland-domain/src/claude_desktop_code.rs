//! Claude Desktop's Code tab routes by host session ID, distinct from the CLI UUID.

pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("local_").is_some_and(|body| {
        (1..=64).contains(&body.len())
            && body.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_valid_code_host_ids() {
        assert!(valid_id("local_A1b2-C3"));
        assert!(valid_id(&format!("local_{}", "x".repeat(64))));
        for id in ["local_", "a1b2-c3", "last", "local_a_b", "local_字", "local_a&session=last", "Local_ab"] {
            assert!(!valid_id(id), "{id}");
        }
        assert!(!valid_id(&format!("local_{}", "x".repeat(65))));
    }
}
