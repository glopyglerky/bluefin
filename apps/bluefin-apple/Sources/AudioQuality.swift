import AVFoundation
import SwiftUI

struct AudioQualityView: View {
    @ObservedObject var playback: PlaybackController
    @Environment(\.dismiss) private var dismiss
    @State private var output = ""
    private var stream: MediaStream? { playback.audioSourceStream }
    var body: some View {
        BluefinNavigation {
            Form {
                Section("Source") {
                    if let title = stream?.DisplayTitle ?? stream?.Title { Text(title) }
                    if stream == nil { Text("The selected source track has not been identified.").foregroundStyle(.secondary) }
                    LabeledContentCompat("Codec", value: stream?.Codec?.uppercased() ?? "Not supplied")
                    if !AudioQualitySummary.format(stream).isEmpty { Text(AudioQualitySummary.format(stream)) }
                    if let channels = stream?.Channels, channels > 0 { LabeledContentCompat("Channels", value: String(channels)) }
                    if let profile = stream?.Profile, !profile.isEmpty { Text(profile) }
                }
                Section("Playback") {
                    Text(playback.karaoke != nil ? "Playing Sing stems" : playback.playMethod == "DirectPlay" ? "Playing the original source" : "Server conversion is active")
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
        let ports = session.currentRoute.outputs.map(\.portName)
        guard !ports.isEmpty else { output = "No audio output is active"; return }
        let rate = session.sampleRate > 0 ? String(format: "%g kHz", session.sampleRate / 1000) : "Sample rate unavailable"
        let channels = session.outputNumberOfChannels > 0 ? "\(session.outputNumberOfChannels) channels" : "Channel count unavailable"
        output = ports.joined(separator: ", ") + " · \(rate) · \(channels)"
    }
}
