//! Finds transcript directories belonging to the default user of each running
//! WSL distribution. The tailer reads the resulting Windows filesystem paths.

#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WslTranscriptSource {
    Codex,
    Claude,
}

#[cfg(windows)]
#[derive(Clone)]
pub(crate) struct WslDistribution {
    pub name: String,
    pub root: PathBuf,
    pub home: PathBuf,
    pub uid: u32,
}

#[cfg(windows)]
impl WslDistribution {
    pub fn transcript_roots(&self) -> [(WslTranscriptSource, PathBuf); 4] {
        [
            (WslTranscriptSource::Codex, self.home.join(r".codex\sessions")),
            (WslTranscriptSource::Claude, self.home.join(r".claude\projects")),
            (WslTranscriptSource::Claude, self.home.join(r".config\claude\projects")),
            (WslTranscriptSource::Claude, self.home.join(r".config\claude-code\projects")),
        ]
    }
}

#[cfg(windows)]
pub(crate) struct WslDiscovery {
    pub running: Vec<String>,
    pub distributions: Vec<WslDistribution>,
}

#[cfg(windows)]
pub(crate) async fn discover_distributions() -> Option<WslDiscovery> {
    let system_root = std::env::var_os("SystemRoot")?;
    let wsl = PathBuf::from(system_root).join("System32").join("wsl.exe");
    let output = run_wsl(&wsl, &["--list", "--running", "--quiet"]).await?;
    let running = parse_distributions(&output)?;
    if running.is_empty() {
        return Some(WslDiscovery { running, distributions: Vec::new() });
    }
    // Filesystem requests to a WSL share can block if the distribution exits
    // after the running-list snapshot. Keep one outstanding discovery batch.
    tokio::task::spawn_blocking(move || {
        let registered = registered_default_uids();
        let distributions = running.iter().cloned()
            .filter_map(|distro| {
                let uid = registered.iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(&distro))
                    .map(|(_, uid)| *uid);
                distribution(distro, uid)
            })
            .collect();
        WslDiscovery { running, distributions }
    })
    .await
    .ok()
}

#[cfg(windows)]
pub(crate) fn distribution_by_name(name: &str) -> Option<WslDistribution> {
    if !valid_component(name) || name.starts_with('-') {
        return None;
    }
    let uid = registered_default_uids().into_iter()
        .find(|(registered, _)| registered.eq_ignore_ascii_case(name))
        .map(|(_, uid)| uid);
    distribution(name.to_owned(), uid)
}

#[cfg(windows)]
fn registered_default_uids() -> Vec<(String, u32)> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW,
        RegGetValueW, RegOpenKeyExW,
    };
    use windows_sys::w;

    let mut key = null_mut();
    // SAFETY: The path is NUL-terminated and the output handle is writable.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Lxss"),
            0,
            KEY_READ,
            &mut key,
        )
    };
    if opened != ERROR_SUCCESS {
        return Vec::new();
    }

    let mut registered = Vec::new();
    for index in 0.. {
        let mut subkey = [0u16; 256];
        let mut subkey_len = subkey.len() as u32;
        let mut name = [0u16; 512];
        let mut name_bytes = size_of_val(&name) as u32;
        let mut uid = 0u32;
        let mut uid_bytes = size_of_val(&uid) as u32;
        // SAFETY: All buffers remain live and the sizes use the requested units.
        // Registry enumeration returns a NUL-terminated subkey name on success.
        let (listed, named) = unsafe {
            let listed = RegEnumKeyExW(
                key, index, subkey.as_mut_ptr(), &mut subkey_len, null(), null_mut(),
                null_mut(), null_mut(),
            );
            let named = if listed == ERROR_SUCCESS {
                RegGetValueW(
                    key, subkey.as_ptr(), w!("DistributionName"), RRF_RT_REG_SZ, null_mut(),
                    name.as_mut_ptr().cast(), &mut name_bytes,
                )
            } else {
                listed
            };
            if named == ERROR_SUCCESS {
                // An imported distribution without an explicit UID defaults to root.
                let _ = RegGetValueW(
                    key, subkey.as_ptr(), w!("DefaultUid"), RRF_RT_REG_DWORD, null_mut(),
                    (&mut uid as *mut u32).cast(), &mut uid_bytes,
                );
            }
            (listed, named)
        };
        if listed != ERROR_SUCCESS {
            break;
        }
        if named == ERROR_SUCCESS {
            let len = name.iter().position(|&unit| unit == 0).unwrap_or(name.len());
            if let Ok(name) = String::from_utf16(&name[..len]) {
                registered.push((name, uid));
            }
        }
    }
    // SAFETY: This function exclusively owns the successfully opened key.
    unsafe { RegCloseKey(key) };
    registered
}

