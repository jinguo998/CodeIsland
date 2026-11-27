//! Claude Desktop's Code tab routes by host session ID, distinct from the CLI UUID.

pub fn valid_id(id: &str) -> bool {
    codeisland_swift::call(c"claudeHostID", id)
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
