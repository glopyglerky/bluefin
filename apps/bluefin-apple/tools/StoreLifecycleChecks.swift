import Foundation
import CryptoKit

// Host adapters keep this check off the network, Keychain and AVFoundation.
// LibraryStore and PlaybackReporter below are the actual production sources.
actor StoreEvents {
    var operations: [String] = []
    var waiters: [String: [CheckedContinuation<Void, Never>]] = [:]
    var progress: CheckedContinuation<Void, Never>?
    var failRelease = false
    func record(_ operation: String, session: AccountSession, fields: [String: Any]) async throws {
        precondition(session.token == "old-session", "Cleanup used a different account")
        if operation == "releasePlayback" {
            precondition(fields["liveStreamId"] as? String == "live")
            precondition(fields["playSessionId"] as? String == "play")
        }
        if operation == "stopped" { precondition(fields["positionTicks"] as? Int64 == 120) }
        operations.append(operation)
        waiters.removeValue(forKey: operation)?.forEach { $0.resume() }
        if operation == "progress" { await withCheckedContinuation { progress = $0 } }
        if operation == "releasePlayback" && failRelease { throw BluefinError.message("fixture release failure") }
    }
    func wait(_ operation: String) async {
        if operations.contains(operation) { return }
        await withCheckedContinuation { waiters[operation, default: []].append($0) }
    }
    func resumeProgress() { progress?.resume(); progress = nil }
    func snapshot() -> [String] { operations }
    func failCleanup() { failRelease = true }
}

@MainActor enum SessionVault {
    static var clearCount = 0
    static var current: AccountSession?
    static func load() -> AccountSession? { current }
    static func save(_ session: AccountSession) throws { current = session }
    static func clear() { clearCount += 1; current = nil }
}

struct EmptyValue: Decodable {}

