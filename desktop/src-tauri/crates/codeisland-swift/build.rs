use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(4).unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // CI builds the hook, tests and installer sequentially in one checkout.
    // Reuse SwiftPM's debug/release products there. Independent Cargo
    // invocations retain their own staging directories by default.
    println!("cargo:rerun-if-env-changed=CODEISLAND_SWIFT_SCRATCH");
    let scratch = env::var_os("CODEISLAND_SWIFT_SCRATCH").map(PathBuf::from)
        .unwrap_or_else(|| output.join("swiftpm"));
    let profile = if env::var("PROFILE").unwrap() == "debug" { "debug" } else { "release" };
    for input in ["Sources/CodeIslandCore", "Sources/CodeIsland", "desktop/scripts/swift_sources.py", "Tests/CodeIslandCoreTests", "Tests/CodeIslandTests", "desktop/swift", "desktop/scripts/build-swift.py", "desktop/scripts/stage-swift-runtime.ps1"] {
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
    let stage = output.join("swift-core");
    println!("cargo:rustc-env=CODEISLAND_BUILT_SWIFT_DIR={}", stage.display());
    let profile_dir = output.ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "build"))
        .and_then(Path::parent).unwrap_or(&output);
    println!("cargo:rustc-env=CODEISLAND_CARGO_PROFILE_DIR={}", profile_dir.display());
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
