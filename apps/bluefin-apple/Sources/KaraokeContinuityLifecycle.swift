import Foundation
import Combine

/// The platform adapter owns the audio session and releases it even after a partial start.
@MainActor protocol KaraokeContinuityAudio: AnyObject {
    func requestPermission() async -> Bool
    func start(gain: Float) throws
    func setGain(_ gain: Float)
    func stop() throws
}

@MainActor final class KaraokeContinuityLifecycle: ObservableObject {
    enum State: Equatable { case off, selecting, connectedMuted, requestingPermission, monitoring, denied, interrupted, failed }
    @Published private(set) var state: State = .off
    @Published private(set) var status = "Microphone off"
    @Published private(set) var gain: Float = 0.1
    @Published private(set) var isMuted = true
    private let audio: KaraokeContinuityAudio
    private var generation = 0
    init(audio: KaraokeContinuityAudio) { self.audio = audio }

    /// Connection tokens also reject a late picker callback after account change or dismissal.
    func beginSelection() -> Int {
        stop()
        guard state == .off else { return generation }
        state = .selecting; status = "Choose an iPhone microphone"
        return generation
    }
    func selected(token: Int) -> Bool {
        guard token == generation, state == .selecting else { return false }
        state = .connectedMuted; status = "iPhone connected · Microphone off"
        return true
    }
    func cancelSelection(token: Int) {
        guard token == generation, state == .selecting else { return }
        stop()
    }
    func enable() async {
        guard state == .connectedMuted else { return }
        let request = generation
        state = .requestingPermission; status = "Waiting for microphone permission"
        let allowed = await audio.requestPermission()
        guard request == generation else { return }
        guard !Task.isCancelled else { stop(); return }
        guard allowed else { stop(state: .denied, reason: "Microphone access denied. Allow it in Settings, then reconnect."); return }
        do {
            try audio.start(gain: gain)
            guard request == generation, !Task.isCancelled else { stop(); return }
            isMuted = false; state = .monitoring; status = "iPhone microphone live"
        } catch {
            stop(state: .failed, reason: "Monitoring unavailable. Check the selected input and TV audio route, then reconnect.")
        }
    }
    func setGain(_ value: Float) {
        gain = value.isFinite ? min(max(value, 0), 0.25) : 0
        if state == .monitoring { audio.setGain(isMuted ? 0 : gain) }
    }
    func setMuted(_ muted: Bool) {
        guard state == .monitoring else { return }
        isMuted = muted
        audio.setGain(muted ? 0 : gain)
        status = muted ? "iPhone microphone muted" : "iPhone microphone live"
    }
    func stop(state next: State = .off, reason: String = "Microphone off") {
        generation += 1
        isMuted = true; state = next; status = reason
        do { try audio.stop() }
        catch { state = .failed; status = "Microphone stopped. Audio session restoration failed; restart playback." }
    }
    func interrupted() { stop(state: .interrupted, reason: "Audio changed or disconnected. Reconnect to enable monitoring again.") }
}
