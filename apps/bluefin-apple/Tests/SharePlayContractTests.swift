import XCTest
import GroupActivities
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

#if BLUEFIN_SHAREPLAY
final class SharePlayContractTests: XCTestCase {
    func testMusicAndVideoUseTheirNativeGroupActivityTypes() throws {
        let audio = BluefinGroupActivity(serverIdentity: "hashed-origin", itemID: "track", mediaKind: "Audio")
        let video = BluefinGroupActivity(serverIdentity: "hashed-origin", itemID: "movie", mediaKind: "Movie")
        XCTAssertEqual(audio.metadata.type, .listenTogether)
        XCTAssertEqual(video.metadata.type, .watchTogether)
        let payload = try JSONSerialization.jsonObject(with: JSONEncoder().encode(audio)) as? [String: Any]
        XCTAssertEqual(Set(try XCTUnwrap(payload).keys), ["serverIdentity", "itemID", "mediaKind"])
    }
    func testExistingActivitiesRemainDecodable() throws {
        let old = Data(#"{"serverIdentity":"hashed-origin","itemID":"movie"}"#.utf8)
        let value = try JSONDecoder().decode(BluefinGroupActivity.self, from: old)
        XCTAssertEqual(value.metadata.type, .watchTogether)
    }
}
#endif
