import XCTest
import AVFoundation
import CryptoKit
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

/// Run serially in the app-hosted BluefinTests scheme with the parent's final Rust archive.
/// These are synthetic-server native checks; they do not qualify a production server or audio route.
@MainActor final class NativePlaybackTests: XCTestCase {
    private var root: URL!
    private var files: NativePlaybackFixtures.Files!
    private var api: NativePlaybackHTTP!
    private var session: AccountSession!
    private var plans: [PlaybackPlan] = []
    private var availableBefore: Int64 = 0
    private struct Page: Decodable { let Items: [OfflineRecord] }

    override func setUp() async throws {
        try await super.setUp()
        let fm = FileManager.default
        availableBefore = availableBytes()
        root = fm.temporaryDirectory.appendingPathComponent("BluefinNativeAcceptance-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        root = root.resolvingSymlinksInPath()
        #if os(iOS)
        try fm.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: root.path)
        #endif
        var values = URLResourceValues(); values.isExcludedFromBackup = true
        try root.setResourceValues(values)
        files = try await NativePlaybackFixtures.generate(in: root)
        for url in [files.video, files.audio] {
            let bytes = try Data(contentsOf: url)
            print("Native fixture \(url.lastPathComponent): \(bytes.count) bytes SHA256 \(SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined())")
        }
        api = try NativePlaybackHTTP(files: files); try await api.start()
        session = AccountSession(server: api.origin, token: api.token, userId: "native-user",
                                 username: "Synthetic XCTest", deviceId: "native-xctest", allowHttp: true)
    }

    override func tearDown() async throws {
        var cleanupFailure: Error?
        if let session {
            for plan in plans {
                do { try await RustBridge.cleanupPlaybackPlan(plan, session: session) }
                catch { cleanupFailure = cleanupFailure ?? error }
            }
            if let root {
                for id in ["audio", "cancel"] {
                    do { try await RustBridge.notify("downloadRemove", session: session,
                        fields: ["cacheRoot": root.path, "itemId": id]) }
                    catch { cleanupFailure = cleanupFailure ?? error }
                }
            }
        }
        if let api { await api.stop() }
        if let root, FileManager.default.fileExists(atPath: root.path) {
            do { try FileManager.default.removeItem(at: root) }
            catch { cleanupFailure = cleanupFailure ?? error }
            XCTAssertFalse(FileManager.default.fileExists(atPath: root.path), "Owned test root survived cleanup")
        }
        print("Native test available bytes before=\(availableBefore) after=\(availableBytes()); volume-wide delta includes concurrent owners")
        plans = []; api = nil; session = nil; files = nil; root = nil
        try await super.tearDown()
        if let cleanupFailure { throw cleanupFailure }
    }

