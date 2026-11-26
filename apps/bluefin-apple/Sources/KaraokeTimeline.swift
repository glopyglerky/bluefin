import Foundation

/// Tokens may be words or syllables. Include spaces/punctuation in text; never infer timing.
struct KaraokeToken: Codable, Equatable {
    let text: String
    let start: Double
    let end: Double
    func progress(at seconds: Double) -> Double {
        guard seconds.isFinite, end > start else { return 0 }
        return min(1, max(0, (seconds - start) / (end - start)))
    }
}

struct KaraokeLine: Codable, Equatable, Identifiable {
    enum Voice: String, Codable { case lead, partner, together, backing }
    let id: String
    let voice: Voice
    let tokens: [KaraokeToken]
    var start: Double { tokens.first?.start ?? 0 }
    var end: Double { tokens.last?.end ?? 0 }
    var text: String { tokens.map(\.text).joined() }
}

struct KaraokeTimeline: Codable, Equatable {
    /// Positive offsets delay lyric presentation relative to audio.
    let offsetSeconds: Double
    let lines: [KaraokeLine]

    func validate(duration: Double) throws {
        guard duration.isFinite, duration > 0, offsetSeconds.isFinite,
              !lines.isEmpty, lines.count <= 10_000,
              Set(lines.map(\.id)).count == lines.count else { throw KaraokeError.invalidTiming }
        var previousStart = -Double.infinity
        for line in lines {
            guard !line.id.isEmpty, !line.tokens.isEmpty, line.tokens.count <= 1_000,
                  line.start >= previousStart else { throw KaraokeError.invalidTiming }
            previousStart = line.start
            var end = -Double.infinity
            for token in line.tokens {
                guard !token.text.isEmpty, token.text.utf8.count <= 4_096,
                      token.start.isFinite, token.end.isFinite, token.start >= 0,
                      token.end > token.start, token.start >= end,
                      token.end + offsetSeconds <= duration + 0.05 else { throw KaraokeError.invalidTiming }
                end = token.end
            }
        }
    }
    func activeLines(at seconds: Double) -> [KaraokeLine] {
        guard seconds.isFinite else { return [] }
        let time = seconds - offsetSeconds
        return lines.filter { $0.start <= time && time < $0.end }
    }
}

enum KaraokeError: LocalizedError {
    case invalidTiming, unauthorized, invalidStems, unavailable, stale
    var errorDescription: String? {
        switch self {
        case .invalidTiming: return "Word timing is missing or invalid for this track."
        case .unauthorized: return "Authorized stems for this account and track are required."
        case .invalidStems: return "Stems must be local, unprotected, aligned lossless audio with matching duration and format."
        case .unavailable: return "Karaoke is available for local music playback at normal speed."
        case .stale: return "Playback changed while karaoke was preparing."
        }
    }
}

/// Supplied by the authenticated owner after rights admission and integrity checks.
/// Never persist account credentials here. Files remain owned by the caller until playback teardown.
struct KaraokePackage {
    enum Processing: String { case suppliedStems, modelSeparated }
    let itemID: String
    let accountUserID: String
    let accountServer: String
    let accompaniment: URL
    let leadVocal: URL
    let rightsReference: String
    let processing: Processing
    /// Public server processing reference; private model/license/authority custody stays server-side.
    let processingReference: String
    /// Duration of the admitted PCM stems, not optional catalog/container metadata.
    let durationSeconds: Double
    let timeline: KaraokeTimeline
}
