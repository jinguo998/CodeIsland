import XCTest
@testable import CodeIslandCore

final class CodexAppServerClientTests: XCTestCase {
    // MARK: - Process lifecycle (sandboxed subprocess)

    /// When the spawned server exits, its stdout/stderr pipes hit EOF. A
    /// readabilityHandler left installed on an EOF'd pipe is re-invoked in a
    /// tight loop by the underlying dispatch source, pinning a CPU core
    /// forever (#278 class of bug). Spawn a process that exits immediately and
    /// assert both handlers are disarmed once EOF is observed.
    func testProcessExitDisarmsReadabilityHandlers() throws {
        let client = CodexAppServerClient(
            executableURL: URL(fileURLWithPath: "/bin/sh"),
            arguments: ["-c", "exit 0"]
        )
        let exited = expectation(description: "onExit fired")
        client.onExit = { _ in exited.fulfill() }

        try client.start()
        let stdoutHandle = try XCTUnwrap(client.currentStdoutPipe).fileHandleForReading
        let stderrHandle = try XCTUnwrap(client.currentStderrPipe).fileHandleForReading
        wait(for: [exited], timeout: 5)

        // The EOF callbacks race the termination notification — poll briefly.
        let deadline = Date().addingTimeInterval(2)
        while Date() < deadline,
              stdoutHandle.readabilityHandler != nil || stderrHandle.readabilityHandler != nil {
            Thread.sleep(forTimeInterval: 0.05)
        }

        XCTAssertNil(
            stdoutHandle.readabilityHandler,
            "stdout readabilityHandler still installed after EOF — its dispatch source respins forever at 100% CPU"
        )
        XCTAssertNil(
            stderrHandle.readabilityHandler,
            "stderr readabilityHandler still installed after EOF — its dispatch source respins forever at 100% CPU"
        )
        client.stop()
    }
}
