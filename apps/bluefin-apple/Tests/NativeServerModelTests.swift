import XCTest
import AVFoundation
import CryptoKit
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

/// Consumes an externally staged public manifest and original synthetic media
/// produced by the real server model fixture. No model or media ships in the app.
/// This joins server output to native admission; it is not a connected NAS test.
@MainActor final class NativeServerModelTests: XCTestCase {
    private var root: URL?
    private var api: NativePlaybackHTTP?
    private var playback: PlaybackController?

    override func tearDown() async throws {
        var failure: Error?
        if let playback { do { try await playback.stop().value } catch { failure = error } }
        playback = nil
        if let api { await api.stop() }
        api = nil
        if let root {
            do { try FileManager.default.removeItem(at: root) } catch { failure = failure ?? error }
            XCTAssertFalse(FileManager.default.fileExists(atPath: root.path))
        }
        root = nil
        try await super.tearDown()
        if let failure { throw failure }
    }

    func testActualServerModelOutputAdmitsPlaysAndRestoresOriginal() async throws {
        let fixture = try ServerModelOutput.load()
        let manifest = fixture.manifest
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("BluefinServerModel-" + UUID().uuidString)
        self.root = root
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        let video = root.appendingPathComponent("unused-original-colours.mp4")
        try await NativePlaybackFixtures.makeVideo(video, seconds: 1)
        let api = try NativePlaybackHTTP(files: .init(video: video, audio: fixture.original))
        self.api = api
        api.intercept = { request, complete in fixture.respond(request, complete: complete) }
        try await api.start()
        let session = AccountSession(server: api.origin, token: api.token, userId: "native-user",
            username: "Original server model fixture", deviceId: "native-model-xctest", allowHttp: true)
        // Catalog rounding is intentionally separate from the exact public PCM duration.
        let item = MediaItem(Id: manifest.itemId, Name: "Original server model fixture", Type: "Audio",
            RunTimeTicks: Int64((manifest.durationSeconds + 0.01) * 10_000_000))
        let plan = PlaybackPlan(url: fixture.original.absoluteString, playSessionId: UUID().uuidString,
            mediaSourceId: try XCTUnwrap(manifest.mediaSourceId), playMethod: "DirectPlay")
        let controller = try PlaybackController(item: item, url: fixture.original, plan: plan, session: session)
        playback = controller
        try await ready(controller.player)
        let original = try XCTUnwrap(controller.player.currentItem)
        var statuses: [String] = []
        try await KaraokePreparation.enable(playback: controller, session: session,
            currentSession: { session }, status: { statuses.append($0) })
        var sing: KaraokeAudio? = try XCTUnwrap(controller.karaoke)
        XCTAssertEqual(sing?.package.processing, .modelSeparated)
        XCTAssertEqual(sing?.package.processingReference, manifest.processingReference)
        XCTAssertEqual(sing?.duration ?? 0, manifest.durationSeconds, accuracy: 1.0 / 44_100)
        XCTAssertTrue(controller.player.currentItem === sing?.nativeItem)
        XCTAssertFalse(controller.player.allowsExternalPlayback)
        XCTAssertFalse(statuses.contains("Preparing vocals on your server"), "The exact already prepared manifest must be used")
        try await ready(controller.player)
        sing?.setVocalLevel(0.15)
        XCTAssertEqual(sing?.vocalLevel, 0.15)
        XCTAssertEqual(sing?.nativeItem.audioMix?.inputParameters.count, 2)
        controller.resume()
        let deadline = ProcessInfo.processInfo.systemUptime + 8
        while controller.player.currentTime().seconds < 0.4 && ProcessInfo.processInfo.systemUptime < deadline {
            if let error = controller.player.currentItem?.error { throw error }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertGreaterThanOrEqual(controller.player.currentTime().seconds, 0.4)
        controller.player.pause()
        let sought = await controller.player.seek(to: CMTime(seconds: 0.75, preferredTimescale: 44_100),
            toleranceBefore: .zero, toleranceAfter: .zero)
        XCTAssertTrue(sought)
        let position = controller.player.currentTime().seconds
        let directory = try XCTUnwrap(sing?.package.accompaniment.deletingLastPathComponent())
        weak var native = sing?.nativeItem
        await controller.disableKaraoke()
        XCTAssertNil(controller.karaoke)
        XCTAssertTrue(controller.player.currentItem === original)
        XCTAssertTrue(controller.player.allowsExternalPlayback)
        XCTAssertEqual(controller.player.currentTime().seconds, position, accuracy: 0.05)
        XCTAssertTrue(FileManager.default.fileExists(atPath: directory.path))
        sing = nil
        let releaseDeadline = ProcessInfo.processInfo.systemUptime + 5
        while native != nil && ProcessInfo.processInfo.systemUptime < releaseDeadline { try await Task.sleep(nanoseconds: 20_000_000) }
        XCTAssertNil(native)
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.path))
        let requests = await api.requests()
        XCTAssertFalse(requests.contains { $0.method == "POST" && $0.path.contains("/Preparation") })
        XCTAssertEqual(requests.filter { $0.path.contains("/Stems/") }.count, 2)
        XCTAssertTrue(requests.filter { $0.path.contains("/Stems/") }.allSatisfy {
            $0.headers["authorization"]?.hasPrefix("MediaBrowser ") == true && !$0.path.contains(api.token)
        })
    }

    private func ready(_ player: AVPlayer) async throws {
        let deadline = ProcessInfo.processInfo.systemUptime + 10
        while player.currentItem?.status == .unknown && ProcessInfo.processInfo.systemUptime < deadline {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertEqual(player.currentItem?.status, .readyToPlay, player.currentItem?.error?.localizedDescription ?? "No item")
        await Task.yield()
    }
}

