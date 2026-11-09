import Foundation

/// Keeps a Rust network slot available for playback teardown. Cancelled searches
/// leave the queue immediately instead of consuming a later network request.
actor CoreAdmission {
    static let shared = CoreAdmission()
    private var active: Set<UUID> = []
    private var pending: [(UUID, CheckedContinuation<UUID, Error>)] = []
    func acquire() async throws -> UUID {
        let id = UUID()
        return try await withTaskCancellationHandler(operation: {
            try await withCheckedThrowingContinuation { continuation in
                if Task.isCancelled { continuation.resume(throwing: CancellationError()); return }
                if active.count < 3 { active.insert(id); continuation.resume(returning: id) }
                else if pending.count < 64 { pending.append((id, continuation)) }
                else { continuation.resume(throwing: BluefinError.message("Your library is busy. Try again in a moment.")) }
            }
        }, onCancel: { Task { await self.cancel(id) } })
    }
    private func cancel(_ id: UUID) {
        guard let index = pending.firstIndex(where: { $0.0 == id }) else { return }
        pending.remove(at: index).1.resume(throwing: CancellationError())
    }
    func release(_ id: UUID) {
        guard active.remove(id) != nil else { return }
        if !pending.isEmpty {
            let next = pending.removeFirst()
            active.insert(next.0)
            next.1.resume(returning: next.0)
        }
    }
}
