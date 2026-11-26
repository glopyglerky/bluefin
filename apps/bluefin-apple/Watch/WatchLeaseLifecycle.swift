import Foundation

/// Every cleanup attempt retains the exact account and lease that admitted it.
@MainActor final class WatchLeaseLifecycle {
    struct Owner {
        let id = UUID()
        let account: WatchAccount
        let releasePath: String
    }
    private(set) var owner: Owner?
    private var pending: (id: UUID, task: Task<Void, Error>)?
    var isCleaning: Bool { pending != nil }

    func install(account: WatchAccount, releasePath: String) throws {
        guard owner == nil && pending == nil else {
            throw WatchFailure.message("Previous playback cleanup must succeed first.")
        }
        owner = Owner(account: account, releasePath: releasePath)
    }

    func drain(release: @escaping (Owner) async throws -> Void,
               finished: @escaping () -> Void) async throws {
        let captured: (id: UUID, task: Task<Void, Error>)
        if let pending {
            captured = pending
        } else if let owner {
            let task = Task { @MainActor in
                try await release(owner)
                guard self.owner?.id == owner.id else { return }
                finished()
                self.owner = nil
            }
            // A retry still owns the same lease, but has a different attempt ID.
            // A late waiter from a failed attempt must not clear this retry.
            captured = (UUID(), task)
            pending = captured
        } else {
            finished()
            return
        }
        defer {
            // Another waiter may already have admitted a replacement. Its gate
            // belongs to a different UUID and must survive this completion.
            if pending?.id == captured.id { pending = nil }
        }
        try await captured.task.value
    }
}
