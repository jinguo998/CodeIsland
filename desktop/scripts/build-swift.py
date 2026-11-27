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
    "AgentTaskList", "AgentTaskParsing", "AttentionPolicy", "BackgroundCompactionFilter",
    "CoworkSession", "CoworkAuditLog", "CodexThreadSubsession", "CursorHookPayload", "CursorSessionFolding", "DeclaredSubsession",
    "EventSoundRouting", "SceneMuteState", "HermesHookPayload", "HermesSessionStore", "HostHarness", "JSONLTailer",
    "Models",
    "PushChannel", "PushNotification", "PushPolicy", "IdleSessionRemoval", "SessionListOrdering", "SessionRecap", "SessionSnapshot",
    "SubsessionLink", "SubsessionRouter", "WarpPaneResolver",
    "EventNormalizer", "FollowUpReminderScheduler", "RemoteReconnectPolicy",
)
TESTS = ("ClaudeQuotaTests", "ClaudeQuotaSchedulerTests", "FollowUpReminderSchedulerTests",
         "RemoteReconnectPolicyTests", "ClaudeUsageScannerTests", "ModelLabelTests",
         "CoworkSessionTests", "CoworkAuditLogTests", "AgentTaskListTests", "AttentionPolicyTests", "DerivedSessionStateTests",
         "HookEventFactsTests", "HookEventToolUseIdTests", "OutOfOrderToolStepTests",
         "PushChannelPayloadTests", "PushRedactionTests", "PushPolicyTests", "ReplyCompletePlaceholderTests",
         "SessionRecapTests", "SubsessionReducerTests", "SubsessionRouterTests", "SubsessionTestSupport")


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
    parser.add_argument("action", choices=("build", "test", "macos-test"))
    parser.add_argument("--scratch", type=Path)
    parser.add_argument("--configuration", choices=("debug", "release"), default="debug")
    args = parser.parse_args()
    if args.action == "macos-test":
        subprocess.run(["swift", "test", "--package-path", str(ROOT), "--filter", "|".join((*TESTS, "SessionListOrderingTests", "IdleSessionRemovalTests"))], check=True)
        return
    if args.scratch is None:
        parser.error("--scratch is required for a desktop build")
    package = args.scratch.resolve() / "package"
    # SwiftPM requires every target to be inside its package. The staging area
    # works across volumes on Windows without symlink privileges. It never
    # writes the macOS package or its dependency lockfile.
    stage([BRIDGE / "Package.swift", BRIDGE / "Package.resolved"], package)
    stage([ROOT / "Sources/CodeIslandCore" / f"{name}.swift" for name in CORE], package / "Sources/CodeIslandCore")
    stage(sorted((BRIDGE / "DesktopSQLite").glob("*")), package / "Sources/DesktopSQLite")
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