    func testRustCapabilityVideoDecodesAndReleasesAcrossProxyCapacity() async throws {
        // More sequential sessions than the two-proxy bound proves slots can be reused after stop.
        for _ in 0..<3 {
            let plan: PlaybackPlan = try await RustBridge.call("playback", session: session,
                fields: ["itemId": "video", "mediaType": "Video"])
            plans.append(plan)
            let url = try XCTUnwrap(URL(string: plan.url))
            XCTAssertEqual(url.host, "127.0.0.1")
            XCTAssertNotEqual(url.port, URL(string: session.server)?.port)
            XCTAssertFalse(plan.url.contains(session.token))
            XCTAssertNotNil(plan.proxyId)
            try await checkRangeAndHead(url)
            try await report("started", plan: plan, item: "video", ticks: 0)
            try await play(url, video: true)
            try await report("progress", plan: plan, item: "video", ticks: 4_000_000)
            try await report("stopped", plan: plan, item: "video", ticks: 4_000_000)
            try await RustBridge.cleanupPlaybackPlan(plan, session: session)
            plans.removeLast()
            // Await the actual network error; a timeout does not count as a stopped listener.
            let configuration = URLSessionConfiguration.ephemeral
            configuration.timeoutIntervalForRequest = 2
            let client = URLSession(configuration: configuration)
            defer { client.invalidateAndCancel() }
            do { _ = try await client.data(from: url); XCTFail("Released capability still serves bytes") }
            catch let error as URLError { XCTAssertNotEqual(error.code, .timedOut, "Stop must close the listener") }
        }
        let requests = await api.requests()
        XCTAssertTrue(requests.contains { $0.path.hasPrefix("/Videos/video/stream.mp4") })
        let stopped = try XCTUnwrap(requests.last { $0.path == "/Sessions/Playing/Stopped" })
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: stopped.body) as? [String: Any])
        XCTAssertEqual(body["ItemId"] as? String, "video")
        XCTAssertEqual(body["PlaySessionId"] as? String, "native-session-video")
        XCTAssertEqual((body["PositionTicks"] as? NSNumber)?.int64Value, 4_000_000)
    }

    func testLosslessDirectAudioAndOfflineDownloadLocalPlaybackRemoval() async throws {
        // Validate the fixture through Apple's decoder, not a guessed container signature.
        let original = try AVAudioFile(forReading: files.audio)
        XCTAssertEqual(original.fileFormat.streamDescription.pointee.mFormatID, kAudioFormatAppleLossless)
        XCTAssertGreaterThan(original.length, 44_100)
        try await play(files.audio, video: false)
        let plan: PlaybackPlan = try await RustBridge.call("playback", session: session,
            fields: ["itemId": "audio", "mediaType": "Audio"])
        plans.append(plan)
        XCTAssertEqual(plan.playMethod, "DirectPlay")
        XCTAssertEqual(plan.source?.Container, "m4a")
        let (proxyBytes, _) = try await URLSession.shared.data(from: XCTUnwrap(URL(string: plan.url)))
        XCTAssertEqual(proxyBytes, try Data(contentsOf: files.audio), "Capability proxy changed encoded audio bytes")
        do { try await play(try XCTUnwrap(URL(string: plan.url)), video: false) }
        catch {
            let requests = await api.requests()
            let ranges = requests.map { $0.method + " " + $0.path + " range=" + ($0.headers["range"] ?? "none") }
            print("Native synthetic request ranges: \(ranges)")
            throw error
        }
        try await report("stopped", plan: plan, item: "audio", ticks: 4_000_000)
        try await RustBridge.cleanupPlaybackPlan(plan, session: session); plans.removeLast()
        let _: OfflineRecord = try await RustBridge.call("downloadStart", session: session,
            fields: ["cacheRoot": root.path, "itemId": "audio", "maxBytes": 1_048_576, "quotaBytes": 2_097_152])
        let record = try await completedDownload("audio")
        XCTAssertEqual(record.state, "completed", record.error ?? "No download error supplied")
        let local = try XCTUnwrap(record.localUrl.flatMap(URL.init(string:)))
        XCTAssertTrue(local.isFileURL)
        XCTAssertEqual(local.pathExtension, "m4a", "Offline media must retain its admitted native container")
        XCTAssertTrue(local.resolvingSymlinksInPath().path.hasPrefix(root.path + "/"))
        XCTAssertEqual(try Data(contentsOf: local), try Data(contentsOf: files.audio))
        var other = session!
        other.userId = "another-synthetic-user"
        let isolated: Page = try await RustBridge.call("downloads", session: other, fields: ["cacheRoot": root.path])
        XCTAssertTrue(isolated.Items.isEmpty, "Another account can see the completed download")
        // Stop the fixture server before local playback; this must work without network service.
        await api.stop()
        try await play(local, video: false)
        try await RustBridge.notify("downloadRemove", session: session, fields: ["cacheRoot": root.path, "itemId": "audio"])
        XCTAssertFalse(FileManager.default.fileExists(atPath: local.path))
        let page: Page = try await RustBridge.call("downloads", session: session, fields: ["cacheRoot": root.path])
        XCTAssertFalse(page.Items.contains { $0.itemId == "audio" && $0.state == "completed" })
    }

    func testOfflineCancellationAndSymlinkRootRefusal() async throws {
        let began = expectation(description: "Download response body opened")
        api.onHeldDownload = { began.fulfill() }
        let _: OfflineRecord = try await RustBridge.call("downloadStart", session: session,
            fields: ["cacheRoot": root.path, "itemId": "cancel", "maxBytes": 1_048_576, "quotaBytes": 2_097_152])
        await fulfillment(of: [began], timeout: 8)
        let begin = Date()
        // Core cancellation joins its worker. A stalled read has a documented 30-second ceiling.
        try await RustBridge.notify("downloadCancel", session: session, fields: ["cacheRoot": root.path, "itemId": "cancel"])
        XCTAssertLessThan(Date().timeIntervalSince(begin), 35)
        let page: Page = try await RustBridge.call("downloads", session: session, fields: ["cacheRoot": root.path])
        let record = try XCTUnwrap(page.Items.first { $0.itemId == "cancel" })
        XCTAssertEqual(record.state, "cancelled", record.error ?? "")
        XCTAssertNil(record.localUrl)
        let paths = try FileManager.default.subpathsOfDirectory(atPath: root.path)
        XCTAssertFalse(paths.contains { $0.hasSuffix("download.part") })
        let alias = root.appendingPathComponent("root-alias")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: root)
        do {
            let _: Page = try await RustBridge.call("downloads", session: session, fields: ["cacheRoot": alias.path])
            XCTFail("Unresolved symlink root was admitted")
        } catch { XCTAssertFalse(error.localizedDescription.isEmpty) }
    }

    private func checkRangeAndHead(_ url: URL) async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 5
        let client = URLSession(configuration: configuration)
        defer { client.invalidateAndCancel() }
        var head = URLRequest(url: url); head.httpMethod = "HEAD"
        let (empty, metadata) = try await client.data(for: head)
        XCTAssertTrue(empty.isEmpty)
        XCTAssertEqual((metadata as? HTTPURLResponse)?.statusCode, 200)
        var range = URLRequest(url: url); range.setValue("bytes=0-15", forHTTPHeaderField: "Range")
        let (prefix, response) = try await client.data(for: range)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 206)
        XCTAssertEqual(prefix, try Data(contentsOf: files.video).prefix(16))
        XCTAssertEqual((response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Content-Range"),
                       "bytes 0-15/\(try Data(contentsOf: files.video).count)")
    }

    private func report(_ operation: String, plan: PlaybackPlan, item: String, ticks: Int64) async throws {
        try await RustBridge.notify(operation, session: session, fields: ["itemId": item,
            "mediaSourceId": plan.mediaSourceId, "playSessionId": plan.playSessionId,
            "playMethod": plan.playMethod ?? "DirectPlay", "positionTicks": ticks])
    }

    private func completedDownload(_ id: String) async throws -> OfflineRecord {
        // Bounded observation of asynchronous disk work. No fixed sleep assumes completion.
        let timer = DispatchSource.makeTimerSource(queue: .global())
        let ticks = AsyncStream<Void>(bufferingPolicy: .bufferingNewest(1)) { continuation in
            timer.setEventHandler { continuation.yield(()) }
            timer.schedule(deadline: .now(), repeating: .milliseconds(50)); timer.resume()
        }
        defer { timer.cancel() }
        let deadline = Date().addingTimeInterval(12)
        for await _ in ticks {
            let page: Page = try await RustBridge.call("downloads", session: session, fields: ["cacheRoot": root.path])
            if let record = page.Items.first(where: { $0.itemId == id }), record.state != "downloading" {
                guard record.state == "completed" else {
                    throw NativePlaybackFixtures.Failure.invalid("Download \(record.state): \(record.error ?? "no reason")")
                }
                return record
            }
            if Date() >= deadline { throw NativePlaybackFixtures.Failure.invalid("Download did not complete before deadline") }
        }
        throw NativePlaybackFixtures.Failure.invalid("Download observation ended")
    }

    private func play(_ url: URL, video: Bool) async throws {
        #if os(iOS)
        try AVAudioSession.sharedInstance().setCategory(.playback)
        try AVAudioSession.sharedInstance().setActive(true)
        defer {
            do { try AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation) }
            catch { XCTFail("Audio session deactivation failed: \(error)") }
        }
        #endif
        let item = AVPlayerItem(url: url)
        let player = AVPlayer(playerItem: item)
        let output = AVPlayerItemVideoOutput(pixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA])
        if video { item.add(output) }
        let ready = expectation(description: "AVPlayerItem ready or failed: \(url.lastPathComponent)")
        var observedStatus = false; var observedTime = false; var observedFrame = false
        let observation = item.observe(\.status, options: [.initial, .new]) { item, _ in
            if item.status != .unknown && !observedStatus { observedStatus = true; ready.fulfill() }
        }
        defer {
            player.pause(); observation.invalidate()
            player.replaceCurrentItem(with: nil)
            XCTAssertNil(player.currentItem)
        }
        await fulfillment(of: [ready], timeout: 10)
        if let error = item.error {
            let events = item.errorLog()?.events.map { String($0.errorStatusCode) + ":" + ($0.errorComment ?? "") } ?? []
            print("Native media failure suffix=\(url.pathExtension), file=\(url.isFileURL), events=\(events)")
            throw error
        }
        guard item.status == .readyToPlay else { throw NativePlaybackFixtures.Failure.invalid("AVPlayer not ready: \(item.status.rawValue)") }
        let advanced = expectation(description: "AVPlayer media clock advanced beyond 350ms")
        let decoded = video ? expectation(description: "AVPlayer decoded a 160x90 frame") : nil
        let time = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 30), queue: .main) { time in
            if time.seconds >= 0.35 && !observedTime { observedTime = true; advanced.fulfill() }
            if video && !observedFrame, let frame = output.copyPixelBuffer(forItemTime: time, itemTimeForDisplay: nil) {
                XCTAssertEqual(CVPixelBufferGetWidth(frame), 160); XCTAssertEqual(CVPixelBufferGetHeight(frame), 90)
                observedFrame = true; decoded?.fulfill()
            }
        }
        defer { player.removeTimeObserver(time) }
        player.play()
        await fulfillment(of: [advanced] + (decoded.map { [$0] } ?? []), timeout: 10)
        if let error = item.error { throw error }
        XCTAssertGreaterThan(player.currentTime().seconds, 0.35)
    }
    private func availableBytes() -> Int64 {
        let values = try? FileManager.default.attributesOfFileSystem(forPath: NSTemporaryDirectory())
        return (values?[.systemFreeSize] as? NSNumber)?.int64Value ?? -1
    }
}
