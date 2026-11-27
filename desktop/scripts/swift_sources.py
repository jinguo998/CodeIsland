"""Select unmodified source from the macOS checkout for a standalone SwiftPM target.

Whole files are copied verbatim by build-swift.py. For files that also contain a
Cocoa view, Unix socket or Darwin file watcher, only the named source ranges are
compiled. Boundaries must be unique. There is no substitution in source bodies.
"""
from pathlib import Path


class Source:
    def __init__(self, root, path):
        self.path = path
        self.lines = (root / path).read_text(encoding="utf-8").splitlines(keepends=True)

    def index(self, prefix):
        matches = [i for i, line in enumerate(self.lines) if line.startswith(prefix)]
        if len(matches) != 1:
            raise ValueError(f"{self.path}: expected one source boundary {prefix!r}, found {len(matches)}")
        return matches[0]

    def section(self, start=None, until=None):
        first = self.index(start) if start else 0
        end = self.index(until) if until else len(self.lines)
        if end <= first:
            raise ValueError(f"{self.path}: empty source selection {start!r} .. {until!r}")
        return self.span(first, end)

    def span(self, first, end):
        return (f'#sourceLocation(file: "{self.path}", line: {first + 1})\n'
                + "".join(self.lines[first:end]) + "\n#sourceLocation()\n")

    def decl(self, signature):
        first = self.index(signature)
        indent = signature[:len(signature) - len(signature.lstrip())]
        end = next(i for i in range(first + 1, len(self.lines)) if self.lines[i].rstrip() == indent + "}")
        return self.span(first, end + 1)

    def line(self, signature):
        i = self.index(signature)
        return self.span(i, i + 1)


def namespace(name, *sections):
    return name + " {\n" + "".join(sections) + "}\n"


NETWORK = "#if canImport(FoundationNetworking)\nimport FoundationNetworking\n#endif\n"
FOUNDATION = "import Foundation\n"
APP = "import CodeIslandCore\n" + FOUNDATION


def core_selections(root):
    def source(name):
        return Source(root, f"Sources/CodeIslandCore/{name}.swift")
    task = source("AgentTaskParsing")
    # The only imported Darwin function is memmem; the desktop CRT adapter supplies it.
    yield "AgentTaskParsing", FOUNDATION + task.section("/// A checklist tool")
    tail = source("JSONLTailer")
    yield "JSONLTailer", (FOUNDATION + tail.section("public enum ConversationTurnStatus", "public final class JSONLTailer")
                         + "public final class JSONLTailer {\n" + tail.section("    public struct ScanResult"))
    ai = source("AiWorkWatchClient")
    yield "AiWorkStatusMapper", FOUNDATION + ai.section("public enum AiWorkStatusMapper", "    public static func fetchBusyDaemonSessionIds") + "}\n"
    codex = source("CodexAppServerClient")
    yield "CodexAppServerClient", (codex.section(until="public final class CodexAppServerClient") +
        namespace("public enum CodexAppServerClient", codex.decl("    public static func drainMessages"),
                  codex.decl("    public static func parseMessage")))
    quota = source("ClaudeQuotaClient")
    yield "ClaudeQuotaClient", (NETWORK + quota.section(until="    /// Raw item data from the login keychain")
        + quota.decl("    public static func readFile") + "}\n" + quota.section("public enum ClaudeQuotaClientError"))
    push = source("PushTransport")
    yield "PushTransport", (NETWORK + push.section(until="public final class URLSessionPushTransport") +
        namespace("public enum URLSessionPushTransport", push.decl("    static func retryAfter")))
    channel = source("PushChannel")
    yield "PushChannel", NETWORK + channel.section()