#[cfg(windows)]
fn distribution(distro: String, uid: Option<u32>) -> Option<WslDistribution> {
    use std::{fs::File, io::Read, path::Path};

    fn read_config(path: &Path) -> Option<String> {
        const LIMIT: u64 = 256 * 1024;
        let mut content = String::new();
        File::open(path).ok()?.take(LIMIT + 1).read_to_string(&mut content).ok()?;
        (content.len() as u64 <= LIMIT).then_some(content)
    }

    for server in ["wsl.localhost", "wsl$"] {
        let root = PathBuf::from(format!(r"\\{server}\{distro}\"));
        let etc = root.join("etc");
        let Some(passwd) = read_config(&etc.join("passwd")) else {
            continue;
        };
        let configuration = read_config(&etc.join("wsl.conf"));
        let Some((uid, home)) = default_home(&passwd, configuration.as_deref().and_then(configured_user), uid)
        else {
            return None;
        };
        let home = home.split('/').fold(root.clone(), |path, component| path.join(component));
        return Some(WslDistribution { name: distro, root, home, uid });
    }
    None
}

#[cfg(windows)]
async fn run_wsl(program: &std::path::Path, args: &[&str]) -> Option<Vec<u8>> {
    use std::{process::Stdio, time::Duration};
    use tokio::{io::AsyncReadExt, process::Command, time::timeout};

    const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
    const MAX_OUTPUT: u64 = 64 * 1024;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut command = Command::new(program);
    command
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    timeout(COMMAND_TIMEOUT, async {
        let mut child = command.spawn().ok()?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()?
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return None;
        }
        child.wait().await.ok()?.success().then_some(bytes)
    })
    .await
    .ok()
    .flatten()
}

fn parse_distributions(bytes: &[u8]) -> Option<Vec<String>> {
    // wsl.exe --list normally emits UTF-16LE without a BOM when redirected.
    // UTF-8 is also possible, for example with WSL_UTF8 enabled.
    let text = if bytes.starts_with(b"\xff\xfe") || bytes.contains(&0) {
        let bytes = bytes.strip_prefix(b"\xff\xfe").unwrap_or(bytes);
        let (words, remainder) = bytes.as_chunks::<2>();
        if !remainder.is_empty() {
            return None;
        }
        let units = words.iter().copied().map(u16::from_le_bytes).collect::<Vec<_>>();
        String::from_utf16(&units).ok()
    } else {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        std::str::from_utf8(bytes).ok().map(ToOwned::to_owned)
    }?;
    let mut names: Vec<_> = text
        .lines()
        .filter(|name| !name.starts_with('-') && valid_component(name))
        .map(ToOwned::to_owned)
        .collect();
    names.sort_unstable();
    names.dedup();
    Some(names)
}

fn configured_user(config: &str) -> Option<&str> {
    let mut user_section = false;
    for raw in config.trim_start_matches('\u{feff}').lines() {
        let line = raw.split(['#', ';']).next().unwrap_or_default().trim();
        if let Some(section) = line.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
            user_section = section.trim().eq_ignore_ascii_case("user");
        } else if user_section
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("default")
        {
            let user = value.trim();
            if !user.is_empty() {
                return Some(user);
            }
        }
    }
    None
}

fn default_home<'a>(passwd: &'a str, user: Option<&str>, uid: Option<u32>) -> Option<(u32, &'a str)> {
    let mut by_uid = None;
    for line in passwd.lines() {
        let mut fields = line.split(':');
        let (Some(name), Some(_), Some(record_uid)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Some(home) = fields.nth(2)
            .and_then(|home| home.strip_prefix('/'))
            .filter(|path| path.split('/').all(valid_component))
        else {
            continue;
        };
        let Ok(record_uid) = record_uid.parse::<u32>() else { continue };
        if Some(name) == user {
            return Some((record_uid, home));
        }
        if by_uid.is_none() && Some(record_uid) == uid {
            by_uid = Some((record_uid, home));
        }
    }
    by_uid
}

pub(crate) fn valid_component(name: &str) -> bool {
    !name.is_empty()
        && !name.ends_with(['.', ' '])
        && !name.contains(['\\', '/', ':', '<', '>', '"', '|', '?', '*'])
        && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_distro_lists_with_utf16_or_utf8_output() {
        let listed = "Ubuntu 24.04\r\n开发\r\nUbuntu 24.04\r\n";
        let utf16: Vec<_> = listed.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let expected = ["Ubuntu 24.04", "开发"];
        assert_eq!(parse_distributions(&utf16).unwrap(), expected);
        assert_eq!(
            parse_distributions(&[b"\xff\xfe".as_slice(), &utf16].concat()).unwrap(),
            expected,
        );
        assert_eq!(parse_distributions(listed.as_bytes()).unwrap(), expected);
        assert_eq!(
            parse_distributions(&[b"\xef\xbb\xbf".as_slice(), listed.as_bytes()].concat()).unwrap(),
            expected,
        );
        assert_eq!(
            parse_distributions(b"Ubuntu\r\n..\r\nfoo/bar\r\n--shutdown\r\n").unwrap(),
            ["Ubuntu"],
        );
        assert_eq!(parse_distributions(b""), Some(Vec::new()));
        assert_eq!(parse_distributions(b"\xff\xfeX"), None);
    }

    #[test]
    fn resolves_the_configured_linux_user_then_the_registered_uid() {
        let passwd = concat!(
            "root:x:0:0:root:/root:/bin/bash\n",
            "alice:x:1000:1000::/srv/é:/bin/sh\n",
            "bob:x:1001:1001::/home/bob:/bin/sh\n",
            "invalid:x:1002:1002::/home/../escape:/bin/sh\n",
            "invalid2:x:1003:1003::/home/has\\slash:/bin/sh\n",
        );
        let user = configured_user("[boot]\ndefault=wrong\n[ user ] # comment\ndefault = bob ; comment\n");
        assert_eq!(user, Some("bob"));
        assert_eq!(default_home(passwd, user, Some(1000)), Some((1001, "home/bob")));
        assert_eq!(default_home(passwd, Some("stale"), Some(1000)), Some((1000, "srv/é")));
        assert_eq!(default_home(passwd, None, Some(0)), Some((0, "root")));
        assert_eq!(default_home(passwd, None, Some(1002)), None);
        assert_eq!(default_home(passwd, None, Some(1003)), None);
        assert_eq!(default_home(passwd, None, None), None);
    }
}
