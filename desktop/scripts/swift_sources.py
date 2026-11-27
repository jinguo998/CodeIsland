"""Legacy source extraction for declarations in mixed macOS files.

Each selection generates a second Swift source file in the build directory.
Whole-file references are listed separately in build-swift.py.
"""
from pathlib import Path


class Source:
    def __init__(self, root, path):
        self.path = path
        self.lines = (root / path).read_bytes().decode("utf-8").splitlines(keepends=True)

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

    def decl_bounds(self, signature):
        first = self.index(signature)
        indent = signature[:len(signature) - len(signature.lstrip())]
        for i in range(first + 1, len(self.lines)):
            if self.lines[i].rstrip() == indent + "}":
                return first, i + 1
        raise ValueError(f"{self.path}: missing closing brace for {signature!r}")

    def decl(self, signature):
        return self.span(*self.decl_bounds(signature))

    def part(self, declaration, start, until, first_match=False):
        """Select unique boundaries inside a declaration with repeated local names."""
        first, last = self.decl_bounds(declaration)
        def boundary(prefix, take_first=False):
            found = [i for i in range(first, last) if self.lines[i].startswith(prefix)]
            if not found or (not take_first and len(found) != 1):
                raise ValueError(f"{self.path}: {declaration}: ambiguous or missing {prefix!r}")
            return found[0]
        return self.span(boundary(start, first_match), boundary(until))

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
    tail = source("JSONLTailer")
    scanner = (FOUNDATION + tail.section("public enum ConversationTurnStatus", "public final class JSONLTailer")
               + "public final class JSONLTailer {\n" + tail.section("    public struct ScanResult"))
    # Rust also sends decoded records, whose keys retain no provider wire order.
    # Run the original post-decode body with the scanner's private helpers.
    yield "JSONLTailer", scanner + namespace("extension JSONLTailer",
        "    public static func desktopRecord(_ json: [String: Any]) -> ScanResult.Delta {\n" +
        "        var delta = ScanResult.Delta()\n        desktopApply(json, into: &delta)\n        return delta\n    }\n",
        "    private static func desktopApply(_ json: [String: Any], into delta: inout ScanResult.Delta) {\n" +
        tail.section('        if json["isMeta"]', "    /// How Claude Code records"))
    ai = source("AiWorkWatchClient")
    yield "AiWorkStatusMapper", FOUNDATION + ai.section("public enum AiWorkStatusMapper", "    public static func fetchBusyDaemonSessionIds") + "}\n"
    quota = source("ClaudeQuotaClient")
    yield "ClaudeQuotaClient", (NETWORK + quota.section(until="    /// Raw item data from the login keychain")
        + quota.decl("    public static func readFile") + "}\n" + quota.section("public enum ClaudeQuotaClientError"))
    push = source("PushTransport")
    yield "PushTransport", (NETWORK + push.section(until="public final class URLSessionPushTransport") +
        namespace("public enum URLSessionPushTransport", push.decl("    static func retryAfter")))


