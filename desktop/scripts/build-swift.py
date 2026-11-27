#!/usr/bin/env python3
"""Build the macOS core sources and their original tests for the native desktop host."""
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BRIDGE = ROOT / "desktop/swift"
CORE = (
    "ClaudeConfigPaths", "ConfigPathIdentity", "ExtraConfigDirs", "ClaudeDesktopCodeSession", "ClaudeQuota", "ClaudeQuotaClient", "ClaudeQuotaScheduler", "ClaudeUsageScanner", "ModelLabel",
    "EventNormalizer", "FollowUpReminderScheduler", "RemoteReconnectPolicy",
)
TESTS = ("ClaudeQuotaTests", "ClaudeQuotaSchedulerTests", "FollowUpReminderSchedulerTests",
         "RemoteReconnectPolicyTests", "ClaudeUsageScannerTests", "ModelLabelTests")


def stage(sources, destination):
    destination.mkdir(parents=True, exist_ok=True)
    names = {source.name for source in sources}
    for source in sources:
        output = destination / source.name
        if not output.exists() or source.read_bytes() != output.read_bytes():
            shutil.copyfile(source, output)
    for stale in destination.glob("*.swift"):
        if stale.name not in names:
            stale.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "test"))
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--configuration", choices=("debug", "release"), default="debug")
    args = parser.parse_args()
    package = args.scratch.resolve() / "package"
    # SwiftPM requires every target to be inside its package. The staging area
    # works across volumes on Windows without symlink privileges. It never
    # writes the macOS package or its dependency lockfile.
    stage([BRIDGE / "Package.swift"], package)
    stage([ROOT / "Sources/CodeIslandCore" / f"{name}.swift" for name in CORE], package / "Sources/CodeIslandCore")
    stage(sorted((BRIDGE / "Sources").glob("*.swift")), package / "Sources/CodeIslandDesktopCore")
    stage([ROOT / "Tests/CodeIslandCoreTests" / f"{name}.swift" for name in TESTS], package / "Tests/CodeIslandCoreTests")
    base = ["swift", args.action, "--package-path", str(package), "-c", args.configuration]
    if jobs := os.environ.get("NUM_JOBS"):
        base += ["--jobs", jobs]
    if args.action == "test":
        subprocess.run(base, check=True)
    else:
        subprocess.run(base + ["--product", "CodeIslandDesktopCore"], check=True, stdout=sys.stderr)
        subprocess.run(base + ["--show-bin-path"], check=True)


if __name__ == "__main__":
    main()
