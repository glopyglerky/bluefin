import XCTest
import AVFoundation
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

final class BluefinTests: XCTestCase {
    @MainActor func testBundledPreviewIsOriginalLocalAndDecodable() async throws {
        XCTAssertFalse(MediaItem.preview.isEmpty)
        for item in MediaItem.preview {
            let url = try XCTUnwrap(item.previewURL, "Preview video is missing from the built app")
            XCTAssertTrue(url.isFileURL)
            let asset = AVURLAsset(url: url)
            let playable = try await asset.load(.isPlayable)
            let duration = try await asset.load(.duration)
            XCTAssertTrue(playable)
            XCTAssertEqual(duration.seconds, 30, accuracy: 0.1)
        }
        let audioURL = try XCTUnwrap(MediaItem.previewAudio.previewURL, "Preview audio is missing from the built app")
        XCTAssertTrue(audioURL.isFileURL)
        let audio = try AVAudioFile(forReading: audioURL)
        XCTAssertEqual(audio.fileFormat.streamDescription.pointee.mFormatID, kAudioFormatAppleLossless)
        XCTAssertEqual(audio.fileFormat.channelCount, 1)
        XCTAssertNotNil(Bundle.main.url(forResource: "PrivacyInfo", withExtension: "xcprivacy"))
    }
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
