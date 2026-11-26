//! Claude Desktop installation and live process discovery on Windows.

use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use chrono::DateTime;
#[cfg(windows)]
use codeisland_domain::cowork::STORE_DIRECTORY;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

#[cfg(windows)]
use super::cowork_watcher::Host;

#[cfg(windows)]
pub(crate) fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for support in [dirs::data_local_dir(), dirs::data_dir()].into_iter().flatten() {
        for app in ["Claude", "Claude-3p", "Claude-Data"] {
            let root = support.join(app).join(STORE_DIRECTORY);
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        let cache = local.join("Packages").join("Claude_pzs8sxrjxfjjc").join("LocalCache");
        for area in ["Local", "Roaming"] {
            for app in ["Claude", "Claude-3p", "Claude-Data"] {
                roots.push(cache.join(area).join(app).join(STORE_DIRECTORY));
            }
        }
    }
    roots
}

#[cfg(windows)]
pub(crate) fn running() -> Option<Host> {
    let local = dirs::data_local_dir();
    // SAFETY: The snapshot handle is closed after walking the process table.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut running = Process32FirstW(snapshot, &mut entry);
        let mut main: Option<Host> = None;
        while running != 0 {
            let len = entry.szExeFile.iter().position(|unit| *unit == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case("Claude.exe")
                && let Some(host) = inspect(entry.th32ProcessID, local.as_deref())
                && main.is_none_or(|previous| host.started < previous.started)
            {
                // Renderer and utility processes share Claude.exe. The
                // oldest matching process is the owning Desktop instance.
                main = Some(host);
            }
            running = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        main
    }
}

/// Checks the same Claude Desktop installation used by the Cowork watcher.
#[cfg(windows)]
pub fn is_claude_desktop_process(pid: u32) -> bool {
    // SAFETY: inspect opens and closes its own read-only process handle.
    unsafe { inspect(pid, dirs::data_local_dir().as_deref()) }.is_some()
}

#[cfg(windows)]
unsafe fn inspect(pid: u32, local: Option<&Path>) -> Option<Host> {
    // SAFETY: Buffers and FILETIMEs match the Win32 signatures and the handle is closed.
    let (image, created) = unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32_768];
        let mut len = path.len() as u32;
        let found = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) != 0;
        let mut created = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let timed = GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        if !found || !timed {
            return None;
        }
        (String::from_utf16_lossy(&path[..len as usize]), created)
    };
    if !is_desktop_image(&image, local) {
        return None;
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    let millis = i64::try_from(ticks / 10_000).ok()? - 11_644_473_600_000;
    Some(Host { pid, started: DateTime::from_timestamp_millis(millis)? })
}

fn is_desktop_image(image: &str, local: Option<&Path>) -> bool {
    let image = image.replace('/', "\\").to_ascii_lowercase();
    let Some((parent, "claude.exe")) = image.rsplit_once('\\') else {
        return false;
    };
    let package_dir = parent.strip_suffix("\\app").unwrap_or(parent);
    if let Some((install, package)) = package_dir.rsplit_once('\\')
        && install.ends_with("\\windowsapps")
        && package.starts_with("claude_")
        && package.ends_with("__pzs8sxrjxfjjc")
    {
        return true;
    }

    // The Desktop app also ships a claude.exe CLI under Claude-3p/claude-code.
    // Legacy GUI installations place the executable directly in the app folder.
    local.is_some_and(|local| {
        ["Claude", "Claude-3p", "Programs/Claude"].into_iter().any(|directory| {
            let expected = local.join(directory).to_string_lossy().replace('/', "\\");
            parent.eq_ignore_ascii_case(&expected)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_gui_executable_identifies_claude_desktop() {
        let local = Some(Path::new("C:/Users/demo/AppData/Local"));
        for path in [
            "C:/Program Files/WindowsApps/Claude_5.1231.2.0_x64__pzs8sxrjxfjjc/app/Claude.exe",
            "D:/WindowsApps/Claude_5.1231.2.0_arm64__pzs8sxrjxfjjc/Claude.exe",
            "C:/Users/demo/AppData/Local/Programs/Claude/Claude.exe",
        ] {
            assert!(is_desktop_image(path, local), "{path}");
        }
        for path in [
            "C:/Users/demo/AppData/Local/Claude-3p/claude-code/2.1.321/d0e23af87588/claude.exe",
            "C:/Users/demo/AppData/Local/bin/claude.exe",
            "C:/Program Files/WindowsApps/Claude_5.1231.2.0_x64__pzs8sxrjxfjjc/app/resources/claude.exe",
            "C:/Program Files/WindowsApps/Claude_5.1231.2.0_x64__different/app/Claude.exe",
        ] {
            assert!(!is_desktop_image(path, local), "{path}");
        }
    }
}
