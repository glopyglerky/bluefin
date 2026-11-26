import XCTest
#if !SPORTS_STANDALONE
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif
#endif

final class SportsPresentationTests: XCTestCase {
    func testGuideAndUnknownNeverImplyLiveGameOrScores() {
        for kind in ["ON_NOW", "UNKNOWN"] {
            let item = SportsFixtures.game(kind: kind, state: "IN")
            XCTAssertEqual(item.status, .unknown)
            XCTAssertFalse(item.showsScores)
        }
        XCTAssertFalse(SportsFixtures.game(kind: "UNKNOWN").canOpenChannel)
    }
    func testFinalUpcomingAndDelayedStates() {
        XCTAssertEqual(SportsFixtures.game(state: "POST").status, .final)
        XCTAssertTrue(SportsFixtures.game(state: "POST").showsScores)
        XCTAssertEqual(SportsFixtures.game(kind: "UPCOMING", state: "PRE").status, .upcoming)
        XCTAssertFalse(SportsFixtures.game(kind: "UPCOMING", state: "PRE").showsScores)
        XCTAssertEqual(SportsFixtures.game(state: "POSTPONED").status, .postponed)
        XCTAssertEqual(SportsFixtures.game(state: "CANCELED").status, .cancelled)
    }
    func testWatchChannelRequiresAChannelAndCurrentGuideWindow() throws {
        var item = SportsFixtures.game(kind: "ON_NOW")
        item.native_start = "2026-11-08T00:00:00Z"
        item.native_end = "2026-11-08T01:00:00Z"
        let start = try XCTUnwrap(SportsSchedule.date(item.native_start))
        let end = try XCTUnwrap(SportsSchedule.date(item.native_end))
        XCTAssertFalse(item.isAiring(on: start.addingTimeInterval(-1)))
        XCTAssertTrue(item.isAiring(on: start))
        XCTAssertFalse(item.isAiring(on: end))
        item.channel_id = nil
        XCTAssertFalse(item.isAiring(on: start))
    }
    func testLocalDayIncludesGameAcrossUTCMidnight() throws {
        let zone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        let game = try XCTUnwrap(SportsFixtures.game().scheduleDate)
        let selected = try XCTUnwrap(SportsSchedule.date("2026-11-07T12:00:00-08:00"))
        XCTAssertTrue(SportsSchedule.sameDay(game, as: selected, timeZone: zone))
        XCTAssertFalse(SportsSchedule.sameDay(game, as: selected, timeZone: TimeZone(secondsFromGMT: 0)!))
    }
    func testDSTDayUsesCalendarRatherThan24Hours() throws {
        let zone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        let start = try XCTUnwrap(SportsSchedule.date("2026-11-01T00:00:00-07:00"))
        let end = try XCTUnwrap(SportsSchedule.calendar(timeZone: zone).date(byAdding: .day, value: 1, to: start))
        XCTAssertEqual(end.timeIntervalSince(start), 25 * 3600)
        XCTAssertNotNil(SportsSchedule.date("2026-11-08T00:30:00.123Z"))
        XCTAssertNil(SportsSchedule.date("unknown"))
    }
    func testFavoritesUseStableTeamIDs() {
        XCTAssertTrue(SportsFixtures.game().matchesFavorites(SportsFixtures.preferences))
        var preferences = SportsFixtures.preferences; preferences.teams = ["Harbor Herons"]
        XCTAssertFalse(SportsFixtures.game().matchesFavorites(preferences))
    }
}
