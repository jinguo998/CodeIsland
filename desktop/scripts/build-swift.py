#!/usr/bin/env python3
"""Build the macOS core sources and their original tests for the native desktop host."""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True
from swift_sources import app_selections, core_selections, test_selections

ROOT = Path(__file__).resolve().parents[2]
BRIDGE = ROOT / "desktop/swift"
CORE = (
    "ClaudeConfigPaths", "ConfigPathIdentity", "ExtraConfigDirs", "ClaudeDesktopCodeSession",
    "ClaudeQuota", "ClaudeQuotaScheduler", "ClaudeUsageScanner", "ModelLabel",
    "ESP32Protocol", "AppleCompanionPayload", "AgentTaskList", "AttentionPolicy", "BackgroundCompactionFilter",
    "CoworkSession", "CoworkAuditLog", "CodexHooksConfig", "CodexThreadSubsession", "CursorHookPayload",
    "CursorSessionFolding", "DeclaredSubsession", "DisplayOnlyWait", "EventSoundRouting", "SoundVolumeCurve", "SceneMuteState",
    "HermesHookPayload", "HermesSessionStore", "HostHarness", "Models", "PushNotification", "PushPolicy", "SessionRecap",
    "ChatMessageTextFormatter", "MarkdownBlockParser", "MarkdownInlineLiterals", "MarkdownPreviewText", "TextRenderCache",
    "SubsessionLink", "SubsessionRouter", "WarpPaneResolver", "HookIntake", "HookEdgeFacts",
    "AntigravityHookContract", "GrokHookForwardingPolicy", "GrokSessionPaths", "EventNormalizer",
    "FollowUpReminderScheduler", "RemoteReconnectPolicy", "SessionSnapshot", "PushChannel", "CodexAppServerClient",
    "HookOrigin", "AgentTaskParsing", "GitBranchReader",
)
APP = (
    "CodexPermissionRules.swift", "Discovery/SQLiteReader.swift", "RemoteHost.swift",
    "IslandSurface.swift", "IdleSessionRemoval.swift", "JSONMinimalEditor.swift", "NotchAnimationMetrics.swift",
)
TESTS = (
    "ChatMessageTextFormatterTests", "MarkdownBlockParserTests", "MarkdownInlineLiteralsTests", "MarkdownPreviewTextTests",
    "JSONLTailerCursorQuestionTests", "DisplayOnlyWaitTests", "TurnFailureSoundTests", "SoundVolumeCurveTests", "GitBranchReaderTests",
    "CursorHookPayloadTests", "HookIntakeGoldenTests", "HookEdgeFactsTests", "HookOriginTests", "BridgeGoldenFixtures", "BridgeGoldenExpectations",
    "AntigravityHookContractTests", "GrokHookForwardingPolicyTests", "ESP32ProtocolTests", "AppleCompanionPayloadTests",
    "ClaudeQuotaSchedulerTests", "FollowUpReminderSchedulerTests", "RemoteReconnectPolicyTests", "ClaudeUsageScannerTests",
    "ModelLabelTests", "CoworkSessionTests", "CoworkAuditLogTests", "AgentTaskListTests", "AttentionPolicyTests", "DerivedSessionStateTests",
    "HookEventFactsTests", "HookEventToolUseIdTests", "OutOfOrderToolStepTests", "PushChannelPayloadTests", "PushRedactionTests",
    "PushPolicyTests", "ReplyCompletePlaceholderTests", "SessionRecapTests", "SubsessionReducerTests", "SubsessionRouterTests",
    "SubsessionTestSupport", "PushSigningAndResponseTests", "CodexAppServerClientTests",
)
MAC_TESTS = ("ClaudeQuotaTests", "JSONLTailerTests",
             "SessionListOrderingTests", "IdleSessionRemovalTests", "CodexAutoReviewRoutingTests", "ApprovalTrustBoundaryTests",
             "AppStateToolUseCacheTests", "AppStateCodexTranscriptTests", "ClaudeSessionRecordPairingTests", "SQLiteReaderClosedWALTests")


def link_directory(link, original):
    """Give SwiftPM a path inside the package that resolves to the checkout itself."""
    if link.exists():
        if not link.samefile(original):
            raise ValueError(f"{link} already points somewhere other than {original}")
        return
    link.parent.mkdir(parents=True, exist_ok=True)
    try:
        if os.name == "nt":
            # NTFS junctions work across local volumes without Developer Mode or elevation.
            subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                            "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction "
                            "-Path $env:CODEISLAND_LINK -Target $env:CODEISLAND_ORIGINAL | Out-Null"],
                           env={**os.environ, "CODEISLAND_LINK": str(link), "CODEISLAND_ORIGINAL": str(original)},
                           check=True)
        else:
            link.symlink_to(os.path.relpath(original, link.parent), target_is_directory=True)
    except (FileExistsError, subprocess.CalledProcessError):
        if not link.is_dir() or not link.samefile(original):
            raise
    if not link.samefile(original):
        raise ValueError(f"{link} does not reference {original}")


