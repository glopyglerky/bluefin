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


/// These fixtures hold the same task used by the real decoder/FFI. They exercise
/// presentation ownership without opening a browser or changing saved accounts.
@MainActor final class MeshPairingPresentationTests: XCTestCase {
    private var preview: MeshInvitePreview {
        MeshInvitePreview(code: "synthetic-local-invitation", link: "bluefin://synthetic",
            expiresAtUnixMs: 1, intent: "login", accountLinked: true, singleUse: true,
            service: "bluefin/connect/1")
    }

    func testOtherWindowCannotCancelWorkOrClearTheOwnersPreview() async throws {
        let entered = expectation(description: "decoder entered")
        var continuation: CheckedContinuation<MeshInvitePreview, Error>?
        var cancelled = false
        let model = MeshPairingModel { _ in
            let value: MeshInvitePreview = try await withCheckedThrowingContinuation {
                continuation = $0; entered.fulfill()
            }
            cancelled = Task.isCancelled
            return value
        }
        let owner = UUID(), other = UUID()
        XCTAssertTrue(model.claimPresentation(owner))
        let work = try XCTUnwrap(model.decode("synthetic", owner: owner))
        await fulfillment(of: [entered], timeout: 5)
        XCTAssertFalse(model.claimPresentation(other))
        XCTAssertNil(model.decode("stale input", owner: other))
        model.releasePresentation(other)
        model.reportProblem("stale image failure", owner: other)
        XCTAssertTrue(model.busy)
        XCTAssertNil(model.problem)
        XCTAssertEqual(model.presentationOwner, owner)
        continuation?.resume(returning: preview)
        continuation = nil
        await work.value
        XCTAssertFalse(cancelled)
        XCTAssertFalse(model.busy)
        XCTAssertEqual(model.preview?.code, preview.code)
        model.releasePresentation(other)
        XCTAssertEqual(model.preview?.code, preview.code)
        XCTAssertTrue(model.ownsPresentation(owner))
        model.releasePresentation(owner)
        XCTAssertNil(model.preview)
        XCTAssertNil(model.presentationOwner)
    }

    func testDismissedOwnerRetainsTheSlotUntilItsPhysicalWorkReturns() async throws {
        let entered = expectation(description: "decoder entered")
        var continuation: CheckedContinuation<MeshInvitePreview, Error>?
        var cancelled = false
        let model = MeshPairingModel { _ in
            let value: MeshInvitePreview = try await withCheckedThrowingContinuation {
                continuation = $0; entered.fulfill()
            }
            cancelled = Task.isCancelled
            return value
        }
        let owner = UUID(), replacement = UUID()
        XCTAssertTrue(model.claimPresentation(owner))
        let work = try XCTUnwrap(model.decode("synthetic", owner: owner))
        await fulfillment(of: [entered], timeout: 5)
        model.releasePresentation(owner)
        XCTAssertTrue(model.busy)
        XCTAssertEqual(model.presentationOwner, owner)
        XCTAssertFalse(model.ownsPresentation(owner))
        XCTAssertFalse(model.claimPresentation(replacement))
        XCTAssertNil(model.decode("replacement input", owner: replacement))
        continuation?.resume(returning: preview)
        continuation = nil
        await work.value
        XCTAssertTrue(cancelled)
        XCTAssertFalse(model.busy)
        XCTAssertNil(model.preview, "A cancelled decoder must not publish its old input")
        XCTAssertNil(model.presentationOwner)
        XCTAssertTrue(model.claimPresentation(replacement))
        model.releasePresentation(owner)
        XCTAssertTrue(model.ownsPresentation(replacement), "A late dismissal cannot release the new owner")
        model.releasePresentation(replacement)
    }
}
