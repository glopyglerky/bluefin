import XCTest
import AVFoundation
import CryptoKit
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

/// Production Swift controller and Rust FFI against a bounded synthetic HTTP server.
/// The fixture supplies original tones; it does not exercise server model inference.
@MainActor final class NativeKaraokeTests: XCTestCase {
    private var root: URL!
    private var api: NativePlaybackHTTP!
    private var session: AccountSession!
    private var fixture: KaraokeProtocolFixture!
    private var playback: PlaybackController?

    override func setUp() async throws {
        try await super.setUp()
        root = FileManager.default.temporaryDirectory.appendingPathComponent("BluefinNativeSing-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let files = try await NativePlaybackFixtures.generate(in: root)
        fixture = try KaraokeProtocolFixture(in: root)
        api = try NativePlaybackHTTP(files: files)
        api.intercept = { [fixture] request, complete in fixture!.respond(request, complete: complete) }
        try await api.start()
        session = AccountSession(server: api.origin, token: api.token, userId: "native-user",
            username: "Original synthetic Sing", deviceId: "native-sing-xctest", allowHttp: true)
    }

    override func tearDown() async throws {
        var failure: Error?
        if let playback { do { try await playback.stop().value } catch { failure = error } }
        playback = nil
        if let api { await api.stop() }
        fixture = nil; api = nil; session = nil
        if let root {
            do { try FileManager.default.removeItem(at: root) } catch { failure = failure ?? error }
            XCTAssertFalse(FileManager.default.fileExists(atPath: root.path))
        }
        root = nil
        try await super.tearDown()
        if let failure { throw failure }
    }

    func testPreparedStemsBecomeNativeSingAndRestoreOriginalPlayback() async throws {
        let controller = try await controller()
        let original = try XCTUnwrap(controller.player.currentItem)
        var statuses: [String] = []
        try await KaraokePreparation.enable(playback: controller, session: session,
            currentSession: { self.session }, status: { statuses.append($0) })
        var sing: KaraokeAudio? = try XCTUnwrap(controller.karaoke)
        XCTAssertEqual(sing?.package.processing, .modelSeparated)
        XCTAssertEqual(sing?.package.processingReference, "deezer/spleeter/2stems@v1.4.0")
        XCTAssertFalse(controller.player.allowsExternalPlayback)
        XCTAssertTrue(statuses.contains("Preparing vocals on your server"))
        XCTAssertTrue(controller.player.currentItem === sing?.nativeItem)
        try await ready(controller.player)
        sing?.setVocalLevel(0.25)
        XCTAssertEqual(sing?.vocalLevel, 0.25)
        let stem = try XCTUnwrap(sing?.package.accompaniment)
        weak var native = sing?.nativeItem
        controller.resume()
        try await clock(controller.player, after: 0.25)
        controller.player.pause()
        let position = controller.player.currentTime().seconds
        await controller.disableKaraoke()
        XCTAssertNil(controller.karaoke)
        XCTAssertTrue(controller.player.currentItem === original)
        XCTAssertTrue(controller.player.allowsExternalPlayback)
        XCTAssertEqual(controller.player.currentTime().seconds, position, accuracy: 0.05)
        XCTAssertTrue(FileManager.default.fileExists(atPath: stem.path), "The retained native consumer still owns its inputs")
        sing = nil
        let deadline = ProcessInfo.processInfo.systemUptime + 5
        while native != nil && ProcessInfo.processInfo.systemUptime < deadline { try await Task.sleep(nanoseconds: 20_000_000) }
        XCTAssertNil(native)
        XCTAssertFalse(FileManager.default.fileExists(atPath: stem.deletingLastPathComponent().path))
        let requests = await api.requests()
        let post = try XCTUnwrap(requests.first { $0.method == "POST" && $0.path.hasSuffix("/Preparation") })
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: post.body) as? [String: Any])
        let job = try XCTUnwrap(body["requestId"] as? String)
        XCTAssertEqual(body["mediaSourceId"] as? String, "audio")
        XCTAssertTrue(requests.contains { $0.method == "GET" && $0.path == "/Bluefin/Karaoke/audio/Preparation/\(job)" })
        let stems = requests.filter { $0.path.contains("/Stems/") }
        XCTAssertEqual(stems.count, 2)
        XCTAssertTrue(stems.allSatisfy { $0.path.contains("MediaSourceId=audio") })
        XCTAssertFalse(requests.contains { $0.path.contains(session.token) })
    }

    func testExplicitStemOnlyPreparationPlaysAndRestoresOriginalWithoutInventedTiming() async throws {
        fixture.timingKind = "none"
        fixture.includeTimeline = false
        let controller = try await controller()
        let original = try XCTUnwrap(controller.player.currentItem)
        try await KaraokePreparation.enable(playback: controller, session: session,
            currentSession: { self.session }, status: { _ in })
        let sing = try XCTUnwrap(controller.karaoke)
        XCTAssertNil(sing.package.timeline)
        XCTAssertEqual(sing.package.timingKind, KaraokeTimingKind.none)
        try await ready(controller.player)
        sing.setVocalLevel(0)
        XCTAssertEqual(sing.vocalLevel, 0)
        controller.resume()
        try await clock(controller.player, after: 0.25)
        controller.player.pause()
        let position = controller.player.currentTime().seconds
        await controller.disableKaraoke()
        XCTAssertTrue(controller.player.currentItem === original)
        XCTAssertEqual(controller.player.currentTime().seconds, position, accuracy: 0.05)
    }

    func testLegacyPackageWithMissingTimingIsRejectedBeforeDownloadingStems() async throws {
        fixture.includeTimeline = false
        let controller = try await controller()
        do {
            try await KaraokePreparation.enable(playback: controller, session: session,
                currentSession: { self.session }, status: { _ in })
            XCTFail("A corrupt legacy word package was treated as stem-only audio")
        } catch KaraokeError.invalidTiming {} catch { XCTFail("Unexpected error: \(error)") }
        XCTAssertNil(controller.karaoke)
        let requests = await api.requests()
        XCTAssertFalse(requests.contains { $0.path.contains("/Stems/") })
    }

    func testCancellationReachesServerBeforeDelayedPreparationReply() async throws {
        let controller = try await controller()
        let began = expectation(description: "Preparation POST held by server")
        let cancelled = expectation(description: "Known job DELETE arrived before POST reply")
        fixture.holdPreparation = true
        fixture.onStart = { began.fulfill() }
        fixture.onCancel = { cancelled.fulfill() }
        let task = Task { try await KaraokePreparation.enable(playback: controller, session: session,
            currentSession: { self.session }, status: { _ in }) }
        await fulfillment(of: [began], timeout: 8)
        // The held POST owns the first ordinary slot. Fill the other two so only
        // the reserved cleanup path can issue DELETE and release that response.
        let second = try await CoreAdmission.shared.acquire()
        let third = try await CoreAdmission.shared.acquire()
        task.cancel()
        await fulfillment(of: [cancelled], timeout: 5)
        await CoreAdmission.shared.release(second)
        await CoreAdmission.shared.release(third)
        do { try await task.value; XCTFail("Cancelled preparation became active") } catch {}
        XCTAssertNil(controller.karaoke)
        let requests = await api.requests()
        let post = try XCTUnwrap(requests.first { $0.method == "POST" && $0.path.hasSuffix("/Preparation") })
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: post.body) as? [String: Any])
        let job = try XCTUnwrap(body["requestId"] as? String)
        XCTAssertTrue(requests.contains { $0.method == "DELETE" && $0.path == "/Bluefin/Karaoke/audio/Preparation/\(job)" })
        XCTAssertFalse(requests.contains { $0.path.contains("/Stems/") })
    }

    func testPreparationUsesPlannerSourceWhenRawMetadataOmitsItsID() async throws {
        let controller = try await controller(omitRawSourceID: true)
        XCTAssertNil(controller.source?.Id)
        try await KaraokePreparation.enable(playback: controller, session: session,
            currentSession: { self.session }, status: { _ in })
        XCTAssertNotNil(controller.karaoke)
    }

    func testDeclaredDurationRejectsLongerStemsWithoutTrackDurationMetadata() async throws {
        try await mismatchedDurationRefused(stemSeconds: 8)
    }

    func testDeclaredDurationRejectsShorterStemsWithoutTrackDurationMetadata() async throws {
        try await mismatchedDurationRefused(stemSeconds: 2)
    }

    private func mismatchedDurationRefused(stemSeconds: Int) async throws {
        try fixture.replaceStems(in: root, seconds: stemSeconds)
        let controller = try await controller(includeRuntimeMetadata: false)
        XCTAssertNil(controller.item.RunTimeTicks)
        let original = try XCTUnwrap(controller.player.currentItem)
        func leases() throws -> Set<URL> {
            Set(try FileManager.default.contentsOfDirectory(at: FileManager.default.temporaryDirectory,
                includingPropertiesForKeys: nil).filter { $0.lastPathComponent.hasPrefix("bluefin-karaoke-") })
        }
        let before = try leases()
        do {
            try await KaraokePreparation.enable(playback: controller, session: session,
                currentSession: { self.session }, status: { _ in })
            XCTFail("Stems inconsistent with their declared duration became native Sing")
        } catch KaraokeError.invalidStems {} catch { XCTFail("Unexpected admission failure: \(error)") }
        XCTAssertNil(controller.karaoke)
        XCTAssertTrue(controller.player.currentItem === original)
        XCTAssertTrue(try leases().subtracting(before).isEmpty, "Rejected package retained a temporary lease")
    }

    func testAccountChangeNeverPublishesPreparedStems() async throws {
        try await staleSessionRefused { $0.userId = "another-synthetic-account" }
    }

    func testDeviceIdentityChangeNeverPublishesPreparedStems() async throws {
        try await staleSessionRefused { $0.deviceId = "another-synthetic-device" }
    }

    private func staleSessionRefused(_ change: (inout AccountSession) -> Void) async throws {
        let controller = try await controller()
        let began = expectation(description: "Preparation admitted")
        let cancelled = expectation(description: "Stale session job cancelled")
        fixture.onStart = { began.fulfill() }
        fixture.onCancel = { cancelled.fulfill() }
        var current = session!
        let task = Task { try await KaraokePreparation.enable(playback: controller, session: session,
            currentSession: { current }, status: { _ in }) }
        await fulfillment(of: [began], timeout: 8)
        change(&current)
        do { try await task.value; XCTFail("A changed session inherited Sing preparation") } catch {}
        await fulfillment(of: [cancelled], timeout: 5)
        XCTAssertNil(controller.karaoke)
        let requests = await api.requests()
        XCTAssertFalse(requests.contains { $0.path.contains("/Stems/") })
    }

    private func controller(omitRawSourceID: Bool = false, includeRuntimeMetadata: Bool = true) async throws -> PlaybackController {
        var plan: PlaybackPlan = try await RustBridge.call("playback", session: session,
            fields: ["itemId": "audio", "mediaType": "Audio"])
        if omitRawSourceID { plan.source?.Id = nil }
        var metadata: [String: Any] = ["Id": "audio", "Name": "Original synthetic tone", "Type": "Audio"]
        if includeRuntimeMetadata { metadata["RunTimeTicks"] = 40_000_000 }
        let item = try JSONDecoder().decode(MediaItem.self, from: JSONSerialization.data(withJSONObject: metadata))
        let controller = try PlaybackController(item: item, url: XCTUnwrap(URL(string: plan.url)), plan: plan, session: session)
        playback = controller
        try await ready(controller.player)
        return controller
    }

    private func ready(_ player: AVPlayer) async throws {
        let deadline = ProcessInfo.processInfo.systemUptime + 10
        while player.currentItem?.status == .unknown && ProcessInfo.processInfo.systemUptime < deadline {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertEqual(player.currentItem?.status, .readyToPlay, player.currentItem?.error?.localizedDescription ?? "No item")
        // The controller's status observer finishes its initial seek on the main actor.
        await Task.yield()
    }

    private func clock(_ player: AVPlayer, after seconds: Double) async throws {
        let deadline = ProcessInfo.processInfo.systemUptime + 8
        while player.currentTime().seconds < seconds && ProcessInfo.processInfo.systemUptime < deadline {
            if let error = player.currentItem?.error { throw error }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertGreaterThanOrEqual(player.currentTime().seconds, seconds)
    }
}

/// State is confined to NativePlaybackHTTP.queue after configuration, before each task starts.
private final class KaraokeProtocolFixture {
    var holdPreparation = false
    var timingKind: String?
    var includeTimeline = true
    var onStart: (() -> Void)?
    var onCancel: (() -> Void)?
    private var job: String?
    private var ready = false
    private var cancelled = false
    private var held: ((NativePlaybackHTTP.Response) -> Void)?
    private var accompaniment = Data()
    private var vocal = Data()
    private let revision = String(repeating: "a", count: 64)

    init(in root: URL) throws {
        try replaceStems(in: root, seconds: 4)
    }

    /// Configured while no preparation request is running; original media stays unchanged.
    func replaceStems(in root: URL, seconds: Int) throws {
        guard (1...8).contains(seconds) else { throw NativePlaybackFixtures.Failure.invalid("Stem duration") }
        func tone(_ name: String, frequency: Double) throws -> Data {
            let url = root.appendingPathComponent(name)
            do {
                let format = AVAudioFormat(standardFormatWithSampleRate: 44_100, channels: 1)!
                let file = try AVAudioFile(forWriting: url, settings: [AVFormatIDKey: kAudioFormatLinearPCM,
                    AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 1, AVLinearPCMBitDepthKey: 16,
                    AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false])
                let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(seconds * 44_100))!
                buffer.frameLength = buffer.frameCapacity
                for i in 0..<Int(buffer.frameLength) {
                    buffer.floatChannelData![0][i] = Float(0.1 * sin(2 * .pi * frequency * Double(i) / 44_100))
                }
                try file.write(from: buffer)
            }
            return try Data(contentsOf: url)
        }
        accompaniment = try tone("original-backing.wav", frequency: 440)
        vocal = try tone("original-vocal.wav", frequency: 880)
    }

    func respond(_ request: NativePlaybackHTTP.Request, complete: @escaping (NativePlaybackHTTP.Response) -> Void) -> Bool {
        let path = request.path.components(separatedBy: "?")[0]
        guard path == "/Bluefin/Karaoke/audio" || path.hasPrefix("/Bluefin/Karaoke/audio/") else { return false }
        func json(_ value: Any) {
            do { complete(.init(bytes: try JSONSerialization.data(withJSONObject: value))) }
            catch { complete(.init(bytes: Data(), status: "500 Fixture encoding failed")) }
        }
        func status(_ state: String) -> [String: Any] {
            ["itemId": "audio", "jobId": job ?? "", "mediaSourceId": "audio", "state": state, "retryAfterMilliseconds": 500]
        }
        if path == "/Bluefin/Karaoke/audio/Preparation", request.method == "POST" {
            guard let body = try? JSONSerialization.jsonObject(with: request.body) as? [String: Any],
                  let id = body["requestId"] as? String, UUID(uuidString: id) != nil,
                  body["mediaSourceId"] as? String == "audio" else {
                complete(.init(bytes: Data(), status: "400 Bad Request")); return true
            }
            job = id
            if holdPreparation { held = complete } else { json(status("preparing")) }
            onStart?()
        } else if let job, path == "/Bluefin/Karaoke/audio/Preparation/\(job)" {
            if request.method == "DELETE" {
                let first = !cancelled; cancelled = true
                json(status("cancelled"))
                if let held {
                    self.held = nil
                    held(.init(bytes: (try? JSONSerialization.data(withJSONObject: status("cancelled"))) ?? Data()))
                }
                if first { onCancel?() }
            } else if request.method == "GET" { ready = true; json(status(cancelled ? "cancelled" : "ready")) }
            else { complete(.init(bytes: Data(), status: "405 Method Not Allowed")) }
        } else if path == "/Bluefin/Karaoke/audio", ready && !cancelled {
            func descriptor(_ bytes: Data) -> [String: Any] {
                ["bytes": bytes.count, "sha256": SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()]
            }
            var manifest: [String: Any] = ["itemId": "audio", "mediaSourceId": "audio", "revision": revision,
                  "durationSeconds": 4.0, "processing": "modelSeparatedStems",
                  "processingReference": "deezer/spleeter/2stems@v1.4.0",
                  "accompaniment": descriptor(accompaniment), "leadVocal": descriptor(vocal)]
            if includeTimeline {
                manifest["timeline"] = ["offsetSeconds": 0.0, "lines": [["id": "original", "voice": "lead",
                    "tokens": [["text": "Original test tone", "start": 0.0, "end": 1.0]]]]]
            }
            if let timingKind { manifest["timingKind"] = timingKind }
            json(manifest)
        } else if ready && !cancelled && path.hasPrefix("/Bluefin/Karaoke/audio/\(revision)/Stems/") {
            let bytes = path.hasSuffix("/accompaniment") ? accompaniment : vocal
            complete(.init(bytes: bytes, type: "audio/wav"))
        } else { complete(.init(bytes: Data(), status: "404 Not Found")) }
        return true
    }
}