private struct ServerModelOutput {
    let manifest: KaraokePreparationManifest
    let bytes: Data
    let accompaniment: Data
    let vocal: Data
    let original: URL

    static func load() throws -> Self {
        guard let resources = Bundle(for: NativeServerModelTests.self).resourceURL else { throw XCTSkip("No test bundle resources") }
        let directory = resources.appendingPathComponent("ServerModelFixture", isDirectory: true)
        guard FileManager.default.fileExists(atPath: directory.path) else {
            throw XCTSkip("Stage the real server fixture in the test bundle to run model-output acceptance. See qa/server-model-native/README.md.")
        }
        func read(_ name: String) throws -> Data {
            let url = directory.appendingPathComponent(name)
            let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
            guard attributes[.type] as? FileAttributeType == .typeRegular,
                  let size = attributes[.size] as? NSNumber, (1...1_048_576).contains(size.intValue) else {
                throw NativePlaybackFixtures.Failure.invalid("Invalid bounded server model fixture")
            }
            let bytes = try Data(contentsOf: url)
            print("Server model output \(name): \(bytes.count) bytes SHA256 \(digest(bytes))")
            return bytes
        }
        let bytes = try read("public-manifest.json")
        let manifest = try JSONDecoder().decode(KaraokePreparationManifest.self, from: bytes)
        try manifest.validate(itemID: manifest.itemId, mediaSourceID: manifest.mediaSourceId)
        guard manifest.processing == "modelSeparatedStems", (1...3).contains(manifest.durationSeconds) else {
            throw NativePlaybackFixtures.Failure.invalid("Expected the short real server model fixture")
        }
        let accompaniment = try read("accompaniment.wav"), vocal = try read("lead-vocal.wav")
        for (data, descriptor) in [(accompaniment, manifest.accompaniment), (vocal, manifest.leadVocal)] {
            guard UInt64(data.count) == descriptor.bytes, digest(data) == descriptor.sha256 else {
                throw NativePlaybackFixtures.Failure.invalid("Server fixture integrity mismatch")
            }
        }
        _ = try read("original-encoded.flac")
        return Self(manifest: manifest, bytes: bytes, accompaniment: accompaniment, vocal: vocal,
            original: directory.appendingPathComponent("original-encoded.flac"))
    }

    func respond(_ request: NativePlaybackHTTP.Request, complete: @escaping (NativePlaybackHTTP.Response) -> Void) -> Bool {
        let path = request.path.components(separatedBy: "?")[0]
        let base = "/Bluefin/Karaoke/" + manifest.itemId
        guard path == base || path.hasPrefix(base + "/") else { return false }
        let components = URLComponents(string: "http://127.0.0.1" + request.path)
        guard request.method == "GET", components?.queryItems?.contains(where: {
            $0.name == "MediaSourceId" && $0.value == manifest.mediaSourceId
        }) == true else { complete(.init(bytes: Data(), status: "400 Bad Request")); return true }
        if path == base { complete(.init(bytes: bytes)) }
        else if path == base + "/" + manifest.revision + "/Stems/accompaniment" { complete(.init(bytes: accompaniment, type: "audio/wav")) }
        else if path == base + "/" + manifest.revision + "/Stems/lead-vocal" { complete(.init(bytes: vocal, type: "audio/wav")) }
        else { complete(.init(bytes: Data(), status: "404 Not Found")) }
        return true
    }

    private static func digest(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
}
