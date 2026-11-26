import SwiftUI
#if os(tvOS)
import AVKit
import AVFoundation

@available(tvOS 17.0, *)
@MainActor final class KaraokeContinuityMicrophone: ObservableObject {
    let audio: KaraokeContinuityAudioAdapter
    let lifecycle: KaraokeContinuityLifecycle
    init() {
        let audio = KaraokeContinuityAudioAdapter()
        self.audio = audio; lifecycle = KaraokeContinuityLifecycle(audio: audio)
        audio.onInvalidated = { [weak lifecycle] in lifecycle?.interrupted() }
    }
    func stop() { lifecycle.stop() }
}

/// Parent owns this model so account/item/playback changes can stop it immediately.
@available(tvOS 17.0, *)
@MainActor struct KaraokeContinuityControls: View {
    let microphone: KaraokeContinuityMicrophone
    @ObservedObject private var lifecycle: KaraokeContinuityLifecycle
    @State private var picker = false
    @State private var token = 0
    @Environment(\.scenePhase) private var scenePhase
    init(microphone: KaraokeContinuityMicrophone) {
        self.microphone = microphone
        _lifecycle = ObservedObject(wrappedValue: microphone.lifecycle)
    }
    var body: some View {
        VStack(alignment: .leading) {
            Text(lifecycle.status).accessibilityLabel("Microphone: \(lifecycle.status)")
            if AVContinuityDevicePickerViewController.isSupported {
                Button("Connect iPhone microphone") {
                    guard ["NSMicrophoneUsageDescription", "NSCameraUsageDescription"].allSatisfy({ key in
                        guard let value = Bundle.main.object(forInfoDictionaryKey: key) as? String else { return false }
                        return !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    }) else {
                        lifecycle.stop(state: .failed, reason: "This build is missing Continuity privacy descriptions.")
                        return
                    }
                    token = lifecycle.beginSelection()
                    picker = lifecycle.state == .selecting
                }
                .disabled(picker)
                if lifecycle.state == .connectedMuted {
                    Button("Enable local voice monitoring") { Task { await lifecycle.enable() } }
                    Text("Start at low TV volume. Stop if you hear feedback.")
                }
                if lifecycle.state == .monitoring {
                    // tvOS uses focusable buttons rather than an unavailable Slider.
                    HStack {
                        Button("Voice quieter") { lifecycle.setGain(lifecycle.gain - 0.025) }
                        Button("Voice louder") { lifecycle.setGain(lifecycle.gain + 0.025) }
                    }
                }
                Button("Stop microphone") { picker = false; microphone.stop() }
            } else { Text("This Apple TV does not support Continuity microphones.") }
        }
        .continuityDevicePicker(isPresented: $picker) { device in
            guard let device else { lifecycle.cancelSelection(token: token); return }
            guard lifecycle.selected(token: token) else { return }
            microphone.audio.select(device)
            if !device.isConnected { lifecycle.interrupted() }
        }
        .onChange(of: scenePhase) { phase in
            if phase != .active { picker = false; microphone.stop() }
        }
        .onDisappear { picker = false; microphone.stop() }
    }
}
#endif
