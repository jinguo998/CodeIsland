//! Path resolution for agent configs, backups and the hook executable.

use std::path::{Path, PathBuf};

use codeisland_domain::{AgentKind, AppSettings};

use crate::definitions::AgentDefinition;

/// Resolved user paths. Tests can build one with [`Paths::with_home`] instead
/// of touching the real profile.
#[derive(Debug, Clone)]
pub struct Paths {
    home: PathBuf,
    data_local: PathBuf,
    extra_dirs: Vec<PathBuf>,
    claude_config_preference: String,
    codex_config_preference: Option<String>,
}

impl Paths {
    pub fn current() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let data_local = env_nonempty("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        let extra_dirs = env_nonempty("CODEISLAND_EXTRA_CONFIG_DIRS")
            .map(|value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            home,
            data_local,
            extra_dirs,
            claude_config_preference: String::new(),
            codex_config_preference: env_nonempty("CODEX_HOME"),
        }
    }

    /// A hermetic resolver rooted at `home`.
    pub fn with_home(home: PathBuf) -> Self {
        let data_local = home.join("AppData").join("Local");
        let extra_dirs = env_nonempty("CODEISLAND_EXTRA_CONFIG_DIRS")
            .map(|value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            home,
            data_local,
            extra_dirs,
            claude_config_preference: String::new(),
            codex_config_preference: None,
        }
    }

    /// Applies preferences without changing the process environment.
    pub fn with_settings(mut self, settings: &AppSettings) -> Self {
        self.claude_config_preference = settings.claude_config_dir.clone();
        self.extra_dirs = settings.extra_config_dirs
            .iter()
            .filter(|raw| !raw.trim().is_empty())
            .map(|raw| self.expand(raw))
            .collect();
        self
    }

    pub fn claude_config_dir(&self) -> PathBuf {
        self.resolve_claude_config_dir(&self.claude_config_preference)
    }

    pub fn codex_config_dir(&self) -> PathBuf {
        self.codex_config_preference
            .as_deref()
            .map(|path| self.expand(path))
            .unwrap_or_else(|| self.home.join(".codex"))
    }

    pub fn resolve_claude_config_dir(&self, preference: &str) -> PathBuf {
        self.resolve_claude_config_dir_from(
            preference,
            env_nonempty("CLAUDE_CONFIG_DIR").as_deref().unwrap_or_default(),
        )
    }

    fn resolve_claude_config_dir_from(&self, preference: &str, environment: &str) -> PathBuf {
        // Expand Windows variables and validate drive/UNC roots before calling
        // ClaudeConfigPaths, whose Unix absolute-path check only accepts '/'.
        let expanded = |raw: &str| {
            let raw = raw.trim();
            if let Some(rest) = raw.strip_prefix("~\\") {
                Some(self.home.join(rest).to_string_lossy().into_owned())
            } else if raw.starts_with('%') {
                expand_env(raw)
            } else {
                Some(raw.to_owned())
            }
        };
        let preference = expanded(preference);
        let environment = expanded(environment);
        #[cfg(windows)]
        for candidate in [preference.as_deref(), environment.as_deref()].into_iter().flatten() {
            let path = if candidate == "~" { self.home.clone() }
                else if let Some(rest) = candidate.strip_prefix("~/") { self.home.join(rest) }
                else { PathBuf::from(candidate) };
            if path.is_absolute() && path.parent().is_some() { return path; }
        }
        let resolved: String = codeisland_swift::call(c"install.claudeRoot", serde_json::json!({
            "home": self.home, "preference": preference, "environment": environment,
        }));
        PathBuf::from(resolved)
    }

