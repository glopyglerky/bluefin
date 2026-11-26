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
        VStack(alignment: .leading, spacing: 12) {
            Label(lifecycle.status, systemImage: lifecycle.state == .monitoring && !lifecycle.isMuted ? "mic.fill" : "mic.slash.fill")
                .font(.headline).foregroundStyle(lifecycle.state == .monitoring && !lifecycle.isMuted ? BluefinStyle.accent : .secondary)
                .accessibilityLabel("Microphone: \(lifecycle.status)")
            if AVContinuityDevicePickerViewController.isSupported {
                if lifecycle.state != .connectedMuted && lifecycle.state != .requestingPermission && lifecycle.state != .monitoring {
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
                }
                if lifecycle.state == .connectedMuted {
                    Button("Start microphone") { Task { await lifecycle.enable() } }
                    Text("Your voice plays through the TV. Start with a low TV volume.").font(.caption).foregroundStyle(.secondary)
                }
                if lifecycle.state == .monitoring {
                    // tvOS uses focusable buttons rather than an unavailable Slider.
                    HStack {
                        Button(lifecycle.isMuted ? "Unmute" : "Mute") { lifecycle.setMuted(!lifecycle.isMuted) }
                        Button("Voice quieter") { lifecycle.setGain(lifecycle.gain - 0.025) }
                        Button("Voice louder") { lifecycle.setGain(lifecycle.gain + 0.025) }
                    }
                }
                if lifecycle.state == .requestingPermission { ProgressView("Waiting for microphone permission") }
                if lifecycle.state == .connectedMuted || lifecycle.state == .requestingPermission || lifecycle.state == .monitoring {
                    Button("Disconnect microphone") { picker = false; microphone.stop() }
                }
            } else { Text("This Apple TV does not support Continuity microphones.") }
        }
        .continuityDevicePicker(isPresented: $picker) { device in
            guard let device else { lifecycle.cancelSelection(token: token); return }
            guard lifecycle.selected(token: token) else { return }
            microphone.audio.select(device)
            if !device.isConnected { lifecycle.interrupted() }
        }
        .onChange(of: scenePhase) { phase in
            if phase == .background || (phase == .inactive && lifecycle.state == .monitoring) { picker = false; microphone.stop() }
        }
        .onDisappear { if !picker { microphone.stop() } }
    }
}
#endif
