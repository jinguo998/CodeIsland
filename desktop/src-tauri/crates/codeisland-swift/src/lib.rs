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
    destroy: Destroy,
    call: Call,
    _library: Library,
}

static API: LazyLock<Api> = LazyLock::new(|| {
    let exe = std::env::current_exe().expect("get executable path");
    let dir = exe.parent().expect("executable directory");
    let mut path = dir.join("swift-core").join(libloading::library_filename("CodeIslandDesktopCore"));
    if !path.is_file() && dir.file_name().is_some_and(|name| name == "deps") {
        path = dir.parent().unwrap().join("swift-core").join(path.file_name().unwrap());
    }
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

fn invoke<R: DeserializeOwned>(context: *mut c_void, operation: &CStr, args: impl Serialize) -> R {
    let input = serde_json::to_vec(&args).expect("encode Swift arguments");
    let mut result = Vec::<u8>::new();
    let status = unsafe {
        (API.call)(context, operation.as_ptr(), input.as_ptr(), input.len(), receive, (&mut result as *mut Vec<u8>).cast())
    };
    assert_eq!(status, 0, "Swift {:?}: {}", operation, String::from_utf8_lossy(&result));
    serde_json::from_slice(&result).unwrap_or_else(|error| panic!("decode Swift {operation:?}: {error}"))
}

pub fn call<R: DeserializeOwned>(operation: &CStr, args: impl Serialize) -> R {
    invoke(std::ptr::null_mut(), operation, args)
}

pub struct Context(Mutex<Handle>);
struct Handle(NonNull<c_void>);

// The owner synchronizes all mutations. No Swift reference escapes the ABI.
unsafe impl Send for Handle {}

impl Default for Context {
    fn default() -> Self {
        Context(Mutex::new(Handle(NonNull::new(unsafe { (API.create)() }).expect("Swift context"))))
    }
}

impl Context {
    pub fn call<R: DeserializeOwned>(&self, operation: &CStr, args: impl Serialize) -> R {
        let handle = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        invoke(handle.0.as_ptr(), operation, args)
    }
}

impl Drop for Handle {
    fn drop(&mut self) { unsafe { (API.destroy)(self.0.as_ptr()) } }
}