    fn extra_claude_config_dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.extra_dirs.iter().map(|dir| {
            let nested = dir.join(".claude");
            if nested.is_dir() {
                return nested;
            }
            if dir.file_name().is_some_and(|name| name == ".claude")
                || dir.join("projects").is_dir()
            {
                return dir.clone();
            }
            nested
        })
    }

    pub fn claude_config_dirs(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.claude_config_dir()];
        for path in self.extra_claude_config_dirs() {
            if !roots.contains(&path) {
                roots.push(path);
            }
        }
        roots
    }

    /// The default store can still contain active sessions launched without
    /// the override used for hook installation.
    pub fn claude_discovery_dirs(&self) -> Vec<PathBuf> {
        let mut roots = self.claude_config_dirs();
        let default = self.resolve_claude_config_dir_from("", "");
        if !roots.contains(&default) {
            roots.push(default);
        }
        roots
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn data_local(&self) -> &Path {
        &self.data_local
    }

    /// Expand `%USERPROFILE%`, `~`, `$HOME`, other `%VAR%` references and
    /// relative paths.
    pub fn expand(&self, raw: &str) -> PathBuf {
        let raw = raw.trim();
        let expanded = if let Some(rest) = raw.strip_prefix("%USERPROFILE%") {
            format!("{}{}", self.home.display(), rest)
        } else if let Some(rest) = raw.strip_prefix("%LOCALAPPDATA%") {
            format!("{}{}", self.data_local.display(), rest)
        } else if let Some(rest) = raw.strip_prefix("%APPDATA%") {
            format!("{}{}", self.home.join("AppData").join("Roaming").display(), rest)
        } else if let Some(rest) = raw.strip_prefix("$HOME") {
            format!("{}{}", self.home.display(), rest)
        } else if raw == "~" {
            self.home.display().to_string()
        } else if let Some(rest) = raw.strip_prefix("~/") {
            self.home.join(rest).display().to_string()
        } else if raw.starts_with('%') {
            expand_env(raw).unwrap_or_else(|| raw.to_owned())
        } else {
            raw.to_owned()
        };

        let path = PathBuf::from(expanded);
        if path.is_absolute() {
            path
        } else {
            self.home.join(path)
        }
    }

    pub fn config_paths(&self, def: &AgentDefinition) -> Vec<PathBuf> {
        if def.agent == AgentKind::Claude {
            return vec![self.claude_config_dir().join("settings.json")];
        }
        if def.agent == AgentKind::Codex {
            return vec![self.codex_config_dir().join("hooks.json")];
        }
        def.config_paths
            .iter()
            .map(|path| config_candidate(def, self.expand(path)))
            .collect()
    }

    /// Config candidates under the user's extra config roots. The definition's
    /// path is treated as relative to each root.
    pub fn extra_config_paths(&self, def: &AgentDefinition) -> Vec<PathBuf> {
        if def.agent == AgentKind::Claude {
            return self.extra_claude_config_dirs().map(|root| root.join("settings.json")).collect();
        }
        self.extra_dirs
            .iter()
            .flat_map(|dir| {
                def.config_paths.iter().map(move |raw| {
                    let relative = raw
                        .trim_start_matches("%USERPROFILE%")
                        .trim_start_matches("~/")
                        .trim_start_matches('/')
                        .trim_start_matches('\\');
                    config_candidate(def, dir.join(relative))
                })
            })
            .collect()
    }

    /// The config file an installer should write.
    ///
    /// Prefers an existing file, then a path whose parent exists (the tool is
    /// installed but the file has not been created yet), then the first path.
    pub fn primary_config(&self, def: &AgentDefinition) -> Option<PathBuf> {
        if def.agent == AgentKind::Claude {
            return Some(self.claude_config_dir().join("settings.json"));
        }
        if let Some(path) = self
            .extra_config_paths(def)
            .into_iter()
            .find(|path| path.exists())
        {
            return Some(path);
        }
        let paths = self.config_paths(def);
        if paths.is_empty() {
            return None;
        }
        if let Some(path) = paths.iter().find(|path| path.exists()) {
            return Some(path.clone());
        }
        if let Some(path) = paths
            .iter()
            .find(|path| path.parent().is_some_and(|parent| parent.exists()))
        {
            return Some(path.clone());
        }
        paths.into_iter().next()
    }

    /// `%LOCALAPPDATA%\CodeIsland\backups\<agent>\`
    pub fn backup_dir(&self, agent: AgentKind) -> PathBuf {
        self.data_local
            .join("CodeIsland")
            .join("backups")
            .join(agent.tag())
    }

    /// The hook executable, if one exists.
    ///
    /// Resolution order: `CODEISLAND_HOOK`, a sibling of the current
    /// executable, then `PATH`.
    pub fn hook_exe(&self) -> Option<PathBuf> {
        if let Some(raw) = env_nonempty("CODEISLAND_HOOK") {
            let path = self.expand(&raw);
            if path.is_file() {
                return Some(path);
            }
        }

        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                for name in hook_names() {
                    let candidate = dir.join(name);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }

        for name in hook_names() {
            if let Some(path) = find_on_path(name) {
                return Some(path);
            }
        }
        None
    }

    /// The base command every installed hook entry receives.
    ///
    /// The `--source` value is the per-agent marker (`codeisland-claude`).
    /// `kind_for_source` strips the prefix, so the hook still reports the
    /// canonical [`AgentKind`] while the command string remains recognizable
    /// and stable for uninstall/health checks.
    pub fn hook_command(&self, def: &AgentDefinition) -> String {
        format!(
            "{} send --source {} --stdin",
            self.hook_program(),
            def.marker()
        )
    }

    /// Base command plus the event name. Every supported CLI gets `--event`
    /// because some of them omit `hook_event_name` from stdin.
    pub fn hook_command_for_event(&self, def: &AgentDefinition, event: &str) -> String {
        format!(
            "{} send --source {} --event {} --stdin",
            self.hook_program(),
            def.marker(),
            event
        )
    }

    fn hook_program(&self) -> String {
        if let Some(path) = self.hook_exe() {
            return quote_path(&path);
        }
        if let Some(raw) = env_nonempty("CODEISLAND_HOOK") {
            return quote_path(Path::new(&raw));
        }
        "codeisland-hook".to_owned()
    }

    /// The hook executable as a bare path (no shell quoting), for generated
    /// plugins that spawn it with `execFile` and an argument array.
    pub fn hook_program_path(&self) -> String {
        if let Some(path) = self.hook_exe() {
            return path.display().to_string();
        }
        if let Some(raw) = env_nonempty("CODEISLAND_HOOK") {
            return raw;
        }
        "codeisland-hook".to_owned()
    }
}

