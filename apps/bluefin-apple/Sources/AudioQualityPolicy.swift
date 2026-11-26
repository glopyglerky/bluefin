import Foundation

/// Source metadata describes the encoded track, not the final device signal.
enum AudioQualitySummary {
    static let lossless = Set(["alac", "flac", "pcm_u8", "pcm_s8", "pcm_s16le", "pcm_s24le", "pcm_s32le", "pcm_s16be", "pcm_s24be", "pcm_s32be", "pcm_f32le", "pcm_f32be", "pcm_f64le", "pcm_f64be"])

    static func selectedStream(source: MediaSource?, index: Int?) -> MediaStream? {
        let audio = source?.MediaStreams?.filter { $0.Type == "Audio" } ?? []
        let selected = index.flatMap { $0 >= 0 ? $0 : nil }
            ?? source?.DefaultAudioStreamIndex.flatMap { $0 >= 0 ? $0 : nil }
        if let selected {
            let matches = audio.filter { $0.Index == selected }
            return matches.count == 1 ? matches[0] : nil
        }
        // Multiple tracks with no admitted/default index are ambiguous. Never
        // substitute the first track's lossless badge for an unknown selection.
        return audio.count == 1 ? audio[0] : nil
    }

    static func label(stream: MediaStream?, method: String) -> String {
        if method != "DirectPlay" { return "Converted audio" }
        guard let codec = stream?.Codec?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased(), !codec.isEmpty else { return "Audio format unavailable" }
        if lossless.contains(codec) {
            return (stream?.BitDepth ?? 0) >= 24 && (stream?.SampleRate ?? 0) > 48_000
                ? "Hi-Res Lossless" : "Lossless"
        }
        switch codec { case "eac3": return "Dolby Digital Plus"; case "ac3": return "Dolby Digital"; default: return codec.uppercased() }
    }

    static func badge(stream: MediaStream?, method: String, singing: Bool) -> String {
        if singing { return "Sing · Processed audio" }
        let label = label(stream: stream, method: method)
        return method == "DirectPlay" ? "Source · \(label)" : label
    }

    static func format(_ stream: MediaStream?) -> String {
        [stream?.BitDepth.flatMap { $0 > 0 ? "\($0)-bit" : nil },
         stream?.SampleRate.flatMap { $0 > 0 ? String(format: "%g kHz", Double($0) / 1000) : nil }]
            .compactMap { $0 }.joined(separator: " / ")
    }
}
