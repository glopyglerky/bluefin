import AVFoundation
import Combine
import ObjectiveC

/// Two tracks in one composition share the native AVPlayer clock, including seeks and buffering.
/// No center cancellation, MusicKit extraction, network fetch, or claimed source separation.
@MainActor final class KaraokeAudio: ObservableObject {
    private static var resourceKey: UInt8 = 0
    let package: KaraokePackage
    let nativeItem: AVPlayerItem
    let duration: Double
    @Published private(set) var vocalLevel: Float = 1
    func retainResources(_ owner: AnyObject) {
        objc_setAssociatedObject(nativeItem, &Self.resourceKey, owner, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    }
    private let backing: AVCompositionTrack
    private let vocal: AVCompositionTrack

    private init(package: KaraokePackage, composition: AVMutableComposition,
                 backing: AVCompositionTrack, vocal: AVCompositionTrack, duration: Double) {
        self.package = package; self.backing = backing; self.vocal = vocal; self.duration = duration
        nativeItem = AVPlayerItem(asset: composition)
        setVocalLevel(1)
    }
    static func prepare(_ package: KaraokePackage) async throws -> KaraokeAudio {
        guard !package.rightsReference.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              package.processing != .modelSeparated || !package.processingReference.isEmpty else { throw KaraokeError.unauthorized }
        let urls = [package.accompaniment, package.leadVocal]
        guard urls.allSatisfy({ $0.isFileURL && ["wav", "aif", "aiff", "caf"].contains($0.pathExtension.lowercased()) }),
              urls[0].standardizedFileURL != urls[1].standardizedFileURL else { throw KaraokeError.invalidStems }
        // PCM decoding also rejects DRM and establishes exact sample alignment. Admission is bounded
        // to stereo PCM, one continuous audio track per file, at most four hours.
        let files = try urls.map { try AVAudioFile(forReading: $0) }
        let a = files[0], b = files[1]
        guard a.length > 0, a.length == b.length, a.processingFormat.sampleRate == b.processingFormat.sampleRate,
              (8_000...192_000).contains(a.processingFormat.sampleRate),
              a.processingFormat.channelCount == b.processingFormat.channelCount,
              (1...2).contains(a.processingFormat.channelCount),
              a.fileFormat.streamDescription.pointee.mFormatID == kAudioFormatLinearPCM, b.fileFormat.streamDescription.pointee.mFormatID == kAudioFormatLinearPCM else { throw KaraokeError.invalidStems }
        let duration = Double(a.length) / a.processingFormat.sampleRate
        // Bind the checked manifest to the decoded PCM, allowing one sample for
        // representation rounding. Catalog/container duration is checked separately.
        guard duration <= 14_400, package.durationSeconds.isFinite,
              package.durationSeconds > 0, package.durationSeconds <= 14_400,
              abs(duration - package.durationSeconds) <= 1 / a.processingFormat.sampleRate + duration.ulp * 4 else {
            throw KaraokeError.invalidStems
        }
        try KaraokeTimingKind.validate(package.timingKind, timeline: package.timeline, duration: duration)
        let composition = AVMutableComposition()
        var inserted: [AVMutableCompositionTrack] = []
        for url in urls {
            try Task.checkCancellation()
            let asset = AVURLAsset(url: url)
            let protected = try await asset.load(.hasProtectedContent)
            let tracks = try await asset.loadTracks(withMediaType: .audio)
            guard !protected, tracks.count == 1, let source = tracks.first,
                  let target = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid) else { throw KaraokeError.invalidStems }
            let range = try await source.load(.timeRange)
            guard abs(range.start.seconds) < 0.00001, abs(range.duration.seconds - duration) < 0.001 else { throw KaraokeError.invalidStems }
            try target.insertTimeRange(range, of: source, at: .zero)
            inserted.append(target)
        }
        return KaraokeAudio(package: package, composition: composition, backing: inserted[0], vocal: inserted[1], duration: duration)
    }
    func setVocalLevel(_ value: Float) {
        vocalLevel = value.isFinite ? min(1, max(0, value)) : 1
        nativeItem.audioMix = makeMix(level: vocalLevel)
    }
    func makeMix(level: Float) -> AVAudioMix {
        let backingParameters = AVMutableAudioMixInputParameters(track: backing)
        let vocalParameters = AVMutableAudioMixInputParameters(track: vocal)
        // Fixed headroom, no gain compensation when muting the singer.
        backingParameters.setVolume(0.35, at: .zero)
        vocalParameters.setVolume(0.35 * min(1, max(0, level.isFinite ? level : 1)), at: .zero)
        let mix = AVMutableAudioMix(); mix.inputParameters = [backingParameters, vocalParameters]
        return mix
    }
    var qualityDescription: String {
        package.processing == .suppliedStems
            ? "Supplied PCM stems · shared playback clock · reserved mix headroom"
            : "Model-separated stems · quality depends on the admitted model and source"
    }
}
