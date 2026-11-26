//! Windows discovery for the running Codex Desktop package and its bundled CLI.

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

#[cfg(windows)]
pub(crate) fn running_root() -> Option<PathBuf> {
    // SAFETY: snapshot and process handles are closed on every path.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut running = Process32FirstW(snapshot, &mut entry);
        let mut root = None;
        while running != 0 {
            let length = entry.szExeFile.iter().position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if desktop_executable_name(&name)
                && let Some(image) = image_path(entry.th32ProcessID)
                && let Some(package) = desktop_root(&image)
            {
                root = Some(package.to_path_buf());
                break;
            }
            running = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        root
    }
}

#[cfg(windows)]
unsafe fn image_path(pid: u32) -> Option<PathBuf> {
    // SAFETY: Win32 writes at most the supplied UTF-16 buffer length.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut path = vec![0; 32_768];
        let mut length = path.len() as u32;
        let found = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut length);
        CloseHandle(handle);
        (found != 0).then(|| PathBuf::from(OsString::from_wide(&path[..length as usize])))
    }
}

/// Checks the same package identity used by Codex Desktop discovery.
#[cfg(windows)]
pub fn is_codex_desktop_process(pid: u32) -> bool {
    // SAFETY: image_path opens and closes its own read-only process handle.
    unsafe { image_path(pid) }.is_some_and(|image| desktop_root(&image).is_some())
}

fn desktop_executable_name(name: &str) -> bool {
    // The Codex package now uses ChatGPT.exe for its GUI. Standalone
    // ChatGPT and Codex CLI processes still fail the package path check.
    name.eq_ignore_ascii_case("Codex.exe") || name.eq_ignore_ascii_case("ChatGPT.exe")
}

fn desktop_root(image: &Path) -> Option<&Path> {
    if !desktop_executable_name(image.file_name()?.to_str()?) {
        return None;
    }
    let parent = image.parent()?;
    let root = if parent.file_name()?.to_str()?.eq_ignore_ascii_case("app") {
        parent.parent()?
    } else {
        parent
    };
    let package = root.file_name()?.to_str()?.to_ascii_lowercase();
    let install = root.parent()?.file_name()?.to_str()?;
    (install.eq_ignore_ascii_case("WindowsApps") && package.starts_with("openai.codex_"))
        .then_some(root)
}

#[cfg(windows)]
pub(crate) fn executable(root: &Path) -> Option<PathBuf> {
    for relative in ["app/resources/codex.exe", "resources/codex.exe"] {
        let path = root.join(relative);
        if path.is_file() {
            return Some(path);
        }
    }
    let bin = dirs::data_local_dir()?.join("OpenAI").join("Codex").join("bin");
    std::fs::read_dir(bin).ok()?.flatten().filter_map(|entry| {
        let path = entry.path().join("codex.exe");
        let metadata = path.metadata().ok()?;
        metadata.is_file().then_some((metadata.modified().ok(), path))
    }).max_by(|a, b| a.cmp(b)).map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_both_gui_names_in_the_codex_package() {
        let package = Path::new("C:/Program Files/WindowsApps/OpenAI.Codex_26.1103.7773.0_x64__2p2nqsd0c76g0");
        for image in ["app/ChatGPT.exe", "app/Codex.exe", "Codex.exe"] {
            assert_eq!(desktop_root(&package.join(image)), Some(package));
        }
    }

    #[test]
    fn rejects_unrelated_chatgpt_codex_cli_and_package_helpers() {
        for image in [
            "C:/Program Files/WindowsApps/OpenAI.ChatGPT_26.1103.7773.0_x64__2p2nqsd0c76g0/app/ChatGPT.exe",
            "C:/Users/demo/AppData/Local/OpenAI/Codex/bin/5449c1f0f26f1f98/codex.exe",
            "C:/Users/demo/AppData/Roaming/Code/agent-host/codex.exe",
            "C:/Program Files/WindowsApps/OpenAI.Codex_26.1103.7773.0_x64__2p2nqsd0c76g0/app/helper.exe",
        ] {
            assert_eq!(desktop_root(Path::new(image)), None, "{image}");
        }
    }
}
