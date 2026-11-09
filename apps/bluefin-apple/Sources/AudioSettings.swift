import SwiftUI

@MainActor final class AudioPreferences: ObservableObject {
    @Published var settings: NativeAudioSettings {
        didSet {
            UserDefaults.standard.set(settings.eqEnabled, forKey: "bluefin.audio.eqEnabled")
            UserDefaults.standard.set(settings.gainsDB, forKey: "bluefin.audio.bands")
            UserDefaults.standard.set(mode, forKey: "bluefin.audio.normalization")
        }
    }
    init() {
        let defaults = UserDefaults.standard
        var value = NativeAudioSettings()
        value.eqEnabled = defaults.bool(forKey: "bluefin.audio.eqEnabled")
        if let bands = defaults.array(forKey: "bluefin.audio.bands") as? [Double], bands.count == 6 {
            value.gainsDB = bands.map { NativeAudioSettings.bounded($0, limit: 12) }
        }
        switch defaults.string(forKey: "bluefin.audio.normalization") {
        case "track": value.normalization = .track
        case "album": value.normalization = .album
        default: break
        }
        settings = value
    }
    var mode: String {
        get { settings.normalization == .track ? "track" : settings.normalization == .album ? "album" : "off" }
        set { settings.normalization = newValue == "track" ? .track : newValue == "album" ? .album : .off }
    }
}

struct AudioSettingsView: View {
    @EnvironmentObject private var store: LibraryStore
    @ObservedObject var preferences: AudioPreferences
    var body: some View {
        Form {
            Section {
                Picker("Volume normalization", selection: Binding(get: { preferences.mode }, set: { preferences.mode = $0 })) {
                    Text("Off").tag("off")
                    Text("Track gain").tag("track")
                    Text("Album gain").tag("album")
                }
            } footer: { Text("Uses loudness metadata from your server. Tracks without gain metadata keep their original level.") }
            Section {
                Toggle("Equalizer", isOn: Binding(get: { preferences.settings.eqEnabled }, set: { preferences.settings.eqEnabled = $0 }))
                ForEach(0..<6, id: \.self) { index in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Text(frequency(index)).font(.subheadline)
                            Spacer()
                            Text(String(format: "%+.0f dB", preferences.settings.gainsDB[index])).font(.subheadline.monospacedDigit()).foregroundStyle(.secondary)
                        }
                        #if os(iOS)
                        Slider(value: Binding(get: { preferences.settings.gainsDB[index] }, set: { preferences.settings.gainsDB[index] = $0 }), in: -12...12, step: 1)
                            .accessibilityLabel(frequency(index)).disabled(!preferences.settings.eqEnabled)
                        #else
                        HStack(spacing: 24) {
                            Button { preferences.settings.gainsDB[index] -= 1 } label: { Image(systemName: "minus").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Reduce \(frequency(index))").disabled(!preferences.settings.eqEnabled || preferences.settings.gainsDB[index] <= -12)
                            Button { preferences.settings.gainsDB[index] += 1 } label: { Image(systemName: "plus").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Increase \(frequency(index))").disabled(!preferences.settings.eqEnabled || preferences.settings.gainsDB[index] >= 12)
                        }
                        #endif
                    }.padding(.vertical, 4)
                }
                Button("Reset to flat") { preferences.settings.gainsDB = Array(repeating: 0, count: 6) }
            } header: { Text("Equalizer") } footer: {
                Text("Flat and disabled settings bypass processing. HLS, protected streams, unsupported decoded formats and external playback use their original audio path.")
            }
            if let playback = store.playback { Section("Current playback") { AudioProcessingStatus(playback: playback) } }
        }.navigationTitle("Audio")
        .onReceive(preferences.$settings) { store.playback?.configureAudio($0) }
    }
    private func frequency(_ index: Int) -> String {
        let hz = NativeAudioSettings.frequencies[index]
        return hz >= 1000 ? String(format: "%.1f kHz", hz / 1000) : "\(Int(hz)) Hz"
    }
}

private struct AudioProcessingStatus: View {
    @ObservedObject var playback: PlaybackController
    var body: some View { Text(playback.audioProcessingStatus).font(.footnote).foregroundStyle(.secondary) }
}
