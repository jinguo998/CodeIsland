#!/usr/bin/env python3
"""Build the macOS core sources and their original tests for the native desktop host."""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from swift_sources import app_selections, core_selections, test_selections

ROOT = Path(__file__).resolve().parents[2]
BRIDGE = ROOT / "desktop/swift"
CORE = (
    "ClaudeConfigPaths", "ConfigPathIdentity", "ExtraConfigDirs", "ClaudeDesktopCodeSession",
    "ClaudeQuota", "ClaudeQuotaScheduler", "ClaudeUsageScanner", "ModelLabel",
    "GitBranchReader", "ESP32Protocol", "AppleCompanionPayload", "AgentTaskList", "AttentionPolicy", "BackgroundCompactionFilter",
    "CoworkSession", "CoworkAuditLog", "CodexHooksConfig", "CodexThreadSubsession", "CursorHookPayload",
    "CursorSessionFolding", "DeclaredSubsession", "DisplayOnlyWait", "EventSoundRouting", "SoundVolumeCurve", "SceneMuteState",
    "HermesHookPayload", "HermesSessionStore", "HostHarness", "Models", "PushNotification", "PushPolicy", "SessionRecap",
    "SessionSnapshot", "SubsessionLink", "SubsessionRouter", "WarpPaneResolver", "HookIntake", "HookEdgeFacts",
    "AntigravityHookContract", "GrokHookForwardingPolicy", "GrokSessionPaths", "EventNormalizer",
    "FollowUpReminderScheduler", "RemoteReconnectPolicy",
)
TESTS = (
    "JSONLTailerCursorQuestionTests", "DisplayOnlyWaitTests", "TurnFailureSoundTests", "SoundVolumeCurveTests", "GitBranchReaderTests",
    "CursorHookPayloadTests", "HookIntakeGoldenTests", "HookEdgeFactsTests", "BridgeGoldenFixtures", "BridgeGoldenExpectations",
    "AntigravityHookContractTests", "GrokHookForwardingPolicyTests", "ESP32ProtocolTests", "AppleCompanionPayloadTests",
    "ClaudeQuotaSchedulerTests", "FollowUpReminderSchedulerTests", "RemoteReconnectPolicyTests", "ClaudeUsageScannerTests",
    "ModelLabelTests", "CoworkSessionTests", "CoworkAuditLogTests", "AgentTaskListTests", "AttentionPolicyTests", "DerivedSessionStateTests",
    "HookEventFactsTests", "HookEventToolUseIdTests", "OutOfOrderToolStepTests", "PushChannelPayloadTests", "PushRedactionTests",
    "PushPolicyTests", "ReplyCompletePlaceholderTests", "SessionRecapTests", "SubsessionReducerTests", "SubsessionRouterTests",
    "SubsessionTestSupport",
)
MAC_TESTS = ("ClaudeQuotaTests", "JSONLTailerTests", "PushSigningAndResponseTests", "CodexAppServerClientTests",
             "SessionListOrderingTests", "IdleSessionRemovalTests", "CodexAutoReviewRoutingTests", "ApprovalTrustBoundaryTests",
             "AppStateToolUseCacheTests", "AppStateCodexTranscriptTests", "ClaudeSessionRecordPairingTests", "SQLiteReaderClosedWALTests")


def stage(sources, destination, selections=()):
    destination.mkdir(parents=True, exist_ok=True)
    inputs = {source.name: source.read_bytes() for source in sources}
    for name, content in selections:
        filename = f"Selected{name}.swift"
        if filename in inputs:
            raise ValueError(f"duplicate Swift build input: {filename}")
        inputs[filename] = content.encode("utf-8")
    for name, content in inputs.items():
        output = destination / name
        if not output.exists() or output.read_bytes() != content:
            output.write_bytes(content)
    for stale in destination.glob("*.swift"):
        if stale.name not in inputs:
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
        subprocess.run(["swift", "test", "--package-path", str(ROOT), "--filter", "|".join((*TESTS, *MAC_TESTS))], check=True)
        return
    if args.scratch is None:
        parser.error("--scratch is required for a desktop build")
    package = (args.scratch / "package").resolve()
    if any(package.is_relative_to(ROOT / name) for name in ("Sources", "Tests")):
        parser.error("--scratch must be outside the original source and test directories")
    # SwiftPM requires every target to be inside its package. The staging area
    # works across volumes on Windows without symlink privileges. It never
    # writes the macOS package or its dependency lockfile.
    stage([BRIDGE / "Package.swift", BRIDGE / "Package.resolved"], package)
    core_files = [ROOT / "Sources/CodeIslandCore" / f"{name}.swift" for name in CORE]
    stage([*core_files, BRIDGE / "Platform/CorePlatform.swift"], package / "Sources/CodeIslandCore", core_selections(ROOT))
    stage((BRIDGE / "SQLite3").iterdir(), package / "Sources/SQLite3")
    stage((BRIDGE / "CryptoKit").glob("*.swift"), package / "Sources/CryptoKit")
    app_files = [ROOT / "Sources/CodeIsland" / name for name in ("CodexPermissionRules.swift", "Discovery/SQLiteReader.swift")]
    stage([*sorted((BRIDGE / "Sources").glob("*.swift")), *app_files], package / "Sources/CodeIslandDesktopCore", app_selections(ROOT))
    tests = [ROOT / "Tests/CodeIslandCoreTests" / f"{name}.swift" for name in TESTS]
    stage([*tests, *sorted((BRIDGE / "Tests").glob("*.swift"))], package / "Tests/CodeIslandCoreTests", test_selections(ROOT))
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
