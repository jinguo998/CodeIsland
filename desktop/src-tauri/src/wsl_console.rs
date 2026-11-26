use std::{fs::OpenOptions, io::Write};

use codeisland_domain::WslProcess;

/// Send a title sequence to the verified PTY's output. Opening the slave
/// device for writing does not inject keystrokes into the agent.
pub(crate) fn write_title(process: &WslProcess, title: &str) -> bool {
    let Some(tty) = process.tty.as_deref().and_then(pty_number) else {
        return false;
    };
    let Some(sequence) = title_sequence(title) else {
        return false;
    };
    if !codeisland_runtime::wsl_process_is_current(process) {
        return false;
    }

    for server in ["wsl.localhost", "wsl$"] {
        let path = format!(r"\\{server}\{}\dev\pts\{tty}", process.distro);
        if let Ok(mut tty_output) = OpenOptions::new().write(true).open(path) {
            return tty_output.write_all(sequence.as_bytes()).is_ok();
        }
    }
    false
}

fn pty_number(tty: &str) -> Option<u32> {
    let number = tty.strip_prefix("/dev/pts/")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

fn title_sequence(title: &str) -> Option<String> {
    if title.chars().any(char::is_control) || title.encode_utf16().count() >= 255 {
        return None;
    }
    Some(format!("\u{1b}]0;{title}\u{1b}\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_a_linux_pseudoterminal_device() {
        assert_eq!(pty_number("/dev/pts/10"), Some(10));
        for tty in ["/dev/tty", "/dev/pts/../tty", "/dev/pts/+1", "/dev/pts/10/file"] {
            assert_eq!(pty_number(tty), None);
        }
    }

    #[test]
    fn writes_one_title_sequence_and_rejects_control_characters() {
        assert_eq!(title_sequence("CodeIsland 终端").as_deref(), Some("\u{1b}]0;CodeIsland 终端\u{1b}\\"));
        assert!(title_sequence("invalid\u{1b}").is_none());
        assert!(title_sequence(&"x".repeat(255)).is_none());
    }
}