@MainActor enum RustBridge {
    static var events = StoreEvents()
    static var allowPlayback = false
    static func call<T: Decodable>(_ operation: String, session: AccountSession? = nil, fields: [String: Any] = [:]) async throws -> T {
        if allowPlayback && operation == "playback" {
            return try JSONDecoder().decode(T.self, from: Data(#"{"url":"https://example.com/fixture","playSessionId":"play","mediaSourceId":"media","playMethod":"DirectPlay","source":{"LiveStreamId":"live"}}"#.utf8))
        }
        throw BluefinError.message("Unexpected fixture request: \(operation)")
    }
    static func notify(_ operation: String, session: AccountSession, fields: [String: Any] = [:]) async throws {
        try await events.record(operation, session: session, fields: fields)
    }
    static func cleanupPlaybackPlan(_ plan: PlaybackPlan, session: AccountSession) async throws {
        throw BluefinError.message("Unexpected preparation cleanup")
    }
}

struct NativeAudioSettings {}
@MainActor final class AudioPreferences { var settings = NativeAudioSettings() }

@MainActor final class SportsStore { func disconnect() {} }

@MainActor final class OfflineLibrary {
    func disconnect() {}
    func localURL(for item: MediaItem, session: AccountSession) -> URL? { nil }
}

@MainActor final class PlaybackController {
    let id = UUID()
    var onMusicAdvance: ((MediaItem) -> Void)?
    var videoSelection = VideoOptionSelection()
    var onVideoOptions: ((VideoOptionSelection) -> Void)?
    func configureAudio(_ settings: NativeAudioSettings) {}
    func discardNextMusicCandidate() -> Task<Void, Error> { Task {} }
    func setNextMusicCandidate(_ item: MediaItem?, session: AccountSession) {}
    let item: MediaItem
    var positionSeconds: Double = 0
    var isFavorite = false
    var onReplay: (() -> Void)?
    var onFavorite: (() -> Void)?
    func seek(seconds: Double) { positionSeconds = seconds }
    func updateFavorite(_ value: Bool) { isFavorite = value }
    var onSeekOutsideWindow: ((Int64) -> Void)?
    var onNext: (() -> Void)?
    var onPrevious: (() -> Void)?
    var onFinish: (() -> Void)?
    private let reporter: PlaybackReporter
    private var teardown: Task<Void, Error>?
    init(item: MediaItem, url: URL, plan: PlaybackPlan?, session: AccountSession?) throws {
        self.item = item
        guard let plan, let session else { throw BluefinError.message("Missing fixture plan") }
        reporter = PlaybackReporter(session: session, itemId: item.Id, plan: plan) { operation, session, fields in
            try await RustBridge.notify(operation, session: session, fields: fields)
        }
    }
    func holdProgress() async throws { try await reporter.report("progress", ticks: 100, paused: false) }
    func stop() -> Task<Void, Error> {
        if let teardown { return teardown }
        let reporter = reporter
        let task = Task { try await reporter.finish(ticks: 120, wasStarted: true) }
        teardown = task
        return task
    }
}

@main struct StoreLifecycleRegression {
    @MainActor static func main() async throws {
        let account = AccountSession(server: "https://example.com", token: "old-session", userId: "user", username: "Fixture", deviceId: "device", allowHttp: false)
        let plan = PlaybackPlan(url: "http://127.0.0.1:1234/cap", playSessionId: "play", mediaSourceId: "media", playMethod: "Transcode", source: .init(LiveStreamId: "live"), proxyId: "cap")
        let store = LibraryStore()
        store.session = account
        let player = try PlaybackController(item: .preview[0], url: URL(string: plan.url)!, plan: plan, session: account)
        store.playback = player
        let events = RustBridge.events
        let progress = Task { try await player.holdProgress() }
        await events.wait("progress")
        store.dismissPlayback()
        precondition(store.playback == nil)
        let logout = Task { await store.signOut() }
        while SessionVault.clearCount == 0 { await Task.yield() }
        await events.wait("stopPlaybackProxy")
        let blocked = await events.snapshot()
        if blocked.contains("logout") {
            print("FAIL: dismissal lost teardown ownership before sign-out: \(blocked)")
            fflush(stdout)
            throw BluefinError.message("Logout overtook playback cleanup")
        }
        await events.resumeProgress()
        try await progress.value
        await logout.value
        let order = await events.snapshot()
        if order != ["started", "progress", "stopPlaybackProxy", "stopped", "releasePlayback", "logout"] {
            print("FAIL: wrong store teardown order: \(order)")
            fflush(stdout)
            throw BluefinError.message("Wrong store teardown order")
        }

        let failedEvents = StoreEvents()
        await failedEvents.failCleanup()
        RustBridge.events = failedEvents
        store.session = account
        let failedPlayer = try PlaybackController(item: .preview[0], url: URL(string: plan.url)!, plan: plan, session: account)
        store.playback = failedPlayer
        store.dismissPlayback()
        _ = await failedPlayer.stop().result
        for _ in 0..<1000 where store.error == nil { await Task.yield() }
        precondition(store.error != nil, "Dismissed player lost its cleanup failure")
        await store.signOut()
        precondition(store.error?.contains("could not confirm") == true, "Sign-out lost an earlier cleanup failure")
        let failedOrder = await failedEvents.snapshot()
        precondition(failedOrder == ["stopPlaybackProxy", "started", "stopped", "releasePlayback", "logout"])
        RustBridge.events = StoreEvents()
        RustBridge.allowPlayback = true
        SessionVault.current = account
        let video = LibraryStore()
        await video.play(.preview[0])
        let ended = video.playback
        precondition(ended?.onFinish != nil, "Video completion has no presentation owner")
        ended?.onFinish?()
        precondition(video.playback == nil, "Completed video left an empty player onscreen")
        await video.signOut()
        SessionVault.current = account
        let queued = LibraryStore()
        queued.clearMusicQueue()
        let a = MediaItem(Id: "audio-a", Name: "A", Type: "Audio")
        let b = MediaItem(Id: "audio-b", Name: "B", Type: "Audio")
        await queued.playMusic([a, a, b], at: 1)
        precondition(queued.musicIndex == 1, "Duplicate playlist entries lost the requested cursor")
        queued.removeMusic(at: IndexSet(integer: 0))
        precondition(queued.musicIndex == 0 && queued.musicQueue.count == 2)
        queued.moveMusic(from: IndexSet(integer: 0), to: 2)
        precondition(queued.musicIndex == 1 && queued.musicQueue.map(\.Id) == [b.Id, a.Id])
        queued.cycleMusicRepeat()
        queued.cycleMusicRepeat()
        precondition(queued.musicRepeat == .one)
        queued.toggleMusicShuffle()
        queued.toggleMusicShuffle()
        precondition(queued.musicQueue.map(\.Id) == [b.Id, a.Id])
        let restored = LibraryStore()
        precondition(restored.musicQueue.map(\.Id) == [b.Id, a.Id] && restored.musicRepeat == .one)
        var other = account; other.userId = "different-fixture-user"
        SessionVault.current = other
        let isolated = LibraryStore()
        precondition(isolated.musicQueue.isEmpty, "Queue leaked between accounts")
        SessionVault.current = nil
        let key = "bluefin.queue." + SHA256.hash(data: Data((account.server + "\u{0}" + account.userId).utf8)).map { String(format: "%02x", $0) }.joined()
        UserDefaults.standard.removeObject(forKey: key)
        let preview = LibraryStore(); preview.explore()
        let favorites = try await preview.musicItems(type: "MusicAlbum", favorites: true)
        precondition(favorites.Items.isEmpty, "Preview falsely labeled an unfavorited album")
        print("PASS: completed video dismisses through owned teardown; preview favorites are filtered")
        print("PASS: duplicate queue cursor, queue remove/move, shuffle restoration, repeat and account-scoped persistence")
        print("PASS: actual LibraryStore dismiss/sign-out retains ordered cleanup, captured account and release fields; dismissed failures remain visible")
    }
}
