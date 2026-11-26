//! Unix domain socket transport helpers.
//!
//! All Unix `unsafe` for the crate lives here (and in `windows.rs`).

use std::io;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::endpoint::endpoint_name;

const RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// Connect to the app's socket, retrying while it is missing or refusing
/// connections (the app may still be starting up).
pub(crate) fn connect(timeout: Duration) -> io::Result<UnixStream> {
    let path = endpoint_name();
    let deadline = Instant::now() + timeout;

    loop {
        match UnixStream::connect(&path) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                let retryable = matches!(
                    error.raw_os_error(),
                    Some(libc::ENOENT) | Some(libc::ECONNREFUSED)
                );
                if !retryable {
                    return Err(error);
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err(error);
                }
                std::thread::sleep((deadline - now).min(RETRY_INTERVAL));
            }
        }
    }
}

/// Effective user id, used to name the socket.
pub(crate) fn current_uid() -> u32 {
    // SAFETY: `getuid` takes no arguments and cannot fail.
    unsafe { libc::getuid() }
}
