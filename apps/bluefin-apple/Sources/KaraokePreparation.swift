import Foundation
import AVFoundation

@MainActor enum KaraokePreparation {
    static func enable(playback: PlaybackController, session: AccountSession,
                       currentSession: () -> AccountSession?, status: (String) -> Void) async throws {
        let id = playback.item.Id
        let sourceID = playback.selectedMediaSourceID
        let requestID = UUID().uuidString
        var fields: [String: Any] = ["itemId": id]
        if let sourceID { fields["mediaSourceId"] = sourceID }
        func current() -> Bool {
            guard let now = currentSession() else { return false }
            return now.server == session.server && now.userId == session.userId && now.token == session.token
                && now.deviceId == session.deviceId && now.allowHttp == session.allowHttp
                && now.meshSessionId == session.meshSessionId
                && playback.item.Id == id && playback.selectedMediaSourceID == sourceID && !playback.isStopped
        }
        try Task.checkCancellation(); guard current() else { throw KaraokeError.stale }
        try await withTaskCancellationHandler(operation: {
        var serverMayBeWorking = false
        defer { if serverMayBeWorking { cancelRequest(session: session, itemID: id, requestID: requestID) } }
        status("Checking authorized stems")
        let manifest: KaraokePreparationManifest
        do { manifest = try await RustBridge.call("karaokeManifest", session: session, fields: fields) }
        catch {
            guard error.localizedDescription.contains("HTTP 404") else { throw error }
            try Task.checkCancellation(); guard current() else { throw KaraokeError.stale }
            var jobFields = fields; jobFields["requestId"] = requestID
            serverMayBeWorking = true
            var job: KaraokePreparationStatus = try await RustBridge.call("prepareKaraoke", session: session, fields: jobFields)
            let deadline = ProcessInfo.processInfo.systemUptime + 610
            preparation: while true {
                try Task.checkCancellation(); guard current() else { throw KaraokeError.stale }
                try job.validate(itemID: id, requestID: requestID, sourceID: sourceID)
                switch job.state {
                case .ready: serverMayBeWorking = false; break preparation
                case .unavailable, .failed:
                    serverMayBeWorking = false
                    throw BluefinError.message(job.failureMessage)
                case .cancelled:
                    serverMayBeWorking = false
                    throw CancellationError()
                case .cancelling: throw CancellationError()
                case .queued, .preparing:
                    status(job.state == .queued ? "Waiting for your server" : "Preparing vocals on your server")
                    guard ProcessInfo.processInfo.systemUptime < deadline else { throw BluefinError.message("Sing preparation took too long. Try again later.") }
                    try await Task.sleep(nanoseconds: UInt64(job.retryAfterMilliseconds ?? 1000) * 1_000_000)
                    try Task.checkCancellation(); guard current() else { throw KaraokeError.stale }
                    job = try await RustBridge.call("karaokePreparationStatus", session: session, fields: jobFields)
                }
            }
            manifest = try await RustBridge.call("karaokeManifest", session: session, fields: fields)
        }
        try Task.checkCancellation(); guard current() else { throw KaraokeError.stale }
        try manifest.validate(itemID: id, mediaSourceID: sourceID)
        let lease = try KaraokePreparationLease()
        let backing = lease.directory.appendingPathComponent("accompaniment.wav")
        let vocal = lease.directory.appendingPathComponent("lead-vocal.wav")
        for (role, descriptor, url) in [("accompaniment", manifest.accompaniment, backing), ("lead-vocal", manifest.leadVocal, vocal)] {
            status(role == "accompaniment" ? "Preparing accompaniment" : "Preparing lead vocal")
            var stemFields = fields
            stemFields.merge(["revision": manifest.revision, "role": role, "sha256": descriptor.sha256, "bytes": descriptor.bytes, "destination": url.path]) { _,new in new }
            let _: EmptyValue = try await RustBridge.call("karaokeStem", session: session, fields: stemFields)
            try Task.checkCancellation(); guard current() else { throw KaraokeError.stale }
        }
        status("Preparing synchronized playback")
        let package = KaraokePackage(itemID: id, accountUserID: session.userId, accountServer: session.server,
                                     accompaniment: backing, leadVocal: vocal,
                                     rightsReference: "Authenticated server admission", processing: manifest.processingKind,
                                     processingReference: manifest.processingReference ?? "",
                                     durationSeconds: manifest.durationSeconds, timeline: manifest.timeline)
        try await playback.enableKaraoke(package, retaining: lease, isCurrent: current)
        if Task.isCancelled || !current() { await playback.disableKaraoke(); throw KaraokeError.stale }
        }, onCancel: {
            // The known request UUID lets the server cancel even a delayed POST.
            cancelRequest(session: session, itemID: id, requestID: requestID)
        })
    }
    private nonisolated static func cancelRequest(session: AccountSession, itemID: String, requestID: String) {
        Task { try? await RustBridge.notify("cancelKaraokePreparation", session: session,
            fields: ["itemId": itemID, "requestId": requestID]) }
    }
}
