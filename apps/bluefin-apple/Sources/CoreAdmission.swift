import Foundation

/// Image work cannot occupy every ordinary request slot. Playback cleanup keeps
/// its existing separate route through the Rust bridge.
actor CoreAdmission {
    static let shared = CoreAdmission()
    private struct Waiter {
        let id: UUID
        let artwork: Bool
        let continuation: CheckedContinuation<UUID, Error>
    }
    private var active: [UUID: Bool] = [:]
    private var pending: [Waiter] = []

    func acquire(artwork: Bool = false) async throws -> UUID {
        let id = UUID()
        return try await withTaskCancellationHandler(operation: {
            try await withCheckedThrowingContinuation { continuation in
                if Task.isCancelled { continuation.resume(throwing: CancellationError()); return }
                if canAdmit(artwork: artwork) && (artwork ? !pending.contains(where: { !$0.artwork }) : true) {
                    active[id] = artwork
                    continuation.resume(returning: id)
                } else if pending.count < 64 {
                    pending.append(Waiter(id: id, artwork: artwork, continuation: continuation))
                } else {
                    continuation.resume(throwing: BluefinError.message("Your library is busy. Try again in a moment."))
                }
            }
        }, onCancel: { Task { await self.cancel(id) } })
    }

    private func canAdmit(artwork: Bool) -> Bool {
        active.count < 3 && (!artwork || active.values.filter { $0 }.count < 2)
    }
    private func cancel(_ id: UUID) {
        guard let index = pending.firstIndex(where: { $0.id == id }) else { return }
        pending.remove(at: index).continuation.resume(throwing: CancellationError())
        drain()
    }
    func release(_ id: UUID) {
        guard active.removeValue(forKey: id) != nil else { return }
        drain()
    }
    private func drain() {
        while active.count < 3 {
            let next = pending.firstIndex(where: { !$0.artwork && canAdmit(artwork: false) })
                ?? pending.firstIndex(where: { canAdmit(artwork: $0.artwork) })
            guard let index = next else { return }
            let request = pending.remove(at: index)
            active[request.id] = request.artwork
            request.continuation.resume(returning: request.id)
        }
    }
}