def write_changed(path, content):
    if path.is_file() and path.read_bytes() == content:
        return
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as handle:
        temporary = Path(handle.name)
        handle.write(content)
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def source_list(root, files):
    """Exclude unused neighbours of the referenced files without traversing their trees."""
    files = sorted(Path(name) for name in files)
    directories = {parent for path in files for parent in path.parents}
    used = directories.union(files)
    excluded = sorted(
        entry.relative_to(root).as_posix() for parent in directories for entry in (root / parent).iterdir()
        if entry.relative_to(root) not in used
    )
    return {"sources": [path.as_posix() for path in files], "exclude": excluded}


def target_inputs(package, target, original, files, adapters, selections):
    root = package / "Targets" / target
    link_directory(root / "Original", original)
    link_directory(root / "Desktop", adapters)
    inputs = ["Original/" + name for name in files]
    inputs += ["Desktop/" + path.name for path in sorted(adapters.glob("*.swift"))]
    # These are the remaining mixed macOS files whose definitions still get extracted.
    # Keep them explicit in the compiler input list, separate from original source references.
    generated = root / "Selected"
    generated.mkdir(exist_ok=True)
    for name, content in selections:
        filename = "Selected" + name + ".swift"
        write_changed(generated / filename, content.encode("utf-8"))
        inputs.append("Selected/" + filename)
    return source_list(root, inputs)


def sound_assets(check):
    with tempfile.TemporaryDirectory() as folder:
        scratch = Path(folder)
        main = scratch / "main.swift"
        main.write_text("""import Foundation
let amplitudes = (0...100).map { SoundVolumeCurve.amplitude(forPercent: $0) }
print(String(decoding: try JSONEncoder().encode(amplitudes), as: UTF8.self))
""", encoding="utf-8")
        binary = scratch / ("sound-assets.exe" if os.name == "nt" else "sound-assets")
        subprocess.run(["swiftc", str(ROOT / "Sources/CodeIslandCore/SoundVolumeCurve.swift"),
                        str(main), "-o", str(binary)], check=True)
        result = subprocess.run([str(binary)], text=True, capture_output=True, check=True)
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
    package.mkdir(parents=True, exist_ok=True)
    write_changed(package / "Package.resolved", (BRIDGE / "Package.resolved").read_bytes())
    link_directory(package / "Targets/SQLite3", BRIDGE / "SQLite3")
    link_directory(package / "Targets/CryptoKit", BRIDGE / "CryptoKit")
    link_directory(package / "Targets/CodeIslandAppTests", ROOT / "Tests/CodeIslandTests")
    sources = {
        "CodeIslandCore": target_inputs(package, "CodeIslandCore", ROOT / "Sources/CodeIslandCore",
            (name + ".swift" for name in CORE), BRIDGE / "Platform", core_selections(ROOT)),
        "CodeIslandDesktopCore": target_inputs(package, "CodeIslandDesktopCore", ROOT / "Sources/CodeIsland",
            APP, BRIDGE / "Sources", app_selections(ROOT)),
        "CodeIslandCoreTests": target_inputs(package, "CodeIslandCoreTests", ROOT / "Tests/CodeIslandCoreTests",
            (name + ".swift" for name in TESTS), BRIDGE / "Tests", test_selections(ROOT)),
        "CodeIslandAppTests": source_list(package / "Targets/CodeIslandAppTests",
            ["HookServerCwdFilterTests.swift", "RemoteCwdFilterTests.swift"]),
    }
    paths = json.dumps(sources, indent=2).encode()
    write_changed(package / "inputs.json", paths)
    # SwiftPM's manifest cache keys on Package.swift, including when only the list changes.
    stamp = hashlib.sha256(paths).hexdigest()
    write_changed(package / "Package.swift", (BRIDGE / "Package.swift").read_bytes() +
                  f"\n// Target inputs: {stamp}\n".encode())
    base = ["swift", args.action, "--package-path", str(package), "-c", args.configuration]
    if jobs := os.environ.get("NUM_JOBS"):
        base += ["--jobs", jobs]
    if args.action == "test":
        if os.name == "nt":
            # The upstream lifecycle fixture launches /bin/sh. The raw Cursor
            # fixture assumes a POSIX root; DesktopCursorPathTests exercises the
            # Windows production entry point for drive and UNC paths instead.
            # One worker avoids putting all remaining names on one command line.
            base += ["--parallel", "--num-workers", "1",
                     "--skip", "CodexAppServerClientTests/testProcessExitDisarmsReadabilityHandlers$",
                     "--skip", "DerivedSessionStateTests/testExtractMetadataStillDecodesCursorTranscriptPath$"]
        subprocess.run(base, check=True)
    else:
        subprocess.run(base + ["--product", "CodeIslandDesktopCore"], check=True, stdout=sys.stderr)
        subprocess.run(base + ["--show-bin-path"], check=True)


if __name__ == "__main__":
    main()
