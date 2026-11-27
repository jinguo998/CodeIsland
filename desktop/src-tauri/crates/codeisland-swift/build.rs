use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(4).unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let scratch = output.join("swiftpm");
    let profile = if env::var("PROFILE").unwrap() == "debug" { "debug" } else { "release" };
    for input in ["Sources/CodeIslandCore", "Tests/CodeIslandCoreTests", "desktop/swift", "desktop/scripts/build-swift.py", "desktop/scripts/stage-swift-runtime.ps1"] {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    assert_eq!(env::var("HOST"), env::var("TARGET"), "Build the Swift and Rust targets on the same host");
    let build = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .arg(root.join("desktop/scripts/build-swift.py")).arg("build")
        .arg("--scratch").arg(&scratch).args(["--configuration", profile])
        .output().expect("Python 3 is required to build the Swift core");
    assert!(build.status.success(), "Swift build failed\n{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8(build.stdout).expect("SwiftPM output");
    let name = format!("{}CodeIslandDesktopCore{}", env::consts::DLL_PREFIX, env::consts::DLL_SUFFIX);
    let source = Path::new(bin.trim()).join(&name);
    // Cargo runs unit/integration binaries from <profile>/deps and the app and
    // its hooks from <profile>. Installed executables use the same layout.
    let stage = output.ancestors().nth(3).unwrap().join("swift-core");
    fs::create_dir_all(&stage).unwrap();
    fs::copy(&source, stage.join(&name)).expect("stage Swift core");
    if cfg!(windows) {
        let status = Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(root.join("desktop/scripts/stage-swift-runtime.ps1"))
            .arg(&source).arg(&stage).status().expect("stage Windows Swift runtime");
        assert!(status.success(), "Unable to stage Windows Swift runtime");
    }
}