def app_selections(root):
    def source(name):
        return Source(root, f"Sources/CodeIsland/{name}.swift")
    server = source("HookServer")
    settings = source("Settings")
    tray = source("StatusItemController")
    yield "TrayDrawing", FOUNDATION + namespace("extension DesktopTrayDrawing",
        "    func draw() {\n        let ctx = self\n" + tray.line("        let size = NSSize(") +
        "        width = size.width\n        height = size.height\n" + tray.line("            let pill = NSRect(") +
        tray.part("    static func menuBarIcon(", "            NSBezierPath(roundedRect:", "            return true") + "    }\n")
    yield "HookServer", APP + namespace("enum HookServer",
        server.decl("    enum RouteKind:"), server.decl("    enum Intake {"),
        *(server.decl("    nonisolated static func " + method) for method in (
            "intake(", "routeKind(", "shouldDeferPermissionRequestToProvider(",
            "cwdMatchesAnyPattern(", "remoteEventPassesCwdFilter(", "remoteEventBypassesCwdFilter(",
            "isAutoApprovedSource(")),
        "    static func approvedSources(_ raw: String) -> Set<String> {\n" +
            settings.part("    var autoApproveSources:", "            let entries = raw.split", "        set {"))
    state = source("AppState")
    shortcuts = source("AppState+CardShortcuts")
    yield "CardShortcut", (APP
        + source("Settings").section("enum ShortcutAction:", "    var defaultBinding: ShortcutBinding?") + "}\n"
        + namespace("extension DesktopCardShortcut", shortcuts.decl("    enum CardShortcutOutcome:"),
            shortcuts.decl("    func performCardShortcut(")))
    calls = source("AppState+ToolUseCache")
    yield "HookAnswers", APP + namespace("struct DesktopHookAnswers",
        "    static func askItems(_ event: HookEvent) -> [AskUserQuestionItem] {\n" +
            state.section("        let originalQuestions = event.toolInput?", "        guard !askItems.isEmpty else {") +
            "        return askItems\n    }\n",
        state.decl("    nonisolated static func agentKeepsAlwaysRule("),
        state.decl("    nonisolated static func claudeAlwaysAllowResponse("),
        state.decl("    nonisolated static func isQoderEvent("),
        state.decl("    private func askUserQuestionUpdatedInput("),
        calls.section("    nonisolated static let plainAllowAnswer", "    /// Cache a PreToolUse"),
        "    static func deny() -> Data {\n" +
            state.part("    func denyPermission(", "        let response = #", "        pending.continuation.") +
            "        return Data(response.utf8)\n    }\n",
        "    func question(_ pending: DesktopQuestion, _ answer: String, answers: [String: String]) -> Data {\n" +
            "        let responseData: Data\n        if pending.isFromPermission {\n" +
            "            let updatedInput = askUserQuestionUpdatedInput(event: pending.event, answers: answers, answer: answer, originalQuestions: nil)\n" +
            state.part("    func answerQuestion(_ answer:", "            let obj: [String: Any]", "        pending.resolution.", first_match=True) +
            "        return responseData\n    }\n",
        "    static func skip(isPermission: Bool) -> Data {\n        let responseData: Data\n        if isPermission {\n" +
            state.part("    func skipQuestion(", "                responseData = Data(#", "            pending.resolution.", first_match=True) +
            "        return responseData\n    }\n")
    yield "Webhook", APP + namespace("enum DesktopWebhook",
        "    static func encode(_ event: HookEvent, defaults: DesktopWebhookFilter, emit: (Data) -> Void) {\n" +
        server.section("        let normalizedName = EventNormalizer.normalize(event.eventName)", "        var request = URLRequest") +
        "        emit(body)\n    }\n")
    remote = source("RemoteInstaller")
    remote_helpers = (
        "_codex_home", "_claude_config_dir", "_display_path",
        "strip_json_comments", "ensure_jsonc_object", "write_opencode_config",
        "append_our_hooks", "_normalize_traecli_hooks_list_indentation", "_detect_traecli_hook_item_indent",
        "_render_managed_traecli_hooks", "_parse_yaml_scalar", "_codex_toml_string",
        "_codex_toml_statements", "_codex_toml_key", "_codex_toml_tail",
        "_codex_is_features_header", "_codex_without_appended_features", "_codex_hooks_toml",
    )
    literals = []
    for name in remote_helpers:
        first = remote.index(f"def {name}(")
        last = first + 1
        while last < len(remote.lines) and (not remote.lines[last].strip() or remote.lines[last][0].isspace()):
            last += 1
        body = "".join(remote.lines[first:last]).rstrip()
        # The Swift compiler interprets the same literal escapes as the original.
        literals.append('"""\n' + body + '\n"""')
    yield "RemoteInstaller", FOUNDATION + namespace("enum DesktopRemoteInstaller",
        "    static let helpers = [" + ",\n".join(literals) + '].joined(separator: "\\n\\n")\n',
        remote.decl("    static func shellWord("), remote.decl("    static func quotedLiteral("),
        remote.line("    static let remoteSocketBase"), remote.decl("    static func loginUID("),
        remote.decl("    private static func isUID("), remote.decl("    static func socketOwnerUID("),
        remote.decl("    static func remoteOpencodePluginForInstall("), remote.decl("    private static func jsonStringLiteral("))
    installer = source("ConfigInstaller")
    yield "InstallerCatalog", (APP + installer.decl("enum HookFormat {") + installer.decl("struct CLIConfig {") +
        namespace("extension ConfigInstaller",
            installer.section("    static func displayCodexPath", "    /// How many agents"),
            installer.decl("    static func defaultEvents("), installer.decl("    private static func traecliNextEvents("),
            installer.line("    static let mimoPluginSourceMarker"), installer.line("    static let mimoPluginSourceLine"),
            installer.decl("    static func mimoPluginSource("),
            installer.line("    private static let minimaxManifestMarker"), installer.decl("    static func minimaxManifestDocument("),
            installer.decl("    static func stripJSONComments("),
            "    static func opencodeTarget(_ opencodeConfigPathNew: String, _ opencodeConfigPathJsonc: String, fm: FileManager = .default) -> String {\n" +
            installer.part("    private static func installOpencodePlugin(", "        let targetPath: String", "        let originalContents:") +
            "        return targetPath\n    }\n",
            installer.decl("    static func mergeOpencodePluginRef("), installer.decl("    static func removeOpencodePluginRef("),
            "    static func traeCommand(event: String, timeout: Int, command: String) -> String {\n" +
            """        let escapedCommand = command.replacingOccurrences(of: "'", with: "''")
        let events = [(event, timeout, false)]
""" + installer.part("    private static func renderManagedTraecliHooksText(", "        let timeout =", "        return lines.joined") +
            '        return lines.joined(separator: "\\n")\n    }\n',
            "    static func kimiBlock(event: String, timeout: Int, baseCommand: String) -> String {\n" +
            installer.part("    internal static func installKimiHooks(", "            var block =", "            hookBlocks.append(block)") +
            "        return block\n    }\n",
            "    static func zcodeEntry(command: String, timeout: Int) -> [String: Any] {\n" +
            installer.part("    static func mergeZcodeHooks(", "            var hook: [String: Any]", "            entries.append(") +
            "        return [\"hooks\": [hook]]\n    }\n",
            "    static var desktopCLIs: [CLIConfig] { builtInCLIs }\n",
            "    static func writeEntry(cli: CLIConfig, event: String, timeout: Int, baseCommand: String, emit: ([String: Any]) -> Void) -> Bool {\n" +
            installer.part("    static func installExternalHooks(", "            let entry: [String: Any]", "            eventEntries.append(entry)") +
            "        emit(entry)\n        return true\n    }\n"))
    ordering = source("SessionListOrdering")
    yield "CardOrdering", (APP + ordering.section("enum SessionCardStatus", "    var label: String") +
        ordering.section("    var tier: Int", "enum SessionListOrdering") +
        namespace("enum SessionListOrdering", ordering.decl("    static func order("),
                  ordering.decl("    static func requestRank(")))
    yield "DesktopAppRouting", APP + namespace("enum AppState",
        source("AppState+DisplayOnlyWaits").decl("    nonisolated static func displayOnlyWaitAsk(forHookEvent"),
        source("AppState+CodexAppServer").line("    nonisolated static let codexAppBundleId"),
        source("TerminalActivator").decl("    static func codexDesktopThreadLink("))
    redaction = source("DiagnosticsRedaction")
    yield "DiagnosticText", (redaction.section(until="    static func redactedConfig(")
        + redaction.section("    /// `value` with its secrets replaced"))
    esp = source("ESP32StatePublisher")
    yield "CompanionState", APP + namespace("extension DesktopCompanion",
        esp.decl("    private struct BuddyDisplayContext {"), esp.decl("    func esp32DisplaySessionId("),
        esp.decl("    func esp32DisplaySession("), esp.decl("    private func esp32DisplayContext("),
        esp.decl("    func appleCompanionStatePayload("), esp.decl("    private func appleCompanionSessionPreviews("),
        esp.decl("    private func appleCompanionSessionMessage("), esp.decl("    private func appleCompanionSessionPriority("),
        esp.decl("    private func appleCompanionQuestionPayload("), esp.decl("    static func companionMessages("),
        esp.decl("    static func companionReplyText("), esp.decl("    private static func appleCompanionPreviewText("))
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
    golden = source("BridgeGoldenHarness")
    yield "BridgeGoldenHarness", (FOUNDATION + "import XCTest\n#if canImport(CoreFoundation)\nimport CoreFoundation\n#endif\n" +
        golden.section("struct BridgeGoldenFixture", "final class BridgeGoldenRunner"))
