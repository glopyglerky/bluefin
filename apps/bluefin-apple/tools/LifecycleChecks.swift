import Foundation

actor Events {
    var values: [String] = []
    var waiters: [String: [CheckedContinuation<Void, Never>]] = [:]
    var progress: CheckedContinuation<Void, Never>?
    var failStopped = false
    func record(_ value: String) async throws {
        values.append(value)
        waiters.removeValue(forKey: value)?.forEach { $0.resume() }
        if value == "progress" { await withCheckedContinuation { progress = $0 } }
        if value == "stopped" && failStopped { throw BluefinError.message("fixture failure") }
    }
    func wait(_ value: String) async {
        if values.contains(value) { return }
        await withCheckedContinuation { waiters[value, default: []].append($0) }
    }
    func release() { progress?.resume(); progress = nil }
    func snapshot() -> [String] { values }
    func setFailure() { failStopped = true }
}

actor PlanGate {
    var continuation: CheckedContinuation<PlaybackPlan, Never>?
    var value: PlaybackPlan?
    func wait() async -> PlaybackPlan {
        if let value { return value }
        return await withCheckedContinuation { continuation = $0 }
    }
    func supply(_ plan: PlaybackPlan) { value = plan; continuation?.resume(returning: plan); continuation = nil }
}

@main struct LifecycleRegression {
    @MainActor static func main() async throws {
        let session = AccountSession(server: "https://example.com", token: "fixture", userId: "u", username: "Fixture", deviceId: "d", allowHttp: false)
        let plan = PlaybackPlan(url: "http://127.0.0.1:1234/cap", playSessionId: "p", mediaSourceId: "m", playMethod: "Transcode", source: .init(LiveStreamId: "live"), proxyId: "cap")
        let events = Events()
        let reporter = PlaybackReporter(session: session, itemId: "item", plan: plan) { operation, _, _ in try await events.record(operation) }
        try await reporter.report("started", ticks: 0, paused: false)
        let progress = Task { try await reporter.report("progress", ticks: 100, paused: false) }
        await events.wait("progress")
        let logout = Task {
            try await reporter.finish(ticks: 120, wasStarted: true)
            try await events.record("logout")
        }
        await events.wait("stopPlaybackProxy")
        let blocked = await events.snapshot()
        precondition(!blocked.contains("logout") && !blocked.contains("releasePlayback"))
        await events.release()
        try await progress.value
        try await logout.value
        try await reporter.finish(ticks: 120, wasStarted: true)
        let order = await events.snapshot()
        precondition(order == ["started", "progress", "stopPlaybackProxy", "stopped", "releasePlayback", "logout"], "Teardown or logout order failed: \(order)")

        let failing = Events()
        await failing.setFailure()
        let failedReporter = PlaybackReporter(session: session, itemId: "item", plan: plan) { operation, _, _ in try await failing.record(operation) }
        do { try await failedReporter.finish(ticks: 5, wasStarted: true); preconditionFailure("Expected reporting failure") } catch {}
        let failedOrder = await failing.snapshot()
        precondition(failedOrder == ["stopPlaybackProxy", "started", "stopped", "releasePlayback"])

        let gate = PlanGate(), discarded = Events()
        let pending = PendingPlayback(request: { await gate.wait() }, cleanup: { _ in try await discarded.record("cleanup") })
        let resolution = Task { try await pending.resolve() }
        pending.cancel()
        let cleanupOne = Task { try await pending.dispose() }
        let cleanupTwo = Task { try await pending.dispose() }
        await gate.supply(plan)
        try await cleanupOne.value; try await cleanupTwo.value
        do { _ = try await resolution.value; preconditionFailure("Cancelled plan was delivered") } catch is CancellationError {}
        let discardEvents = await discarded.snapshot()
        precondition(discardEvents == ["cleanup"], "Plan cleanup must happen exactly once")

        let transferred = Events()
        let accepted = PendingPlayback(request: { plan }, cleanup: { _ in try await transferred.record("cleanup") })
        _ = try await accepted.resolve(); try accepted.transfer(); try await accepted.dispose()
        let transferredEvents = await transferred.snapshot()
        precondition(transferredEvents.isEmpty, "Transferred plan was cleaned by its predecessor")
        print("PASS: ordered reporting/release/logout; cleanup after reporting error; cancelled plan cleanup exactly once; transfer ownership")
    }
}
