import Foundation

struct KaraokePreparationManifest: Decodable {
    struct Stem: Decodable { let sha256: String; let bytes: UInt64 }
    let itemId: String
    let revision: String
    let durationSeconds: Double
    let processing: String
    let processingReference: String?
    let mediaSourceId: String?
    let accompaniment: Stem
    let leadVocal: Stem
    let timeline: KaraokeTimeline?
    let timingKind: KaraokeTimingKind?
    var processingKind: KaraokePackage.Processing { processing == "modelSeparatedStems" ? .modelSeparated : .suppliedStems }
    func validate(itemID: String, mediaSourceID: String? = nil) throws {
        guard KaraokeIdentity.sameItem(itemId, itemID), ["suppliedStems", "modelSeparatedStems"].contains(processing),
              KaraokeIdentity.digest(revision),
              durationSeconds.isFinite, durationSeconds > 0, durationSeconds <= 14_400 else { throw KaraokeError.unauthorized }
        if processing == "modelSeparatedStems" {
            guard let processingReference, !processingReference.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  processingReference.utf8.count <= 256, processingReference.rangeOfCharacter(from: .controlCharacters) == nil,
                  let mediaSourceId, KaraokeIdentity.validSource(mediaSourceId) else { throw KaraokeError.unauthorized }
        }
        if let mediaSourceId, !KaraokeIdentity.validSource(mediaSourceId) { throw KaraokeError.unauthorized }
        if let mediaSourceID {
            guard let mediaSourceId, KaraokeIdentity.sameSource(mediaSourceID, mediaSourceId) else {
                throw KaraokeError.unauthorized
            }
        }
        for stem in [accompaniment, leadVocal] {
            guard stem.bytes > 44, stem.bytes <= 64 * 1024 * 1024, KaraokeIdentity.digest(stem.sha256) else { throw KaraokeError.invalidStems }
        }
        try KaraokeTimingKind.validate(timingKind, timeline: timeline, duration: durationSeconds)
    }
}

enum KaraokeIdentity {
    static func sameItem(_ a: String, _ b: String) -> Bool { a.replacingOccurrences(of: "-", with: "").lowercased() == b.replacingOccurrences(of: "-", with: "").lowercased() }
    static func sameSource(_ a: String, _ b: String) -> Bool {
        if a == b { return true }
        let x = a.replacingOccurrences(of: "-", with: ""), y = b.replacingOccurrences(of: "-", with: "")
        return x.utf8.count == 32 && y.utf8.count == 32 && x.utf8.allSatisfy(hex) && y.utf8.allSatisfy(hex) && x.lowercased() == y.lowercased()
    }
    static func validSource(_ id: String) -> Bool { !id.isEmpty && id.utf8.count <= 1024 && id.rangeOfCharacter(from: .controlCharacters) == nil }
    static func digest(_ value: String) -> Bool { value.utf8.count == 64 && value.utf8.allSatisfy(hex) }
    private static func hex(_ b: UInt8) -> Bool { (48...57).contains(b) || (65...70).contains(b) || (97...102).contains(b) }
}

struct KaraokePreparationStatus: Decodable {
    enum State: String, Decodable { case queued, preparing, ready, unavailable, failed, cancelling, cancelled }
    let itemId: String
    let jobId: String
    let mediaSourceId: String?
    let state: State
    let retryAfterMilliseconds: Int?
    let reason: String?
    func validate(itemID: String, requestID: String, sourceID: String?) throws {
        guard KaraokeIdentity.sameItem(itemId, itemID), let job = UUID(uuidString: jobId), job == UUID(uuidString: requestID),
              (500...5000).contains(retryAfterMilliseconds ?? 1000) else { throw KaraokeError.unauthorized }
        if let mediaSourceId {
            guard KaraokeIdentity.validSource(mediaSourceId) else { throw KaraokeError.unauthorized }
            if let sourceID, !KaraokeIdentity.sameSource(sourceID, mediaSourceId) { throw KaraokeError.unauthorized }
        } else if state == .ready { throw KaraokeError.unauthorized }
    }
    var failureMessage: String {
        switch reason {
        case "timingRequired": return "Word-timed lyrics haven't been added for this track."
        case "durationUnsupported": return "Your server cannot prepare Sing for a track this long."
        case "unsupportedSource": return "This source isn't available for Sing preparation."
        case "serverDisabled": return "Sing preparation isn't enabled on your server."
        case "busy": return "Your server is preparing another track. Try again shortly."
        default: return "Your server couldn't prepare this track for Sing."
        }
    }
}

/// The associated lease outlives views and preparation tasks. AVPlayerItem retains
/// it until the composition and all consumers release the item after replacement.
final class KaraokePreparationLease {
    let directory: URL
    init() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("bluefin-karaoke-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        do {
            var values = URLResourceValues(); values.isExcludedFromBackup = true
            var url = directory; try url.setResourceValues(values)
        } catch {
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
    }
    deinit { try? FileManager.default.removeItem(at: directory) }
}

