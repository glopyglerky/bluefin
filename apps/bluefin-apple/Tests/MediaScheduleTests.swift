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
    func testGuideNameAndEpisodeAreDistinct() throws {
        let value = Data(#"{"Id":"event-42","Name":"Chicago P.D.","EpisodeTitle":"Closure","ChannelId":"channel-2"}"#.utf8)
        let program = try JSONDecoder().decode(LiveProgram.self, from: value)
        XCTAssertEqual(program.title, "Chicago P.D.")
        XCTAssertEqual(program.EpisodeTitle, "Closure")
        XCTAssertEqual(program.Id, "event-42")
        XCTAssertNil(program.artwork)
    }
    func testGuideRetainsOnlyCurrentAndAdjacentChannelPages() {
        XCTAssertEqual(GuidePageRetention.offsets(around: 0, pageSize: 40), Set([0, 40]))
        XCTAssertEqual(GuidePageRetention.offsets(around: 40, pageSize: 40), Set([0, 40, 80]))
        XCTAssertEqual(GuidePageRetention.offsets(around: 500, pageSize: 40), Set([440, 480, 520]))
        XCTAssertTrue(GuidePageRetention.offsets(around: -1, pageSize: 40).isEmpty)
        XCTAssertTrue(GuidePageRetention.offsets(around: 0, pageSize: 0).isEmpty)
    }
    func testGuideClipsOvernightProgramsWithoutChangingTheirIdentity() throws {
        let start = try Date.ISO8601FormatStyle().parse("2026-11-27T23:30:00Z")
        let end = try Date.ISO8601FormatStyle().parse("2026-11-28T01:30:00Z")
        let program = LiveProgram(Id: "overnight", Name: "Movie", ChannelId: "channel",
                                  StartDate: "2026-11-27T23:00:00Z", EndDate: "2026-11-28T01:00:00Z")
        let timeline = GuideTimeline(start: start, end: end)
        let segments = timeline.segments([program])
        XCTAssertEqual(segments.count, 2)
        XCTAssertEqual(segments[0].program?.Id, "overnight")
        XCTAssertEqual(segments[0].start, start)
        XCTAssertEqual(timeline.fraction(at: segments[0].end), 0.75, accuracy: 0.0001)
        XCTAssertNil(segments[1].program)
        XCTAssertEqual(segments[1].end, end)
    }

}
