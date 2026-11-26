//! Windows named-pipe transport and current-user SID helpers.
//!
//! All Win32 `unsafe` for the crate lives here (and in `unix.rs`).

use std::ffi::c_void;
use std::io;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::endpoint::endpoint_name;

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;
const RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// Connect to the app's pipe, retrying while it is busy or not created yet.
pub(crate) fn connect(timeout: Duration) -> io::Result<std::fs::File> {
    let path = endpoint_name();
    let deadline = Instant::now() + timeout;

    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(error) => {
                let retryable = matches!(
                    error.raw_os_error(),
                    Some(ERROR_PIPE_BUSY) | Some(ERROR_FILE_NOT_FOUND)
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

/// SECURITY_ATTRIBUTES plus the SECURITY_DESCRIPTOR it points at.
///
/// Created fresh for each `CreateNamedPipeW` call: the kernel snapshots the
/// descriptor at creation time, so it only has to outlive that call.
pub(crate) struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl PipeSecurity {
    /// DACL granting full access to SYSTEM and the current user only:
    /// `D:P(A;;GA;;;SY)(A;;GA;;;<SID>)`.
    pub(crate) fn for_current_user() -> io::Result<Self> {
        // `OW` (owner rights) is a valid SDDL alias and is used only if the
        // token query fails; the pipe creator then remains the owner.
        let sid = current_user_sid().unwrap_or("OW");
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})");
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();

        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SDDL_REVISION_1 is always 1.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    fn attributes_ptr(&mut self) -> *mut c_void {
        std::ptr::addr_of_mut!(self.attributes).cast()
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                LocalFree(self.descriptor as HLOCAL);
            }
        }
    }
}

/// Create one pipe instance with the current-user DACL.
#[allow(unused_unsafe)] // `create_with_security_attributes_raw` is an unsafe fn in tokio.
pub(crate) fn create_pipe_server(
    options: &ServerOptions,
    path: &str,
    security: &mut PipeSecurity,
) -> io::Result<NamedPipeServer> {
    // SAFETY: `security` owns a descriptor allocated by
    // ConvertStringSecurityDescriptorToSecurityDescriptorW and keeps it alive
    // for the duration of this call.
    unsafe { options.create_with_security_attributes_raw(path, security.attributes_ptr()) }
}

/// String form (`S-1-5-21-...`) of the current process token's user SID.
pub(crate) fn current_user_sid() -> Option<&'static str> {
    static SID: OnceLock<Option<String>> = OnceLock::new();
    SID.get_or_init(query_current_user_sid).as_deref()
}

fn query_current_user_sid() -> Option<String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let _token = TokenHandle(token);

        // First call sizes the buffer and is expected to fail.
        let mut length = 0u32;
        let _ = GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length);
        if length == 0 {
            return None;
        }

        // Use usize words so the buffer is correctly aligned for TOKEN_USER.
        let word = std::mem::size_of::<usize>();
        let words = (length as usize).div_ceil(word);
        let mut buffer = vec![0usize; words];
        let buffer_length = (words * word) as u32;
        if GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            buffer_length,
            &mut length,
        ) == 0
        {
            return None;
        }

        let token_user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut sid_string: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) == 0 {
            return None;
        }

        let result = wide_string(sid_string);
        LocalFree(sid_string as HLOCAL);
        Some(result)
    }
}

fn wide_string(pointer: *const u16) -> String {
    unsafe {
        let mut length = 0usize;
        while *pointer.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
    }
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        unsafe {
            let _: BOOL = CloseHandle(self.0);
        }
    }
}
