import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

final class MediaScheduleTests: XCTestCase {
    func testSearchProgramDecodesActualBroadcastWindow() throws {
        let value = Data(#"{"Id":"programme","Type":"Program","StartDate":"2026-11-27T01:00:00Z","EndDate":"2026-11-27T02:00:00.000Z"}"#.utf8)
        let item = try JSONDecoder().decode(MediaItem.self, from: value)
        let start = try XCTUnwrap(item.start), end = try XCTUnwrap(item.end)
        XCTAssertEqual(end.timeIntervalSince(start), 3600, accuracy: 0.001)
    }
    func testMissingOrMalformedBroadcastDatesStayUnavailable() throws {
        for value in [#"{"Id":"programme","Type":"Program"}"#,
                      #"{"Id":"programme","Type":"Program","StartDate":"bad","EndDate":"bad"}"#] {
            let item = try JSONDecoder().decode(MediaItem.self, from: Data(value.utf8))
            XCTAssertNil(item.start); XCTAssertNil(item.end)
        }
    }
}
