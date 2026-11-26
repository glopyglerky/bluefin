#if os(tvOS)
import AVFoundation
import AVKit
import UIKit

/// No taps, file sinks, network transports, or video capture sessions.
@available(tvOS 17.0, *)
@MainActor final class KaraokeContinuityAudioAdapter: KaraokeContinuityAudio {
    private(set) var device: AVContinuityDevice?
    private var connectionObservation: NSKeyValueObservation?
    private var inputObservation: NSKeyValueObservation?
    private var notifications: [NSObjectProtocol] = []
    private var resource: KaraokeContinuitySession?
    private var configuring = false
    var onInvalidated: (() -> Void)?

    init() {
        for name in [AVAudioSession.routeChangeNotification, AVAudioSession.interruptionNotification,
                     AVAudioSession.mediaServicesWereResetNotification, AVAudioSession.mediaServicesWereLostNotification,
                     UIApplication.didEnterBackgroundNotification, Notification.Name.AVAudioEngineConfigurationChange] {
            notifications.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] note in
                MainActor.assumeIsolated {
                    guard let self, self.device != nil else { return }
                    if note.name == AVAudioSession.routeChangeNotification {
                        // Selection can make inputs available while still muted. Do not auto-start.
                        guard self.resource != nil else { return }
                        // Synchronous setup notifications are checked by start's final route guards.
                        if self.configuring { return }
                        let session = AVAudioSession.sharedInstance()
                        let previous = note.userInfo?[AVAudioSessionRouteChangePreviousRouteKey] as? AVAudioSessionRouteDescription
                        let sameOutput = previous?.outputs.map(\.uid) == session.currentRoute.outputs.map(\.uid)
                        if sameOutput, session.category == .playAndRecord, session.mode == .voiceChat,
                           self.resource?.engine.isRunning == true, self.selectedRouteIsValid(session) { return }
                    }
                    if note.name == Notification.Name.AVAudioEngineConfigurationChange {
                        guard let engine = note.object as? AVAudioEngine,
                              engine === self.resource?.engine, !engine.isRunning else { return }
                    }
                    self.onInvalidated?()
                }
            })
        }
    }
    deinit { notifications.forEach(NotificationCenter.default.removeObserver) }
    func select(_ selected: AVContinuityDevice) {
        device = selected
        connectionObservation = selected.observe(\.isConnected, options: [.new]) { [weak self] _, _ in
            Task { @MainActor in
                guard let self, self.device?.isConnected == false else { return }
                self.onInvalidated?()
            }
        }
        inputObservation = AVAudioSession.sharedInstance().observe(\.isInputAvailable, options: [.new]) { [weak self] _, _ in
            Task { @MainActor in
                guard let self, self.resource != nil, !AVAudioSession.sharedInstance().isInputAvailable else { return }
                self.onInvalidated?()
            }
        }
    }
    func requestPermission() async -> Bool {
        guard let purpose = Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") as? String,
              !purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                AVCaptureDevice.requestAccess(for: .audio) { continuation.resume(returning: $0) }
            }
        default: return false
        }
    }
    func start(gain: Float) throws {
        guard let device, device.isConnected else { throw Failure.route }
        configuring = true
        defer { configuring = false }
        let session = AVAudioSession.sharedInstance()
        let owned = KaraokeContinuitySession(session: session)
        resource = owned // Own rollback before the first throwing mutation.
        try session.setCategory(.playAndRecord, mode: .voiceChat, options: [])
        try session.setActive(true)
        let selectedIDs = Set(device.audioSessionInputs.map(\.uid))
        guard let input = session.availableInputs?.first(where: { selectedIDs.contains($0.uid) }) else { throw Failure.route }
        try session.setPreferredInput(input)
        guard session.isInputAvailable, session.currentRoute.inputs.contains(where: { $0.uid == input.uid }),
              safeOutputs(session) else { throw Failure.route }
        let engine = owned.engine
        try engine.inputNode.setVoiceProcessingEnabled(true)
        engine.inputNode.isVoiceProcessingBypassed = false
        // No unprocessed speaker fallback. System processing is a mitigation, not a feedback guarantee.
        guard engine.inputNode.isVoiceProcessingEnabled else { throw Failure.route }
        let format = engine.inputNode.outputFormat(forBus: 0)
        guard format.channelCount > 0, format.sampleRate > 0 else { throw Failure.route }
        engine.connect(engine.inputNode, to: engine.mainMixerNode, format: format)
        engine.mainMixerNode.outputVolume = 0
        engine.prepare()
        guard resource === owned else { throw Failure.route }
        try engine.start()
        guard resource === owned, device.isConnected, safeOutputs(session),
              session.currentRoute.inputs.contains(where: { $0.uid == input.uid }) else { throw Failure.route }
        engine.mainMixerNode.outputVolume = gain
    }
    private func selectedRouteIsValid(_ session: AVAudioSession) -> Bool {
        guard let device, device.isConnected, session.isInputAvailable else { return false }
        let ids = Set(device.audioSessionInputs.map(\.uid))
        return session.currentRoute.inputs.contains { ids.contains($0.uid) } && safeOutputs(session)
    }
    private func safeOutputs(_ session: AVAudioSession) -> Bool {
        let outputs = session.currentRoute.outputs
        // Reject AirPlay/Bluetooth/unknown routes with unqualified delay and echo behavior.
        return !outputs.isEmpty && outputs.allSatisfy { [.HDMI, .lineOut, .headphones, .builtInSpeaker].contains($0.portType) }
    }
    func setGain(_ gain: Float) { resource?.engine.mainMixerNode.outputVolume = gain }
    func stop() throws {
        connectionObservation = nil; inputObservation = nil; device = nil
        let owned = resource; resource = nil
        try owned?.restore()
    }
    private enum Failure: Error { case route }
}

/// RAII also silences capture if the view/model is released without its explicit stop hook.
@available(tvOS 17.0, *)
private final class KaraokeContinuitySession {
    let engine = AVAudioEngine()
    private let session: AVAudioSession
    private let category: AVAudioSession.Category
    private let mode: AVAudioSession.Mode
    private let options: AVAudioSession.CategoryOptions
    private let input: AVAudioSessionPortDescription?
    private var restored = false
    init(session: AVAudioSession) {
        self.session = session; category = session.category; mode = session.mode
        options = session.categoryOptions; input = session.preferredInput
    }
    func restore() throws {
        guard !restored else { return }
        restored = true
        engine.mainMixerNode.outputVolume = 0; engine.stop(); engine.reset()
        // Keep the shared playback session active. AVAudioSession exposes no previous-active getter.
        // Parent owns activation/deactivation for playback and must stop this lane before reconfiguration.
        var failure: Error?
        do { try session.setCategory(category, mode: mode, options: options) } catch { failure = error }
        do { try session.setPreferredInput(input) } catch { if failure == nil { failure = error } }
        if let failure { throw failure }
    }
    deinit { try? restore() }
}
#endif
