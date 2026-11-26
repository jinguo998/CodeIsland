use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use codeisland_domain::{AppSettings, UserAction};
use codeisland_runtime::Runtime;

fn spawn_hook(connect_timeout_ms: &str) -> std::process::Child {
    let fixture = include_str!("../../../../tests/fixtures/claude-permission.json");
    let mut child = Command::new(env!("CARGO_BIN_EXE_codeisland-hook"))
        .args([
            "send",
            "--source",
            "claude",
            "--event",
            "PermissionRequest",
            "--stdin",
            "--connect-timeout-ms",
            connect_timeout_ms,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hook");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(fixture.as_bytes()).expect("write fixture");
    }
    child
}

/// Full chain: hook child process -> IPC socket -> runtime -> user decision -> hook stdout.
/// The no-island case runs first so it cannot race a live server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_ipc_round_trip_and_fail_open() {
    // No island yet: the hook must exit 0 with no output instead of blocking.
    let offline = spawn_hook("250")
        .wait_with_output()
        .expect("offline hook exits");
    assert_eq!(offline.status.code(), Some(0));
    let offline_stdout = String::from_utf8_lossy(&offline.stdout);
    assert!(
        !offline_stdout.contains("behavior"),
        "offline hook must not invent a decision: {offline_stdout:?}"
    );

    let runtime = Runtime::new(AppSettings::default());
    let _handle = runtime.start();
    tokio::time::sleep(Duration::from_millis(400)).await;

    let child = spawn_hook("1000");
    let mut resolved = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if runtime
            .state()
            .store
            .pending
            .iter()
            .any(|request| !request.hidden)
        {
            let decision = runtime.resolve_current(UserAction::Approve, None);
            assert!(decision.is_some(), "resolve must return a decision");
            resolved = true;
            break;
        }
    }
    assert!(resolved, "the permission request never reached the runtime");

    let output = child.wait_with_output().expect("hook exits");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"behavior\":\"allow\""),
        "hook stdout was {stdout:?} (stderr {:?})",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));
}
