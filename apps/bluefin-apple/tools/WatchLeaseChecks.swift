import Foundation

enum FixtureError: Error { case denied }
actor ReleaseGate {
    var attempts = 0
    var waiting: [Int: CheckedContinuation<Void, Error>] = [:]
    func release() async throws {
        attempts += 1
        let attempt = attempts
        try await withCheckedThrowingContinuation { waiting[attempt] = $0 }
    }
    func count() -> Int { attempts }
    func finish(_ attempt: Int, fail: Bool) {
        if fail { waiting.removeValue(forKey: attempt)?.resume(throwing: FixtureError.denied) }
        else { waiting.removeValue(forKey: attempt)?.resume() }
    }
}
@main enum WatchLeaseRetryChecks {
    @MainActor static func main() async throws {
        for iteration in 0..<100 {
            let lifecycle = WatchLeaseLifecycle()
            let account = WatchAccount(server: URL(string: "https://example.invalid")!, token: "fixture", user: "fixture", name: "Fixture")
            try lifecycle.install(account: account, releasePath: "fixture")
            let gate = ReleaseGate()
            let first = Task(priority: .high) { @MainActor in try await lifecycle.drain(release: { _ in try await gate.release() }, finished: {}) }
            while await gate.count() < 1 { await Task.yield() }
            var secondEntered = false
            let second = Task(priority: .background) { @MainActor in
                secondEntered = true
                try await lifecycle.drain(release: { _ in try await gate.release() }, finished: {})
            }
            while !secondEntered { await Task.yield() }
            await gate.finish(1, fail: true)
            _ = await first.result
            let retry = Task(priority: .high) { @MainActor in try await lifecycle.drain(release: { _ in try await gate.release() }, finished: {}) }
            while await gate.count() < 2 { await Task.yield() }
            _ = await second.result
            guard lifecycle.isCleaning else {
                await gate.finish(2, fail: false)
                _ = await retry.result
                print("FAIL: old failed waiter cleared an active retry at iteration \(iteration)")
                exit(1)
            }
            await gate.finish(2, fail: false)
            try await retry.value
            guard lifecycle.owner == nil && !lifecycle.isCleaning else { exit(1) }
        }
        print("PASS: failed joined cleanup cannot clear a later retry; 100 schedules")
    }
}
