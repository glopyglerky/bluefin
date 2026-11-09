import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

final class ProviderHandoffTests: XCTestCase {
    func testProviderHandoffsDoNotAcceptCredentialsOrUnknownDestinations() {
        XCTAssertNotNil(ProviderDestination.validated(name: "ESPN", rawURL: "https://www.espn.com/watch/"))
        for url in ["http://www.espn.com/watch/", "https://user:password@www.espn.com/watch/",
                    "https://www.espn.com/watch/?Token=secret", "https://www.espn.com.evil.invalid/watch/",
                    "https://www.amazon.com/cart", "https://www.espn.com:9443/watch/"] {
            XCTAssertNil(ProviderDestination.validated(name: "Provider", rawURL: url))
        }
    }
    func testUnknownSportsStatusDoesNotOfferAChannelAsVerifiedGamePlayback() throws {
        let data = Data(#"{"kind":"UNKNOWN","item_id":"item","channel_id":"channel","event":{"name":"A vs B","league_name":"League","competitors":[{"id":"a","name":"A","score":"3"},{"id":"b","name":"B","score":null}]},"playback_verified":false}"#.utf8)
        let value = try JSONDecoder().decode(SportsOption.self, from: data)
        XCTAssertFalse(value.canOpenChannel)
        XCTAssertEqual(value.event?.competitors?.first?.score, "3")
        XCTAssertNil(value.event?.competitors?.last?.score)
        XCTAssertEqual(value.stateLabel, "Status unavailable")
    }
}
