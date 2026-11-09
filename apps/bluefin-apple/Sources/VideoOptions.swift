import AVFoundation
import VideoToolbox
import SwiftUI

/// Observations, not a promise that a particular asset/profile/container will play.
struct VideoDeviceCapabilities: Codable, Equatable {
    let schemaVersion: Int
    let hevcHardwareDecode: Bool
    let av1HardwareDecode: Bool
    let nativeProbeSupported: Bool
    let hdrPlaybackEligible: Bool
    let hdrModes: [String]
    let dolbyVisionProfiles: [Int]
    let losslessAtmos: Bool
    let maximumWidth: Int?
    let maximumHeight: Int?

    static let conservative = Self(schemaVersion: 1, hevcHardwareDecode: false, av1HardwareDecode: false, nativeProbeSupported: false,
        hdrPlaybackEligible: false, hdrModes: [], dolbyVisionProfiles: [],
        losslessAtmos: false, maximumWidth: nil, maximumHeight: nil)

    @MainActor static func current() -> Self {
        #if os(iOS) || os(tvOS)
        let eligible = AVPlayer.eligibleForHDRPlayback
        let modes = AVPlayer.availableHDRModes
        let hevc = VTIsHardwareDecodeSupported(kCMVideoCodecType_HEVC)
        let av1: Bool
        if #available(iOS 17, tvOS 17, *) { av1 = VTIsHardwareDecodeSupported(kCMVideoCodecType_AV1) }
        else { av1 = false }
        var names: [String] = []
        if eligible {
            if modes.contains(.hlg) { names.append("hlg") }
            if modes.contains(.hdr10) { names.append("hdr10") }
            if modes.contains(.dolbyVision) { names.append("dolby_vision") }
        }
        return Self(schemaVersion: 1,
            hevcHardwareDecode: hevc, av1HardwareDecode: av1, nativeProbeSupported: true,
            hdrPlaybackEligible: eligible, hdrModes: names,
            dolbyVisionProfiles: names.contains("dolby_vision") ? (av1 ? [5, 8, 10] : [5, 8]) : [],
            losslessAtmos: false, maximumWidth: hevc || av1 ? 3840 : 1920, maximumHeight: hevc || av1 ? 2160 : 1080)
        #else
        return conservative
        #endif
    }

    /// JSON object uses the exact same keys and values as Codable.
    func jsonDictionary() throws -> [String: Any] {
        let data = try JSONEncoder().encode(self)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw CocoaError(.coderInvalidValue)
        }
        return object
    }
}

/// Parent supplies resolved data and effects. AVPlayerViewController retains transport/PiP/AirPlay.
/// Parent must replace selection only after a successful renegotiation, and show its own errors.
struct VideoOptionsView: View {
    let chapters: [MediaChapter]
    let source: MediaSource?
    let selection: VideoOptionSelection
    let isRenegotiating: Bool
    let supportedRates: [Float]
    let presentationRate: Float
    let nextEpisodeTitle: String?
    let onSeek: (Double) -> Void
    let onRenegotiate: (VideoOptionSelection) -> Void
    let onPresentationRate: (Float) -> Void
    let onNextEpisode: () -> Void

    private var streams: [MediaStream] { source?.MediaStreams ?? [] }
    private func streams(of type: String) -> [MediaStream] {
        var seen = Set<Int>()
        return streams.filter {
            $0.Type?.lowercased() == type && ($0.Index.map { $0 >= 0 && seen.insert($0).inserted } ?? false)
        }
    }
    private func title(_ stream: MediaStream) -> String {
        let parts = [stream.DisplayTitle, stream.Title, stream.Language, stream.Codec]
        return parts.compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first(where: { !$0.isEmpty }) ?? "Track \((stream.Index ?? 0) + 1)"
    }
    private func request(audio: Int? = nil, subtitle: Int? = nil, quality: VideoQualityPreference? = nil,
                         changeAudio: Bool = false, changeSubtitle: Bool = false) {
        var updated = selection
        updated.mediaSourceID = source?.Id
        if changeAudio { updated.audioStreamIndex = audio }
        if changeSubtitle { updated.subtitleStreamIndex = subtitle }
        if let quality { updated.quality = quality }
        if updated != selection { onRenegotiate(updated) }
    }
    private func option(_ title: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack {
                Text(title).fixedSize(horizontal: false, vertical: true)
                Spacer()
                if selected { Image(systemName: "checkmark").accessibilityLabel("Selected") }
            }.frame(minHeight: 44)
        }
    }

    var body: some View {
        List {
            if !chapters.isEmpty {
                Section("Chapters") {
                    ForEach(Array(chapters.enumerated()), id: \.offset) { index, chapter in
                        if chapter.StartPositionTicks >= 0 {
                            Button { onSeek(Double(chapter.StartPositionTicks) / 10_000_000) } label: {
                                Text(chapter.Name?.isEmpty == false ? (chapter.Name ?? "") : "Chapter \(index + 1)")
                                    .frame(minHeight: 44)
                            }
                        }
                    }
                }
            }
            Section("Quality") {
                ForEach(VideoQualityPreference.allCases) { quality in
                    option(quality.title, selected: selection.quality == quality) { request(quality: quality) }
                }
            }.disabled(isRenegotiating)
            if !streams(of: "audio").isEmpty {
                Section("Audio") {
                    option("Automatic", selected: selection.audioStreamIndex == nil) { request(changeAudio: true) }
                    ForEach(streams(of: "audio"), id: \.Index) { stream in
                        option(title(stream), selected: selection.audioStreamIndex == stream.Index) {
                            request(audio: stream.Index, changeAudio: true)
                        }
                    }
                }.disabled(isRenegotiating || source?.Id == nil)
            }
            if !streams(of: "subtitle").isEmpty {
                Section("Subtitles") {
                    option("Off", selected: selection.subtitleStreamIndex == nil) { request(changeSubtitle: true) }
                    ForEach(streams(of: "subtitle"), id: \.Index) { stream in
                        option(title(stream), selected: selection.subtitleStreamIndex == stream.Index) {
                            request(subtitle: stream.Index, changeSubtitle: true)
                        }
                    }
                }.disabled(isRenegotiating || source?.Id == nil)
            }
            if !supportedRates.isEmpty {
                Section("Playback speed") {
                    ForEach(Array(Set(supportedRates.filter { $0.isFinite && $0 > 0 })).sorted(), id: \.self) { rate in
                        option(String(format: "%g×", rate), selected: rate == presentationRate) { onPresentationRate(rate) }
                    }
                }
            }
            if let title = nextEpisodeTitle, !title.isEmpty {
                Section("Up next") {
                    Button { onNextEpisode() } label: {
                        Label(title, systemImage: "forward.end.fill").frame(minHeight: 44)
                    }
                }
            }
        }
        .tint(BluefinStyle.accent)
        .navigationTitle("Video options")
    }
}
