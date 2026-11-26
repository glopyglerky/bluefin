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
    static var failDeletion = false
    static func load() -> AccountSession? { current }
    static func save(_ session: AccountSession) throws { current = session }
    static func clear() throws {
        clearCount += 1
        guard !failDeletion else { throw BluefinError.message("Your saved sign-in could not be removed. Try signing out again.") }
        current = nil
    }
}

struct EmptyValue: Decodable {}

@MainActor enum RustBridge {
    static var events = StoreEvents()
    static var allowPlayback = false
    static var candidatePlans: [String] = []
    static var candidateRequests: [[String: Any]] = []
    static var preparationCleanups: [String] = []
    static func call<T: Decodable>(_ operation: String, session: AccountSession? = nil, fields: [String: Any] = [:]) async throws -> T {
        if operation == "playback", !candidatePlans.isEmpty {
            precondition(session?.token == "old-session")
            if !candidateRequests.isEmpty { precondition(preparationCleanups == ["rejected-candidate"]) }
            candidateRequests.append(fields)
            return try JSONDecoder().decode(T.self, from: Data(candidatePlans.removeFirst().utf8))
        }
        if allowPlayback && operation == "playback" {
            return try JSONDecoder().decode(T.self, from: Data(#"{"url":"https://example.com/fixture","playSessionId":"play","mediaSourceId":"media","playMethod":"DirectPlay","source":{"LiveStreamId":"live"}}"#.utf8))
        }
        throw BluefinError.message("Unexpected fixture request: \(operation)")
    }
    static func notify(_ operation: String, session: AccountSession, fields: [String: Any] = [:]) async throws {
        try await events.record(operation, session: session, fields: fields)
    }
    static func cleanupPlaybackPlan(_ plan: PlaybackPlan, session: AccountSession) async throws {
        precondition(session.token == "old-session")
        guard let id = plan.proxyId else { throw BluefinError.message("Missing candidate cleanup identity") }
        preparationCleanups.append(id)
    }
}

struct NativeAudioSettings {}
@MainActor final class AudioPreferences { var settings = NativeAudioSettings() }

struct VideoDeviceCapabilities {
    var expanded = true
    static let conservative = Self(expanded: false)
    static func current() -> Self { Self() }
    func jsonDictionary() throws -> [String: Any] { ["nativeProbeSupported": expanded] }
}
@MainActor enum NativeAssetAdmission {
    static var onProbe: (() -> Void)?
    static func validate(_ url: URL) async throws {
        guard let onProbe else { throw BluefinError.message("Unexpected native asset probe in host fixture") }
        onProbe()
        throw BluefinError.message("Fixture asset is not decodable")
    }
}

@MainActor enum Artwork {
    static func clearCache() {}
    static func prefetch(items: [MediaItem], session: AccountSession) async {
        // No image transport is used by the lifecycle fixture. Production Home
        // still schedules and cancels this task through the real store.
    }
}

// This executable exercises ordinary HTTPS-account cleanup. Unexpected mesh
// calls fail immediately instead of silently replacing the account protocol.
struct MeshSavedSession {
    let sessionId: String
    let apiOrigin: String
    let userId: String
    let deviceId: String
}
struct HostMeshPending { struct Completion { let id: String }; let completion: Completion? }
@MainActor enum MeshPendingVault {
    static func load() throws -> HostMeshPending? { preconditionFailure("Unexpected mesh pending lookup") }
    static func clear() throws { preconditionFailure("Unexpected mesh pending mutation") }
}
@MainActor enum MeshRevocationVault {
    static func contains(_ reference: String) throws -> Bool { preconditionFailure("Unexpected mesh revocation lookup") }
}
@MainActor enum MeshSessionVault {
    static func save(_ session: MeshSavedSession) throws { preconditionFailure("Unexpected mesh session save") }
    static func clear(reference: String) throws { preconditionFailure("Unexpected mesh session clear") }
}
@MainActor final class MeshRevocationQueue {
    static let shared = MeshRevocationQueue()
    var count: Int { preconditionFailure("Unexpected mesh revocation count") }
    func prepare(_ session: AccountSession) async throws {
        precondition(session.meshSessionId == nil, "HTTPS lifecycle fixture entered mesh cleanup")
    }
    func retry() async { preconditionFailure("Unexpected mesh revocation retry") }
}
@MainActor final class DeviceMusicLibrary {
    func stopPlayback() -> Task<Void, Error>? { nil }
}

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
    func loadSkipSegments(for session: AccountSession) {}
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
    var didStop = false
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
        didStop = true
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
        if CommandLine.arguments.contains("--vault-failure") {
            SessionVault.current = account
            SessionVault.failDeletion = true
            await store.signOut()
            precondition(store.session?.token == account.token && SessionVault.current?.token == account.token)
            precondition(store.error?.contains("sign-out could not be saved") == true)
            let beforeRetry = await RustBridge.events.snapshot()
            precondition(beforeRetry.isEmpty, "A failed local sign-out invalidated the still-saved account")
            SessionVault.failDeletion = false
            await store.signOut()
            precondition(store.session == nil && SessionVault.current == nil && store.error == nil)
            let afterRetry = await RustBridge.events.snapshot()
            precondition(afterRetry == ["logout"], "Retry did not log out the captured account exactly once")
            print("PASS: failed Keychain deletion retains the saved account, reports a retry, and succeeds on retry")
            return
        }
        let player = try PlaybackController(item: .preview[0], url: URL(string: plan.url)!, plan: plan, session: account)
        store.playback = player
        let events = RustBridge.events
        let progress = Task { try await player.holdProgress() }
        await events.wait("progress")
        store.dismissPlayback()
        precondition(store.playback == nil)
        SessionVault.current = account
        let logout = Task { await store.signOut() }
        while !store.signingOut { await Task.yield() }
        await events.wait("stopPlaybackProxy")
        precondition(SessionVault.clearCount == 0 && SessionVault.current?.token == account.token,
                     "Sign-out discarded cleanup credentials before media teardown completed")
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
        precondition(store.error?.contains("remote cleanup is unconfirmed") == true, "Sign-out lost an earlier cleanup failure")
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

        RustBridge.events = StoreEvents()
        SessionVault.current = account
        let fallback = LibraryStore()
        let existing = try PlaybackController(item: .preview[0], url: URL(string: plan.url)!, plan: plan, session: account)
        fallback.playback = existing
        NativeAssetAdmission.onProbe = {
            precondition(fallback.playback === existing && !existing.didStop, "Probe stopped the existing player")
        }
        RustBridge.candidatePlans = [
            #"{"url":"http://127.0.0.1:1234/rejected","playSessionId":"play","mediaSourceId":"chosen-media","playMethod":"DirectPlay","proxyId":"rejected-candidate","nativeProbeRequired":true,"source":{"LiveStreamId":"live"}}"#,
            #"{"url":"http://127.0.0.1:1234/fallback","playSessionId":"play","mediaSourceId":"chosen-media","playMethod":"Transcode","proxyId":"accepted-candidate","nativeProbeRequired":false,"source":{"LiveStreamId":"live"}}"#
        ]
        await fallback.play(.preview[0], startAtTicks: 200_000,
                            videoOptions: .init(mediaSourceID: "chosen-media", audioStreamIndex: 2, subtitleStreamIndex: 3))
        precondition(fallback.playback != nil && fallback.playback !== existing && existing.didStop)
        precondition(RustBridge.candidateRequests.count == 2)
        for request in RustBridge.candidateRequests {
            precondition(request["itemId"] as? String == MediaItem.preview[0].Id)
            precondition(request["mediaSourceId"] as? String == "chosen-media")
            precondition(request["startTicks"] as? Int64 == 200_000)
            precondition(request["audioStreamIndex"] as? Int == 2 && request["subtitleStreamIndex"] as? Int == 3)
        }
        precondition((RustBridge.candidateRequests[0]["videoCapabilities"] as? [String: Any])?["nativeProbeSupported"] as? Bool == true)
        precondition((RustBridge.candidateRequests[1]["videoCapabilities"] as? [String: Any])?["nativeProbeSupported"] as? Bool == false)
        precondition(RustBridge.preparationCleanups == ["rejected-candidate"])
        await fallback.signOut()
        NativeAssetAdmission.onProbe = nil
        print("PASS: native admission fallback drains rejected candidate before retry, preserves existing playback, viewer/source/tracks/timeline, and retries once conservatively")
        print("PASS: completed video dismisses through owned teardown; preview favorites are filtered")
        print("PASS: duplicate queue cursor, queue remove/move, shuffle restoration, repeat and account-scoped persistence")
        print("PASS: actual LibraryStore dismiss/sign-out retains ordered cleanup, captured account and release fields; dismissed failures remain visible")
    }
}
