import Foundation

/// The same ordered lifecycle is used by the player and by host regression checks.
actor PlaybackReporter {
    typealias Request = (String, AccountSession, [String: Any]) async throws -> Void
    private let session: AccountSession
    private let itemId: String
    private let plan: PlaybackPlan
    private let request: Request
    private var started = false
    private var finished = false
    private var tail: Task<Void, Error>?
    private var teardown: Task<Void, Error>?
    init(session: AccountSession, itemId: String, plan: PlaybackPlan, request: @escaping Request) {
        self.session = session; self.itemId = itemId; self.plan = plan; self.request = request
    }
    func report(_ operation: String, ticks: Int64, paused: Bool) async throws {
        guard !finished else { return }
        let previous = tail
        let task = Task {
            if let previous { _ = await previous.result }
            try await send(operation, ticks: ticks, paused: paused)
        }
        tail = task
        try await task.value
    }
    private func send(_ operation: String, ticks: Int64, paused: Bool) async throws {
        var fields: [String: Any] = ["itemId": itemId, "mediaSourceId": plan.mediaSourceId, "playSessionId": plan.playSessionId, "positionTicks": ticks, "isPaused": paused, "playMethod": plan.playMethod ?? "DirectStream"]
        if let index = plan.audioStreamIndex { fields["audioStreamIndex"] = index }
        if let index = plan.subtitleStreamIndex { fields["subtitleStreamIndex"] = index }
        if !started { try await request("started", session, fields); started = true }
        if operation != "started" { try await request(operation, session, fields) }
    }
    func finish(ticks: Int64, wasStarted: Bool) async throws {
        if let teardown { try await teardown.value; return }
        finished = true
        let previous = tail
        let task = Task {
            var failure: Error?
            if let proxy = plan.proxyId {
                do { try await request("stopPlaybackProxy", session, ["proxyId": proxy]) }
                catch { failure = error }
            }
            if let previous { _ = await previous.result }
            if wasStarted {
                do { try await send("stopped", ticks: ticks, paused: true) }
                catch { failure = failure ?? error }
            }
            do { try await request("releasePlayback", session, PlaybackCleanup.fields(plan)) }
            catch { failure = failure ?? error }
            if let failure { throw failure }
        }
        teardown = task
        try await task.value
    }
}

enum PlaybackCleanup {
    static func fields(_ plan: PlaybackPlan) -> [String: Any] {
        var fields: [String: Any] = ["playSessionId": plan.playSessionId, "playMethod": plan.playMethod ?? "DirectPlay"]
        if let live = plan.source?.LiveStreamId { fields["liveStreamId"] = live }
        return fields
    }
}

/// Owns preparation until a controller takes it. Cancellation waits for the
/// returned plan and disposes it exactly once, including during sign-out.
@MainActor final class PendingPlayback {
    private let response: Task<PlaybackPlan, Error>
    private let cleanup: (PlaybackPlan) async throws -> Void
    private var plan: PlaybackPlan?
    private var cancelled = false
    private var transferred = false
    private var disposal: Task<Void, Error>?
    init(request: @escaping () async throws -> PlaybackPlan, cleanup: @escaping (PlaybackPlan) async throws -> Void) {
        response = Task { try await request() }
        self.cleanup = cleanup
    }
    func resolve() async throws -> PlaybackPlan {
        let value = try await response.value
        plan = value
        if cancelled || Task.isCancelled { cancelled = true; try await dispose(); throw CancellationError() }
        return value
    }
    func transfer() throws {
        guard !cancelled, plan != nil else { throw CancellationError() }
        transferred = true
    }
    func cancel() { cancelled = true }
    func dispose() async throws {
        guard !transferred else { return }
        if let disposal { try await disposal.value; return }
        let response = response, cleanup = cleanup
        let task = Task { try await cleanup(response.value) }
        disposal = task
        try await task.value
    }
}
