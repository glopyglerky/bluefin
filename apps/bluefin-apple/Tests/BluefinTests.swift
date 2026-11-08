import XCTest
@testable import Bluefin

final class BluefinTests: XCTestCase {
    func testEpisodeMetadataAndResumeTicks() throws {
        let data = Data(#"{"Id":"episode-1","Name":"Arrival","Type":"Episode","SeriesName":"A show","ParentIndexNumber":2,"IndexNumber":4,"RunTimeTicks":18000000000,"UserData":{"PlaybackPositionTicks":4500000000}}"#.utf8)
        let item = try JSONDecoder().decode(MediaItem.self, from: data)
        XCTAssertEqual(item.subtitle, "A show · S2 E4")
        XCTAssertEqual(item.progress, 0.25, accuracy: 0.0001)
        XCTAssertFalse(item.isContainer)
    }
    func testServerStateCannotProduceInvalidProgress() throws {
        for (duration, position, expected) in [(0, 20, 0.0), (100, -10, 0.0), (100, 200, 1.0)] {
            let data = Data("{\"Id\":\"item\",\"RunTimeTicks\":\(duration),\"UserData\":{\"PlaybackPositionTicks\":\(position)}}".utf8)
            XCTAssertEqual(try JSONDecoder().decode(MediaItem.self, from: data).progress, expected)
        }
    }
    func testFolderAndSeriesNavigateInsteadOfPlaying() throws {
        for kind in ["Series", "Season", "MusicAlbum", "Playlist", "CollectionFolder"] {
            let data = Data("{\"Id\":\"item\",\"Type\":\"\(kind)\"}".utf8)
            XCTAssertTrue(try JSONDecoder().decode(MediaItem.self, from: data).isContainer)
        }
    }
    func testRustBridgeRejectsForeignCredentialsAndUnknownOperations() async throws {
        do {
            let _: EmptyValue = try await RustBridge.call("info", fields: ["server": "https://user:secretnotallowedxyz@example.com"])
            XCTFail("Credential-bearing server URLs must be rejected")
        } catch { XCTAssertFalse(error.localizedDescription.contains("secretnotallowedxyz")) }
        do {
            let _: EmptyValue = try await RustBridge.call("invalid_operation", fields: ["server": "https://example.com"])
            XCTFail("Unknown operations must be rejected")
        } catch { XCTAssertFalse(error.localizedDescription.isEmpty) }
    }
}
