//! Platform-specific endpoint naming.
//!
//! Windows: `\\.\pipe\codeisland-<hash>` where `<hash>` is the lower 32 bits
//! (8 hex chars) of the current user's SID hashed with `DefaultHasher`.
//! Unix: `$XDG_RUNTIME_DIR/codeisland-<uid>.sock`, falling back to
//! `/tmp/codeisland-<uid>.sock`.

#[cfg(windows)]
use std::collections::hash_map::DefaultHasher;
#[cfg(windows)]
use std::hash::{Hash, Hasher};

/// Address the desktop app listens on and hooks connect to.
pub fn endpoint_name() -> String {
    if let Some(address) = endpoint_override() {
        return address;
    }
    #[cfg(windows)]
    {
        let identity = crate::windows::current_user_sid()
            .map(|sid| sid.to_owned())
            .or_else(|| std::env::var("USERNAME").ok())
            .unwrap_or_else(|| "unknown".to_owned());
        format!(r"\\.\pipe\codeisland-{}", short_hash(&identity))
    }

    #[cfg(unix)]
    {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        base.join(format!("codeisland-{}.sock", crate::unix::current_uid()))
            .to_string_lossy()
            .into_owned()
    }

    #[cfg(not(any(windows, unix)))]
    {
        "codeisland".to_owned()
    }
}

/// A dedicated address for an isolated app and its test hooks.
pub fn endpoint_override() -> Option<String> {
    let value = std::env::var("CODEISLAND_SOCKET_PATH").ok()?;
    let value = value.trim();
    if value.is_empty() || value.contains('\0') {
        return None;
    }
    #[cfg(windows)]
    let valid = value.strip_prefix(r"\\.\pipe\")
        .is_some_and(|name| !name.is_empty() && !name.contains(['\\', '/']));
    #[cfg(not(windows))]
    let valid = std::path::Path::new(value).is_absolute();
    valid.then(|| value.to_owned())
}

/// 8 lowercase hex characters derived from `value`.
#[cfg(windows)]
fn short_hash(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_stable() {
        assert_eq!(endpoint_name(), endpoint_name());
        assert!(endpoint_name().contains("codeisland-"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_endpoint_shape() {
        let name = endpoint_name();
        assert!(
            name.starts_with(r"\\.\pipe\codeisland-"),
            "unexpected endpoint: {name}"
        );
        let suffix = name.rsplit('-').next().expect("suffix");
        assert_eq!(suffix.len(), 8, "unexpected suffix: {suffix}");
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(windows)]
    #[test]
    fn short_hash_is_eight_hex_chars() {
        let hash = short_hash("S-1-5-21-1-2-3-1001");
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(unix)]
    #[test]
    fn unix_endpoint_shape() {
        let expected = format!("codeisland-{}.sock", crate::unix::current_uid());
        let name = endpoint_name();
        assert!(name.ends_with(&expected), "unexpected endpoint: {name}");
    }
}
