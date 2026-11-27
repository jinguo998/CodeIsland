import XCTest
@testable import CodeIsland

final class QuietHoursTests: XCTestCase {

    func testQuietHoursL10nKeysExistInAllLanguages() {
        for (lang, table) in L10n.strings {
            for key in ["quiet_hours", "quiet_hours_desc", "quiet_hours_start", "quiet_hours_end"] {
                XCTAssertNotNil(table[key], "\(lang) missing \(key)")
            }
        }
    }
}
