import XCTest
#if os(tvOS)
@testable import BluefinTV
#elseif canImport(Bluefin)
@testable import Bluefin
#else
@testable import TrailersLane
#endif

final class TrailersTests: XCTestCase {
    func testAccountIdentityChangesForCredentialRotationAndServerSwitch() {
        let first = AccountSession(server: "https://library.invalid", token: "synthetic-a", userId: "viewer-a", username: "Synthetic", deviceId: "fixture", allowHttp: false)
        var changed = first
        let key = TrailersAccount.identity(first)
        changed.token = "synthetic-b"
        XCTAssertNotEqual(key, TrailersAccount.identity(changed))
        changed = first; changed.userId = "viewer-b"
        XCTAssertNotEqual(key, TrailersAccount.identity(changed))
        changed = first; changed.server = "https://other.invalid"
        XCTAssertNotEqual(key, TrailersAccount.identity(changed))
        XCTAssertNotEqual(key, TrailersAccount.identity(nil))
    }
    func testOnlyKnownExactPublicVideoURLs() {
        for raw in ["https://www.youtube.com/watch?v=abcdefghijk", "https://youtu.be/abcdefghijk",
                    "https://vimeo.com/12345", "https://tv.apple.com/us/movie/original-study/umc.cmc.synthetic"] {
            XCTAssertNotNil(TrailersProvider.validated(raw), raw)
        }
        for raw in ["http://youtu.be/abcdefghijk", "https://youtu.be.evil.invalid/abcdefghijk",
                    "https://user:secret@youtu.be/abcdefghijk", "https://youtu.be:443/abcdefghijk",
                    "https://www.youtube.com/watch?v=abcdefghijk&token=secret",
                    "https://www.youtube.com/redirect?v=abcdefghijk", "https://youtu.be/abcdefghijk#token",
                    "https://youtu.be/%61bcdefghijk", "https://vimeo.com/123/456",
                    "https://tv.apple.com/", "file:///tmp/trailer.mp4", "https://youtu.be/abcdefghijk\n"] {
            XCTAssertNil(TrailersProvider.validated(raw), raw)
        }
    }
    func testAssociationDoesNotFallBackToOwnerOrAnotherTrailer() throws {
        let data = Data(#"{"item":{"Id":"film-a","Name":"Original study","RemoteTrailers":[{"Url":"https://youtu.be/abcdefghijk"},{"Url":"https://youtu.be/abcdefghijk"},{"Url":"https://evil.invalid/stream"}]},"localTrailers":[{"Id":"trailer-a","Name":"Synthetic trailer","Type":"Trailer"}]}"#.utf8)
        let detail = try JSONDecoder().decode(TrailersDetail.self, from: data)
        XCTAssertEqual(detail.local(id: "trailer-a", ownerID: "film-a")?.Id, "trailer-a")
        XCTAssertNil(detail.local(id: "film-a", ownerID: "film-a"))
        XCTAssertNil(detail.local(id: "trailer-a", ownerID: "film-b"))
        XCTAssertNil(detail.local(id: "trailer-b", ownerID: "film-a"))
        XCTAssertEqual(detail.providers.count, 1)
    }
    func testUUIDFormattingPreservesTitleIdentity() {
        XCTAssertTrue(MediaItem.matchesID("a0b1c2d3-e4f5-6789-abcd-1234567890ef",
                                              "A0B1C2D3E4F56789ABCD1234567890EF"))
        XCTAssertFalse(MediaItem.matchesID("a0b1c2d3-e4f5-6789-abcd-1234567890ef",
                                               "A0B1C2D3E4F56789ABCD1234567890EE"))
        XCTAssertFalse(MediaItem.matchesID("title-a", "TITLE-A"))
    }

    func testEmptyAndDuplicateLocalLists() throws {
        for list in ["[]", #"[{"Id":"t"},{"Id":"t"}]"#] {
            let data = Data((#"{"item":{"Id":"film"},"localTrailers":"# + list + "}").utf8)
            let detail = try JSONDecoder().decode(TrailersDetail.self, from: data)
            XCTAssertNil(detail.local(id: "t", ownerID: "film"))
        }
    }
}
