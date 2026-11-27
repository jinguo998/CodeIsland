use std::{env, path::Path, process::Command};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(4).unwrap();
    for path in ["Sources/CodeIsland/Resources", "desktop/scripts/plugin_sources.py", "desktop/scripts/plugin-transport.js"] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let status = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .arg(root.join("desktop/scripts/plugin_sources.py"))
        .arg(env::var_os("OUT_DIR").unwrap()).status().expect("Python 3 is required for the plugin resources");
    assert!(status.success(), "Unable to stage original agent plugins");
}
