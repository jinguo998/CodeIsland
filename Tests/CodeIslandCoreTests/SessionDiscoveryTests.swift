import XCTest
@testable import CodeIslandCore

final class SessionDiscoveryTests: XCTestCase {
    func testReverseLifecycleScanSkipsOversizedUnterminatedTailLine() throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("codeisland-codex-oversized-tail-\(UUID().uuidString).jsonl")
        defer { try? FileManager.default.removeItem(at: file) }
        var data = Data(
            (#"{"type":"event_msg","payload":{"type":"task_started"}}"# + "\n").utf8
        )
        data.append(Data(repeating: 0x78, count: 512 * 1024))
        try data.write(to: file)

        XCTAssertEqual(
            CodexRollout.latestTurnStatus(
                path: file.path,
                chunkSize: 16 * 1024,
                maxBytesToScan: 1024 * 1024,
                maxLineBytes: 64 * 1024
            ),
            .processing
        )
    }

    func testReverseLifecycleScanHonorsExplicitTotalByteBudget() throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("codeisland-codex-scan-budget-\(UUID().uuidString).jsonl")
        defer { try? FileManager.default.removeItem(at: file) }
        var data = Data(
            (#"{"type":"event_msg","payload":{"type":"task_started"}}"# + "\n").utf8
        )
        data.append(Data(repeating: 0x78, count: 256 * 1024))
        try data.write(to: file)

        XCTAssertNil(CodexRollout.latestTurnStatus(
            path: file.path,
            chunkSize: 16 * 1024,
            maxBytesToScan: 64 * 1024,
            maxLineBytes: 32 * 1024
        ))
    }

    func testLiveTailWaitsForANewlineBeforeApplyingTheLastEvent() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: file) }
        let running = #"{"type":"event_msg","payload":{"type":"task_started"}}"#
        let completed = #"{"type":"event_msg","payload":{"type":"task_complete"}}"#
        try Data((running + "\n" + completed).utf8).write(to: file)
        XCTAssertEqual(CodexRollout.latestTurnStatus(path: file.path), .idle)
        XCTAssertEqual(CodexRollout.latestTurnStatus(path: file.path, chunkSize: 8, requireFinalNewline: true), .processing)
    }

    func testRecordedProcessStartIsReadAsUTC() {
        XCTAssertEqual(
            ClaudeSessionRecord.parseRecordedProcessStart("Fri Oct 23 00:59:52 2026")?.timeIntervalSince1970,
            1_792_717_192
        )
        XCTAssertEqual(
            ClaudeSessionRecord.parseRecordedProcessStart("Sat Oct  3 01:02:03 2026")?.timeIntervalSince1970,
            1_790_989_323
        )
        XCTAssertNil(ClaudeSessionRecord.parseRecordedProcessStart("yesterday"))
    }
}
