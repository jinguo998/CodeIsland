use codeisland_domain::{AgentKind, SessionSnapshot, claude_desktop_code, cowork};

pub(crate) fn is_claude_desktop_session(session: &SessionSnapshot) -> bool {
    session.agent == AgentKind::Claude
        && !session.session_id.starts_with("ssh:")
        && (session.session_id.starts_with(cowork::PREFIX)
            || session.claude_desktop_session_id.as_deref().is_some_and(claude_desktop_code::valid_id))
}

pub fn is_frontmost(session: &SessionSnapshot) -> bool {
    #[cfg(windows)]
    return !session.session_id.starts_with("ssh:") && !is_wsl_session(session) && win32::is_frontmost(session);
    #[cfg(not(windows))]
    {
        let _ = session;
        false
    }
}

pub fn is_tab_visible(session: &SessionSnapshot) -> bool {
    #[cfg(windows)]
    return !session.session_id.starts_with("ssh:") && !is_wsl_session(session) && win32::is_tab_visible(session);
    #[cfg(not(windows))]
    {
        let _ = session;
        false
    }
}

pub async fn activate(session: &SessionSnapshot) -> Option<bool> {
    if session.session_id.starts_with("ssh:") {
        return Some(false);
    }
    #[cfg(windows)]
    {
        if is_wsl_session(session) {
            return Some(activate_wsl(session).await);
        }
        let window = win32::request_activation(session)?;
        for _ in 0..8 {
            if win32::is_foreground_window(window) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        if !win32::is_foreground_window(window) {
            return Some(false);
        }
        if win32::is_wezterm_session(session) {
            return Some(crate::wezterm::activate(session).await && win32::is_frontmost(session));
        }
        return Some(true);
    }
    #[cfg(not(windows))]
    {
        let _ = session;
        None
    }
}

#[cfg(windows)]
fn is_wsl_session(session: &SessionSnapshot) -> bool {
    use std::path::{Component, Path, Prefix};

    if session.session_id.starts_with(codeisland_domain::CODEX_DESKTOP_SESSION_PREFIX)
        || is_claude_desktop_session(session)
    {
        return false;
    }
    if session.wsl_process.is_some() {
        return true;
    }
    let Some(Component::Prefix(prefix)) = session.transcript_path.as_deref()
        .and_then(|path| Path::new(path).components().next())
    else {
        return false;
    };
    matches!(
        prefix.kind(),
        Prefix::UNC(host, _) | Prefix::VerbatimUNC(host, _) if host.to_str().is_some_and(|name|
            name.eq_ignore_ascii_case("wsl.localhost") || name.eq_ignore_ascii_case("wsl$")
        )
    )
}

#[cfg(windows)]
async fn activate_wsl(session: &SessionSnapshot) -> bool {
    let Some(process) = session.wsl_process.clone() else {
        return false;
    };
    let session = session.clone();
    let verified = tokio::task::spawn_blocking(move || codeisland_runtime::wsl_session_is_current(&session))
        .await
        .unwrap_or(false);
    if !verified {
        return false;
    }
    if let Some(pane) = process.wezterm_pane {
        let Some(window) = win32::unique_wezterm_window() else { return false };
        if !crate::wezterm::activate_wsl_pane(pane).await {
            return false;
        }
        win32::raise_window(window);
        for _ in 0..8 {
            if win32::is_foreground_window(window) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        return false;
    }
    if process.tty.is_none()
        || !process.windows_terminal_session.as_deref().is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
    {
        return false;
    }

    tokio::task::spawn_blocking(move || {
        // Keep the temporary title and its restore together. A second click
        // cannot use or overwrite another jump's title.
        static JUMP: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let Ok(_jump) = JUMP.try_lock() else { return false };
        let before = crate::windows_terminal::snapshot_tab_titles();
        if before.is_empty() {
            return false;
        }
        let marker = format!("CodeIsland-WSL-{}", uuid::Uuid::new_v4().simple());
        if !crate::wsl_console::write_title(&process, &marker) {
            return false;
        }
        let result = crate::windows_terminal::activate_marked_tab(&marker, &before);
        if let Some(title) = result.original_title {
            let _ = crate::wsl_console::write_title(&process, &title);
        }
        result.selected
    })
    .await
    .unwrap_or(false)
}

#[cfg(windows)]
pub fn foreground_is_wezterm() -> bool {
    win32::foreground_is_wezterm()
}

#[cfg(windows)]
pub(crate) fn windows_terminal_windows() -> Vec<isize> {
    win32::windows_terminal_windows()
}

pub fn is_foreground_process(pid: u32) -> bool {
    #[cfg(windows)]
    return win32::is_foreground_process(pid);
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

#[cfg(windows)]
mod win32 {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};
    use std::time::{Duration, Instant};

    use codeisland_domain::{CODEX_DESKTOP_SESSION_PREFIX, SessionSnapshot};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindowAsync,
    };
    use windows::core::BOOL;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Host {
        WindowsTerminal,
        WezTerm,
        Warp,
        Alacritty,
        Tabby,
        ConEmu,
        VSCode,
        VSCodeInsiders,
        Cursor,
        VSCodium,
        JetBrains,
        Zed,
        Codex,
        Claude,
    }

    impl Host {
        fn from_name(name: &str) -> Option<Self> {
            Some(
                match name.trim().to_ascii_lowercase().trim_end_matches(".exe") {
                    "windowsterminal" | "windows terminal" | "wt" => Self::WindowsTerminal,
                    "wezterm-gui" | "wezterm" => Self::WezTerm,
                    "warp" | "warp-stable" => Self::Warp,
                    "alacritty" => Self::Alacritty,
                    "tabby" => Self::Tabby,
                    "conemu" | "conemu64" => Self::ConEmu,
                    "code" | "vscode" => Self::VSCode,
                    "code - insiders" | "vscode-insiders" => Self::VSCodeInsiders,
                    "cursor" => Self::Cursor,
                    "vscodium" | "codium" => Self::VSCodium,
                    "jetbrains" | "idea64" | "pycharm64" | "webstorm64" | "rider64"
                    | "goland64" | "clion64" | "rustrover64" | "phpstorm64" => Self::JetBrains,
                    "zed" => Self::Zed,
                    "codex" => Self::Codex,
                    _ => return None,
                },
            )
        }

        fn from_process(name: &str, pid: u32) -> Option<Self> {
            if name.eq_ignore_ascii_case("Codex.exe") || name.eq_ignore_ascii_case("ChatGPT.exe") {
                return codeisland_runtime::is_codex_desktop_process(pid).then_some(Self::Codex);
            }
            if name.eq_ignore_ascii_case("Claude.exe") {
                return codeisland_runtime::is_claude_desktop_process(pid).then_some(Self::Claude);
            }
            Self::from_name(name)
        }

        fn app_is_session_surface(self) -> bool {
            matches!(
                self,
                Self::VSCode
                    | Self::VSCodeInsiders
                    | Self::Cursor
                    | Self::VSCodium
                    | Self::JetBrains
                    | Self::Zed
                    | Self::Codex
                    | Self::Claude
            )
        }
    }

    struct Process {
        parent: u32,
        host: Option<Host>,
    }

    type ProcessTable = HashMap<u32, Process>;

    static PROCESSES: LazyLock<Mutex<Option<(Instant, ProcessTable)>>> =
        LazyLock::new(|| Mutex::new(None));

    fn with_processes<T>(f: impl FnOnce(&ProcessTable) -> T) -> T {
        let mut cached = PROCESSES.lock().unwrap_or_else(|error| error.into_inner());
        if cached
            .as_ref()
            .is_none_or(|(updated, _)| updated.elapsed() >= Duration::from_millis(300))
        {
            *cached = Some((Instant::now(), read_processes()));
        }
        f(&cached
            .as_ref()
            .expect("process snapshot has been initialized")
            .1)
    }

    fn read_processes() -> ProcessTable {
        let mut processes = ProcessTable::new();
        // SAFETY: the snapshot is read-only, the entry is sized for the API,
        // and the handle is closed before returning.
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return processes;
            };
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut result = Process32FirstW(snapshot, &mut entry);
            while result.is_ok() {
                let name_len = entry
                    .szExeFile
                    .iter()
                    .position(|&ch| ch == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);
                processes.insert(
                    entry.th32ProcessID,
                    Process {
                        parent: entry.th32ParentProcessID,
                        host: Host::from_process(&name, entry.th32ProcessID),
                    },
                );
                result = Process32NextW(snapshot, &mut entry);
            }
            let _ = CloseHandle(snapshot);
        }
        processes
    }

    fn window_pid(window: HWND) -> u32 {
        let mut pid = 0;
        // SAFETY: Windows supplies the HWND and writes a single DWORD.
        unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
        pid
    }

    fn foreground_pid() -> Option<u32> {
        // SAFETY: this reads the currently active top-level HWND.
        let pid = window_pid(unsafe { GetForegroundWindow() });
        (pid != 0 && pid != std::process::id()).then_some(pid)
    }

    pub fn is_foreground_process(pid: u32) -> bool {
        foreground_pid() == Some(pid)
    }

    struct Target {
        pid: Option<u32>,
        owner: Option<(u32, Host)>,
        host: Option<Host>,
    }

    impl Target {
        fn new(session: &SessionSnapshot, processes: &ProcessTable) -> Self {
            let desktop = if session.session_id.starts_with(CODEX_DESKTOP_SESSION_PREFIX) {
                Some(Host::Codex)
            } else if super::is_claude_desktop_session(session) {
                Some(Host::Claude)
            } else {
                None
            };
            if let Some(host) = desktop {
                // Desktop threads belong to the packaged GUI, even if their
                // recorded process belongs to its CLI or a stopped helper.
                return Self {
                    pid: None,
                    owner: None,
                    host: Some(host),
                };
            }
            let hint = session.terminal_kind.as_deref().and_then(Host::from_name);
            let mut pid = session.pid;
            let mut owner = None;
            for _ in 0..24 {
                let Some((id, process)) = pid.and_then(|id| processes.get(&id).map(|p| (id, p)))
                else {
                    break;
                };
                if let Some(host) = process.host
                    && (hint == Some(host) || !matches!(host, Host::Codex | Host::Claude))
                {
                    if owner.is_some_and(|(_, previous)| previous != host) {
                        break;
                    }
                    // An Electron CLI or PTY helper can share the app's
                    // executable name. Walk up to the owning GUI process.
                    owner = Some((id, host));
                }
                pid = Some(process.parent);
            }
            Self {
                pid: session.pid,
                owner,
                host: owner.map(|(_, host)| host).or(hint),
            }
        }

        fn matches(&self, pid: u32, processes: &ProcessTable) -> bool {
            self.pid == Some(pid)
                || match self.owner {
                    Some((owner, _)) => owner == pid,
                    None => self.host.is_some_and(|host| {
                        processes
                            .get(&pid)
                            .is_some_and(|process| process.host == Some(host))
                    }),
                }
        }
    }

    pub fn is_frontmost(session: &SessionSnapshot) -> bool {
        foreground_pid().is_some_and(|pid| {
            with_processes(|processes| Target::new(session, processes).matches(pid, processes))
        })
    }

    pub fn is_tab_visible(session: &SessionSnapshot) -> bool {
        foreground_pid().is_some_and(|pid| {
            with_processes(|processes| {
                let target = Target::new(session, processes);
                target.matches(pid, processes)
                    && if target.host == Some(Host::WezTerm) {
                        crate::wezterm::is_active(session)
                    } else {
                        target.pid == Some(pid)
                            || target.host.is_some_and(Host::app_is_session_surface)
                    }
            })
        })
    }

    pub fn is_wezterm_session(session: &SessionSnapshot) -> bool {
        with_processes(|processes| Target::new(session, processes).host == Some(Host::WezTerm))
    }

    pub fn foreground_is_wezterm() -> bool {
        foreground_pid().is_some_and(|pid| {
            with_processes(|processes| {
                processes
                    .get(&pid)
                    .is_some_and(|process| process.host == Some(Host::WezTerm))
            })
        })
    }

    pub fn windows_terminal_windows() -> Vec<isize> {
        terminal_windows(Host::WindowsTerminal)
    }

    pub fn unique_wezterm_window() -> Option<isize> {
        let windows = terminal_windows(Host::WezTerm);
        (windows.len() == 1).then(|| windows[0])
    }

    fn terminal_windows(host: Host) -> Vec<isize> {
        let windows = visible_windows();
        with_processes(|processes| {
            windows
                .into_iter()
                .filter(|window| {
                    processes
                        .get(&window_pid(*window))
                        .is_some_and(|process| process.host == Some(host))
                })
                .map(|window| window.0 as isize)
                .collect()
        })
    }

    pub fn request_activation(session: &SessionSnapshot) -> Option<isize> {
        let windows = visible_windows();
        let hwnd = with_processes(|processes| {
            let target = Target::new(session, processes);
            let candidates = windows
                .iter()
                .copied()
                .filter(|window| target.matches(window_pid(*window), processes));
            let project = session
                .cwd
                .as_deref()
                .and_then(folder_name)
                .or_else(|| session.project.as_deref().and_then(folder_name));
            select_window(candidates, project, window_title)
        })?;
        let handle = hwnd.0 as isize;
        raise_window(handle);
        Some(handle)
    }

    pub fn raise_window(handle: isize) {
        let hwnd = HWND(handle as *mut std::ffi::c_void);
        // SAFETY: this action originates from a click or global shortcut.
        // The OS decides whether the target can take foreground focus.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn is_foreground_window(expected: isize) -> bool {
        // SAFETY: the returned window handle is compared as a value only.
        unsafe { GetForegroundWindow() }.0 as isize == expected
    }

    fn folder_name(path: &str) -> Option<&str> {
        path.trim()
            .rsplit(['/', '\\'])
            .find(|part| !part.is_empty())
    }

    /// Swift raises the shortest window title containing the workspace folder
    /// and falls back to application activation if there is no match.
    fn select_window<T: Copy>(
        mut candidates: impl Iterator<Item = T>,
        project: Option<&str>,
        title: impl Fn(T) -> String,
    ) -> Option<T> {
        let first = candidates.next()?;
        let Some(project) = project.map(str::to_lowercase) else {
            return Some(first);
        };
        std::iter::once(first)
            .chain(candidates)
            .filter_map(|window| {
                let title = title(window);
                title
                    .to_lowercase()
                    .contains(&project)
                    .then_some((window, title.chars().count()))
            })
            .min_by_key(|(_, length)| *length)
            .map(|(window, _)| window)
            .or(Some(first))
    }

    fn visible_windows() -> Vec<HWND> {
        unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
            // SAFETY: EnumWindows invokes this callback synchronously while
            // the caller owns the vector. HWND visibility is read-only.
            unsafe {
                if IsWindowVisible(window).as_bool() && window_pid(window) != std::process::id() {
                    (&mut *(data.0 as *mut Vec<HWND>)).push(window);
                }
            }
            BOOL(1)
        }

        let mut windows = Vec::<HWND>::new();
        // SAFETY: the vector lives for the duration of the synchronous call.
        unsafe {
            let _ = EnumWindows(
                Some(collect),
                LPARAM((&mut windows as *mut Vec<HWND>) as isize),
            );
        }
        windows
    }

    fn window_title(window: HWND) -> String {
        let mut title = [0u16; 512];
        // SAFETY: the destination is an initialized stack buffer.
        let len = unsafe { GetWindowTextW(window, &mut title) }.max(0) as usize;
        String::from_utf16_lossy(&title[..len])
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use codeisland_domain::{AgentEvent, AgentEventType, AgentKind};

        #[test]
        fn claude_desktop_targets_only_the_gui_and_terminal_claude_keeps_its_owner() {
            let mut session = SessionSnapshot::new(&AgentEvent::new(
                "cli-session",
                AgentKind::Claude,
                AgentEventType::SessionStart,
            ));
            let processes = HashMap::from([
                (11, Process { parent: 22, host: None }),
                (22, Process { parent: 1, host: Some(Host::WindowsTerminal) }),
                (33, Process { parent: 1, host: Some(Host::Claude) }),
            ]);
            session.pid = Some(11);
            session.terminal_kind = Some("claude".to_owned());
            let cli = Target::new(&session, &processes);
            assert!(cli.matches(22, &processes));
            assert!(!cli.matches(33, &processes));

            session.claude_desktop_session_id = Some("local_host-id".to_owned());
            let desktop = Target::new(&session, &processes);
            assert!(desktop.matches(33, &processes));
            assert!(!desktop.matches(11, &processes));
            assert!(!desktop.matches(22, &processes));

            session.claude_desktop_session_id = None;
            session.session_id = "cowork:local_cowork-id".to_owned();
            let cowork = Target::new(&session, &processes);
            assert!(cowork.matches(33, &processes));
            assert!(!cowork.matches(11, &processes));
        }

        #[test]
        fn multiple_host_windows_prefer_the_shortest_workspace_title() {
            let titles = [
                "Other project - Editor",
                "A long file name - workspace - Editor",
                "Workspace - Editor",
            ];
            let windows = || 0..titles.len();
            let title = |index: usize| titles[index].to_owned();
            assert_eq!(folder_name(r"C:\\work\\Workspace\\"), Some("Workspace"));
            assert_eq!(folder_name("/home/dev/Workspace/"), Some("Workspace"));
            assert_eq!(select_window(windows(), Some("Workspace"), title), Some(2));
            assert_eq!(select_window(windows(), Some("missing"), title), Some(0));
            assert_eq!(select_window(windows(), None, title), Some(0));
        }
    }
}
