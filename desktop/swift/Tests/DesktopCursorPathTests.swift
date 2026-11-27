import Foundation
import XCTest
@testable import CodeIslandDesktopCore

final class DesktopCursorPathTests: XCTestCase {
    func testSessionReductionKeepsDriveLettersAndUNCVolumes() throws {
        for volume in [#"C:\Users\cursor"#, #"\\server\profiles\cursor"#] {
            let project = volume + #"\.cursor\projects\enc-proj"#
            let input: [String: Any] = [
                "card": "cursor", "fresh": true, "limit": 10,
                "previous": [
                    "status": "idle", "executing": false, "interrupted": false, "failed": false, "tasks": [],
                ] as [String: Any],
                "payload": [
                    "session_id": "cursor", "hook_event_name": "PreToolUse", "_source": "cursor-cli",
                    "cwd": FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".claude").path,
                    "transcript_path": project + #"\agent-transcripts\t.jsonl"#,
                ],
            ]
            var sessions = DesktopSessions()
            let response = try sessions.apply(JSONSerialization.data(withJSONObject: input))
            let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: response) as? [String: Any])
            XCTAssertEqual(fields["cwd"] as? String, project.replacingOccurrences(of: #"\"#, with: "/"))
        }
    }
}
