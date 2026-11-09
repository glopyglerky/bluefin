import AVFoundation
import SwiftUI

enum AudioQualitySummary {
    static let lossless = Set(["alac", "flac", "pcm_s16le", "pcm_s24le", "pcm_s32le", "pcm_s16be", "pcm_s24be", "pcm_s32be"])
    static func label(stream: MediaStream?, method: String) -> String {
        if method != "DirectPlay" { return "Converted audio" }
        guard let codec = stream?.Codec?.lowercased() else { return "Audio format" }
        if lossless.contains(codec) { return "Lossless" }
        switch codec { case "eac3": return "Dolby Digital Plus"; case "ac3": return "Dolby Digital"; default: return codec.uppercased() }
    }
    static func format(_ stream: MediaStream?) -> String {
        [stream?.BitDepth.map { "\($0)-bit" }, stream?.SampleRate.map { String(format: "%g kHz", Double($0) / 1000) }].compactMap { $0 }.joined(separator: " / ")
    }
}

struct AudioQualityView: View {
    @ObservedObject var playback: PlaybackController
    @Environment(\.dismiss) private var dismiss
    @State private var output = ""
    private var stream: MediaStream? { playback.source?.MediaStreams?.first { $0.Type == "Audio" } }
    var body: some View {
        BluefinNavigation {
            Form {
                Section("Source") {
                    LabeledContentCompat("Codec", value: stream?.Codec?.uppercased() ?? "Not supplied")
                    if !AudioQualitySummary.format(stream).isEmpty { Text(AudioQualitySummary.format(stream)) }
                    if let channels = stream?.Channels { LabeledContentCompat("Channels", value: String(channels)) }
                    if let profile = stream?.Profile, !profile.isEmpty { Text(profile) }
                }
                Section("Playback") {
                    Text(playback.playMethod == "DirectPlay" ? "Playing the original source" : "Server conversion is active")
                    LabeledContentCompat("Audio processing", value: playback.audioProcessingStatus)
                }
                Section("Device output") {
                    Text(output)
                    Text("The source format and device output can differ. Wireless codecs, conversion, audio processing and the selected output device affect the final signal.").font(.footnote).foregroundStyle(.secondary)
                }
            }.navigationTitle("Audio quality")
                .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { dismiss() } } }
                .onAppear { refresh() }
                .onReceive(NotificationCenter.default.publisher(for: AVAudioSession.routeChangeNotification)) { _ in refresh() }
        }
    }
    private func refresh() {
        let session = AVAudioSession.sharedInstance()
        output = session.currentRoute.outputs.map(\.portName).joined(separator: ", ")
            + String(format: " · %g kHz · %d channels", session.sampleRate / 1000, session.outputNumberOfChannels)
    }
}
