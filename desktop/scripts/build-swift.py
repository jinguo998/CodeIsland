#!/usr/bin/env python3
"""Build the macOS core sources and their original tests for the native desktop host."""
import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BRIDGE = ROOT / "desktop/swift"
CORE = (
    "ClaudeSessionRecord", "ClaudeConfigPaths", "ConfigPathIdentity", "ExtraConfigDirs", "ClaudeDesktopCodeSession", "ClaudeQuota", "ClaudeQuotaClient", "ClaudeQuotaScheduler", "ClaudeUsageScanner", "ModelLabel",
    "GitBranchReader", "ESP32Protocol", "AppleCompanionPayload", "AppleCompanionBluetooth", "AgentTaskList", "AgentTaskParsing", "AttentionPolicy", "BackgroundCompactionFilter",
    "SQLiteReader", "CodexStateStore", "CodexRollout", "CoworkSession", "CoworkAuditLog", "CodexAppServerProtocol", "CodexHooksConfig", "CodexPermissionRules", "CodexThreadSubsession", "CursorHookPayload", "CursorSessionFolding", "DeclaredSubsession",
    "EventSoundRouting", "SoundVolumeCurve", "SceneMuteState", "HermesHookPayload", "HermesSessionStore", "HostHarness", "JSONLTailer",
    "Models",
    "PushTransport", "PushChannel", "PushNotification", "PushPolicy", "IdleSessionRemoval", "SessionListOrdering", "SessionRecap", "SessionSnapshot",
    "SubsessionLink", "SubsessionRouter", "WarpPaneResolver",
    "HookIntake", "HookEdgeFacts", "AntigravityHookContract", "GrokHookForwardingPolicy", "GrokSessionPaths",
    "EventNormalizer", "FollowUpReminderScheduler", "RemoteReconnectPolicy",
)
APP_TESTS = ("SQLiteReaderClosedWALTests",)
TESTS = ("TurnFailureSoundTests", "SoundVolumeCurveTests", "SessionDiscoveryTests", "CodexAppServerProtocolTests", "GitBranchReaderTests", "CursorHookPayloadTests", "HookIntakeGoldenTests", "HookEdgeFactsTests", "BridgeGoldenHarness", "BridgeGoldenFixtures",
         "BridgeGoldenExpectations", "AntigravityHookContractTests", "GrokHookForwardingPolicyTests", "ESP32ProtocolTests", "AppleCompanionPayloadTests", "ClaudeQuotaTests", "ClaudeQuotaSchedulerTests", "FollowUpReminderSchedulerTests",
         "RemoteReconnectPolicyTests", "ClaudeUsageScannerTests", "ModelLabelTests",
         "CoworkSessionTests", "CoworkAuditLogTests", "AgentTaskListTests", "AttentionPolicyTests", "DerivedSessionStateTests",
         "HookEventFactsTests", "HookEventToolUseIdTests", "OutOfOrderToolStepTests",
         "PushSigningAndResponseTests", "PushChannelPayloadTests", "PushRedactionTests", "PushPolicyTests", "ReplyCompletePlaceholderTests",
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


def sound_assets(check):
    source = (ROOT / "Sources/CodeIslandCore/SoundVolumeCurve.swift").read_text(encoding="utf-8")
    program = source + """
let amplitudes = (0...100).map { SoundVolumeCurve.amplitude(forPercent: $0) }
print(String(decoding: try JSONEncoder().encode(amplitudes), as: UTF8.self))
"""
    result = subprocess.run(["swift", "-"], input=program, text=True, capture_output=True, check=True)
    asset = ROOT / "desktop/src/lib/soundVolume.json"
    values = json.loads(result.stdout)
    if check:
        if json.loads(asset.read_text(encoding="utf-8")) != values:
            raise SystemExit("Run python3 desktop/scripts/build-swift.py sound-assets after changing SoundVolumeCurve.")
    else:
        asset.write_text(json.dumps(values, separators=(",", ":")) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "test", "macos-test", "sound-assets"))
    parser.add_argument("--check", action="store_true", help="verify generated sound assets")
    parser.add_argument("--scratch", type=Path)
    parser.add_argument("--configuration", choices=("debug", "release"), default="debug")
    args = parser.parse_args()
    if args.action == "sound-assets":
        sound_assets(args.check)
        return
    if args.action == "macos-test":
        subprocess.run(["swift", "test", "--package-path", str(ROOT), "--filter", "|".join((*TESTS, *APP_TESTS, "SessionListOrderingTests", "IdleSessionRemovalTests", "CodexAppServerClientTests", "CodexAutoReviewRoutingTests", "ApprovalTrustBoundaryTests", "AppStateCodexTranscriptTests", "ClaudeSessionRecordPairingTests"))], check=True)
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
    tests = [ROOT / "Tests/CodeIslandCoreTests" / f"{name}.swift" for name in TESTS]
    tests.extend(ROOT / "Tests/CodeIslandTests" / f"{name}.swift" for name in APP_TESTS)
    stage(tests, package / "Tests/CodeIslandCoreTests")
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
