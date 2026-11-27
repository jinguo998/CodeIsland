//! Cross-platform process liveness and Git branch helpers.

/// Returns `true` when a process with `pid` is currently running.
///
/// On Windows this asks `OpenProcess`/`GetExitCodeProcess`; on Unix it uses `kill(pid, 0)`.
/// A process that exists but cannot be queried (access denied / `EPERM`) is considered alive.
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    imp::is_process_alive(pid)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    type Handle = *mut c_void;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: i32 = 5;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: Handle) -> i32;
    }

    pub(super) fn is_process_alive(pid: u32) -> bool {
        // SAFETY: plain Win32 calls with a valid PID; the handle is closed before returning.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // Access denied means the process exists but belongs to another user/SYSTEM.
            return std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED);
        }
        let mut exit_code = 0u32;
        let ok = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
        unsafe {
            CloseHandle(handle);
        }
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(unix)]
mod imp {
    const SIGNAL_ZERO: i32 = 0;
    const EPERM: i32 = 1;

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }

    pub(super) fn is_process_alive(pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 performs the permission/existence check without delivering a signal.
        let result = unsafe { kill(pid, SIGNAL_ZERO) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(EPERM)
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub(super) fn is_process_alive(_pid: u32) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
        assert!(!is_process_alive(0));
    }
}


/// The checked-out branch for `cwd`, read from the nearest `.git/HEAD`.
/// Detached HEADs report the short commit hash.
pub fn git_branch(cwd: &str) -> Option<String> {
    codeisland_swift::call(c"git.branch", cwd)
}

#[cfg(windows)]
pub(crate) fn windows_process_info(pid: u32) -> Option<(String, chrono::DateTime<chrono::Utc>)> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    // SAFETY: Query buffers match the Win32 signatures and the handle is closed.
    let (image, creation) = unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32_768];
        let mut len = path.len() as u32;
        let found = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) != 0;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let timed = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        if !found || !timed || exit.dwHighDateTime != 0 || exit.dwLowDateTime != 0 {
            return None;
        }
        (String::from_utf16_lossy(&path[..len as usize]), creation)
    };
    let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    Some((image, windows_filetime(ticks)?))
}

#[cfg(any(windows, test))]
pub(crate) fn windows_filetime(ticks: u64) -> Option<chrono::DateTime<chrono::Utc>> {
    const UNIX_EPOCH: u64 = 116_444_736_000_000_000;
    let unix_ms = i64::try_from(ticks.checked_sub(UNIX_EPOCH)? / 10_000).ok()?;
    chrono::DateTime::from_timestamp_millis(unix_ms)
}

pub(crate) fn is_session_process_alive(pid: u32, started: Option<chrono::DateTime<chrono::Utc>>) -> bool {
    #[cfg(windows)]
    if let Some(expected) = started
        && let Some((_, current)) = windows_process_info(pid)
    {
        return current == expected;
    }
    let _ = started;
    is_process_alive(pid)
}
