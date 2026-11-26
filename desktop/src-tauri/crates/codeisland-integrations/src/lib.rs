//! Advanced integrations for CodeIsland Desktop.
//!
//! - [`push`] — Bark / ntfy / Telegram / Lark / Slack / Discord / webhook
//! - [`ble`] — ESP32 Buddy protocol and (feature `ble`) BLE manager
//! - [`ssh`] — remote hosts and hook deployment (feature `ssh`)
//! - [`lan`] — local HTTP state/action server (feature `lan`)
//! - [`quota`] — local Claude usage scanner
//!
//! Feature-gated modules always expose their types; without the feature the
//! manager is a stub returning `Unsupported`. That keeps the public API stable
//! for the app regardless of the build profile.

pub mod apple_companion;
pub mod ble;
pub mod claude_limits;
pub mod hook_webhook;
pub mod lan;
pub mod push;
mod push_channels;
pub mod quota;
pub mod ssh;

pub use ble::{
    BuddyCommand, BuddyError, BuddyFrame, BuddyManager, BuddyPairResponse, BuddyProtocol,
    BuddyScreenOrientation, BuddyState, BuddyUplinkEvent, MascotId, MascotStatus, agent_frame,
    brightness_frame, control_command_frame, event_frame, model_frame, orientation_frame,
    pair_request_frame, preview_frame, random_host_id, stats_frame, subagent_frame,
    time_hint_frame, tool_history_clear_frame, tool_history_frame, unpair_frame, workspace_frame,
};
pub use lan::{
    ActionSink, LanAction, LanCrypto, LanError, LanServer, SnapshotSource, generate_pairing_code,
    pairing_code_from_bytes,
};
pub use push::{PushChannel, PushError, PushGate, PushNotifier, PushRequest};
pub use quota::{QuotaSnapshot, scan as scan_quota, scan_in as scan_quota_in};
pub use ssh::{DeployResult, RemoteHost, SshError, SshManager};

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Fills `N` bytes from `RandomState` (OS-seeded per process) mixed with a
/// monotonic counter and the clock. Used for pairing codes, HMAC secrets and
/// the Buddy host id; none of these need a cryptographic RNG dependency.
pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    let mut filled = 0;
    while filled < N {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
        );
        hasher.write_u32(std::process::id());
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_usize(out.as_ptr() as usize);
        let word = hasher.finish().to_le_bytes();
        let take = (N - filled).min(word.len());
        out[filled..filled + take].copy_from_slice(&word[..take]);
        filled += take;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_bytes_are_not_all_zero() {
        let bytes = random_bytes::<32>();
        assert!(bytes.iter().any(|byte| *byte != 0));
        let other = random_bytes::<32>();
        assert_ne!(bytes, other);
    }
}
