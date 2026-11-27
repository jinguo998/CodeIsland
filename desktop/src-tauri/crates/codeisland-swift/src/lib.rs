//! The Swift library owns application policy and any state behind a context.
//! Only UTF-8 JSON and borrowed byte buffers cross the versioned C ABI.
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;
use std::sync::{LazyLock, Mutex};

use libloading::Library;
use serde::{Serialize, de::DeserializeOwned};

type Create = unsafe extern "C" fn() -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Write = extern "C" fn(*const u8, usize, *mut c_void);
type Call = unsafe extern "C" fn(*mut c_void, *const c_char, *const u8, usize, Write, *mut c_void) -> i32;

struct Api {
    create: Create,
    copy: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    destroy: Destroy,
    call: Call,
    _library: Library,
}

static API: LazyLock<Api> = LazyLock::new(|| {
    let exe = std::env::current_exe().expect("get executable path");
    let dir = exe.parent().expect("executable directory");
    let filename = libloading::library_filename("CodeIslandDesktopCore");
    // A hook spawned by cargo test can sit next to an older staged bundle.
    // Its Swift library is the one built with this executable. An installed
    // program always resolves its private runtime beside itself.
    let profile = std::path::Path::new(env!("CODEISLAND_CARGO_PROFILE_DIR"));
    let cargo_binary = dir == profile || dir.strip_prefix(profile).ok()
        .and_then(|path| path.components().next())
        .and_then(|part| part.as_os_str().to_str())
        .is_some_and(|part| matches!(part, "build" | "deps" | "examples" | "tests"));
    let path = if cargo_binary {
        std::path::Path::new(env!("CODEISLAND_BUILT_SWIFT_DIR")).join(&filename)
    } else {
        dir.join("swift-core").join(&filename)
    };
    // Load the core from the application's directory, including its private
    // Swift runtime on Windows. Never search the current working directory.
    unsafe {
        #[cfg(windows)]
        let loaded = libloading::os::windows::Library::load_with_flags(&path, 0x100 | 0x1000).map(Library::from);
        #[cfg(not(windows))]
        let loaded = Library::new(&path);
        let library = loaded.unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
        Api {
            create: *library.get(b"codeisland_core_create_v1").expect("Swift context constructor"),
            copy: *library.get(b"codeisland_core_copy_v1").expect("Swift context copy"),
            destroy: *library.get(b"codeisland_core_destroy_v1").expect("Swift context destructor"),
            call: *library.get(b"codeisland_core_call_v1").expect("Swift call interface"),
            _library: library,
        }
    }
});

extern "C" fn receive(bytes: *const u8, len: usize, destination: *mut c_void) {
    if len > 0 {
        // Swift invokes this callback synchronously while its Data is alive.
        unsafe { (*destination.cast::<Vec<u8>>()).extend_from_slice(std::slice::from_raw_parts(bytes, len)); }
    }
}

fn invoke<R: DeserializeOwned>(context: *mut c_void, operation: &CStr, input: &[u8]) -> R {
    let mut result = Vec::<u8>::new();
    let status = unsafe {
        (API.call)(context, operation.as_ptr(), input.as_ptr(), input.len(), receive, (&mut result as *mut Vec<u8>).cast())
    };
    assert_eq!(status, 0, "Swift {:?}: {}", operation, String::from_utf8_lossy(&result));
    serde_json::from_slice(&result).unwrap_or_else(|error| panic!("decode Swift {operation:?}: {error}"))
}

pub fn call<R: DeserializeOwned>(operation: &CStr, args: impl Serialize) -> R {
    call_bytes(operation, &serde_json::to_vec(&args).expect("encode Swift arguments"))
}

pub fn call_bytes<R: DeserializeOwned>(operation: &CStr, bytes: &[u8]) -> R {
    invoke(std::ptr::null_mut(), operation, bytes)
}

#[derive(Debug)]
pub struct Context(Mutex<Handle>);
#[derive(Debug)]
struct Handle(NonNull<c_void>);

// The owner synchronizes all mutations. No Swift reference escapes the ABI.
unsafe impl Send for Handle {}

impl Default for Context {
    fn default() -> Self {
        Context(Mutex::new(Handle(NonNull::new(unsafe { (API.create)() }).expect("Swift context"))))
    }
}

impl Clone for Context {
    fn clone(&self) -> Self {
        let handle = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let copy = unsafe { (API.copy)(handle.0.as_ptr()) };
        Self(Mutex::new(Handle(NonNull::new(copy).expect("copy Swift context"))))
    }
}

impl Context {
    pub fn call<R: DeserializeOwned>(&self, operation: &CStr, args: impl Serialize) -> R {
        self.call_bytes(operation, &serde_json::to_vec(&args).expect("encode Swift arguments"))
    }

    pub fn call_bytes<R: DeserializeOwned>(&self, operation: &CStr, bytes: &[u8]) -> R {
        let handle = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        invoke(handle.0.as_ptr(), operation, bytes)
    }
}

impl Drop for Handle {
    fn drop(&mut self) { unsafe { (API.destroy)(self.0.as_ptr()) } }
}