/// Follow a symlink (or a symlinked parent directory) so an installer writes
/// the file the link points at instead of replacing the link with a regular
/// file.
pub(crate) fn resolve_write_path(path: &Path) -> PathBuf {
    if path.is_symlink() {
        if let Ok(target) = std::fs::canonicalize(path) {
            return target;
        }
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(parent) = std::fs::canonicalize(parent) {
            return parent.join(name);
        }
    }
    path.to_path_buf()
}

fn config_candidate(def: &AgentDefinition, path: PathBuf) -> PathBuf {
    if def.agent == AgentKind::OpenCode && path.file_name().is_some_and(|name| name == "opencode.json") {
        let selected: String = codeisland_swift::call(c"install.opencodeTarget", serde_json::json!({
            "json": path, "jsonc": path.with_extension("jsonc"),
        }));
        return PathBuf::from(selected);
    }
    path
}

fn hook_names() -> [&'static str; 2] {
    if cfg!(windows) {
        ["codeisland-hook.exe", "codeisland-hook"]
    } else {
        ["codeisland-hook", "codeisland-hook.exe"]
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Whether `name` (with the platform's executable extensions on Windows)
/// resolves on `PATH`.
pub(crate) fn executable_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        if dir.join(name).is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            for extension in [".exe", ".cmd", ".bat"] {
                if dir.join(format!("{name}{extension}")).is_file() {
                    return true;
                }
            }
        }
    }
    false
}

fn quote_path(path: &Path) -> String {
    let text = path.display().to_string();
    if text.contains(' ') || text.contains('\t') {
        format!("\"{text}\"")
    } else {
        text
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Expand `%NAME%` references using the process environment.
fn expand_env(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        let name = &after[..end];
        if name.is_empty() {
            out.push('%');
        } else {
            out.push_str(&std::env::var(name).ok()?);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_root_precedence_uses_projects_instead_of_installer_output() {
        let home = std::env::temp_dir().join(format!("codeisland-claude-root-{}", uuid::Uuid::new_v4()));
        let default = home.join(".claude");
        let xdg = home.join(".config").join("claude-code");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("settings.json"), "{}").unwrap();
        std::fs::create_dir_all(xdg.join("projects")).unwrap();
        let paths = Paths::with_home(home.clone());

        assert_eq!(paths.resolve_claude_config_dir_from("", ""), xdg);
        assert_eq!(
            paths.resolve_claude_config_dir_from("relative", "~/environment"),
            home.join("environment"),
        );
        assert_eq!(
            paths.resolve_claude_config_dir_from("~/preferred", "~/environment"),
            home.join("preferred"),
        );
        std::fs::create_dir_all(default.join("projects")).unwrap();
        assert_eq!(paths.resolve_claude_config_dir_from("", ""), default);
        let preferred = home.join("preferred");
        let extra = home.join("extra").join(".claude");
        let paths = paths.with_settings(&AppSettings {
            claude_config_dir: preferred.to_string_lossy().into_owned(),
            extra_config_dirs: vec![extra.to_string_lossy().into_owned()],
            ..AppSettings::default()
        });
        assert_eq!(paths.claude_config_dirs(), [preferred.clone(), extra.clone()]);
        assert_eq!(paths.claude_discovery_dirs(), [preferred, extra, default]);
        std::fs::remove_dir_all(home).unwrap();
    }
}