def app_selections(root):
    def source(name):
        return Source(root, f"Sources/CodeIsland/{name}.swift")
    ordering = source("SessionListOrdering")
    yield "CardOrdering", (APP + ordering.section("enum SessionCardStatus", "    var label: String") +
        ordering.section("    var tier: Int", "enum SessionListOrdering") +
        namespace("enum SessionListOrdering", ordering.decl("    static func order("),
                  ordering.decl("    static func requestRank(")))
    idle = source("IdleSessionRemoval")
    yield "IdleSessionRemoval", idle.section(until="enum RemovedSessionRevivalEvidence")
    sound = source("SoundManager")
    yield "SoundTiming", APP + namespace("struct DesktopSoundTiming",
        sound.line("    private var turnFailureDebouncer"), sound.decl("    nonisolated static func isInQuietHours"),
        "    mutating func shouldPlay(_ key: String, now: Date) -> Bool { turnFailureDebouncer.shouldPlay(key: key, now: now) }\n")
    strings = source("L10n")
    notifier = source("PushNotifier")
    yield "PushStrings", (APP + "struct L10n {\n    static let shared = Self(language: \"system\")\n    let language: String\n" +
        strings.section("    var effectiveLanguage: String") + namespace("enum DesktopPushStrings", notifier.decl("    static func strings(")))
    # The adapter supplies the WinRT subscriber's maximum frame size. The macOS
    # packet header, serialization and summary payload are compiled from the original.
    ble = source("AppleCompanionBluetoothPeripheral")
    yield "CompanionFrames", (APP + "enum DesktopCompanionFrames {\n" +
        namespace("    private enum Limits", ble.line("    private static let maxChunkPayloadBytes"), "    static var maximum: Int { maxChunkPayloadBytes }\n") +
        "    @TaskLocal private static var maxChunkPayloadBytes: Int = 0\n" + ble.decl("    private static func makeChunks") +
        (root / "desktop/swift/Platform/CompanionMTU.inc").read_text() + "}\n" +
        ble.section("private struct AppleCompanionBluetoothSummary"))
    claude = source("Discovery/ClaudeDiscovery")
    yield "ClaudeRecords", (APP + namespace("enum ClaudeDiscovery", claude.line("    static let sessionRecordByteLimit"),
        claude.decl("    static func readSessionRecord("), claude.decl("    static func recordedSessionId(inRecord"),
        claude.decl("    static func parseRecordedProcessStart")))
    codex = source("Discovery/CodexDiscovery")
    discovery = source("Discovery/SessionDiscovery")
    yield "CodexRollout", (APP + codex.decl("enum CodexTranscriptOrigin") +
        namespace("enum CodexDiscovery", "    typealias FileHandle = CompleteRolloutHandle\n", codex.decl("    static func codexTranscriptOrigin(path:"),
            codex.decl("    static func inspectCodexSubagentMetadata("), codex.decl("    static func latestCodexTurnStatus("),
            codex.decl("    private static func codexTurnStatus(fromCompleteLine")) +
        namespace("enum SessionDiscovery", discovery.decl("    static func firstStringRecursively"),
            "    @TaskLocal static var suppliedHeader: String?\n",
            "    static func readFirstLine(path: String) -> String? { suppliedHeader ?? FileHeader.readFirstLine(path: path) }\n") +
        namespace("private enum FileHeader", discovery.decl("    static func readFirstLine(")))
    rpc = source("AppState+CodexAppServer")
    model = source("Models")
    yield "CodexMessages", (APP + model.decl("struct AskUserQuestionItem") +
        namespace("enum DesktopCodexMessage", rpc.decl("    static func applyCodexThreadStatus("),
            rpc.decl("    static func codexRequestUserInputResult("),
            "    static func questions(_ rawQuestions: [AnyCodableLike]) -> [AskUserQuestionItem] {\n" +
            rpc.section("        var askItems:", "        guard !askItems.isEmpty") + "        return askItems\n    }\n"))
    yield "ApprovalAlwaysScope", APP + source("NotchPanelView").decl("enum ApprovalAlwaysScope")
    calls = source("AppState+ToolUseCache")
    yield "PermissionCalls", APP + "struct PermissionRequest { let event: HookEvent }\n" + namespace("struct DesktopPermissionCalls",
        calls.decl("    nonisolated static func step("), calls.decl("    func shouldKeepQueuedPermissionForCompletedEvent("))


def test_selections(root):
    def source(name):
        return Source(root, f"Tests/CodeIslandCoreTests/{name}.swift")
    q = source("ClaudeQuotaTests")
    yield "ClaudeQuotaTests", NETWORK + q.section(until="    func testRunnerReturnsStdout") + "}\n"
    t = source("JSONLTailerTests")
    yield "JSONLTailerTests", (t.section(until="    func testAttachAndDetectAppendedLine") +
                               t.section("    func testScanTailForAttachSkips"))
    c = source("CodexAppServerClientTests")
    yield "CodexAppServerClientTests", c.section(until="    func testProcessExitDisarmsReadabilityHandlers") + "}\n"
    signing = source("PushSigningAndResponseTests")
    yield "PushSigningAndResponseTests", NETWORK + signing.section()
    golden = source("BridgeGoldenHarness")
    yield "BridgeGoldenHarness", (FOUNDATION + "import XCTest\n#if canImport(CoreFoundation)\nimport CoreFoundation\n#endif\n" +
        golden.section("struct BridgeGoldenFixture", "final class BridgeGoldenRunner"))
