import AVFoundation
import Combine

/// Monitoring only. Never writes, uploads, or retains microphone samples.
@MainActor final class KaraokeMicrophone: ObservableObject {
    @Published private(set) var isMonitoring = false
    @Published private(set) var status = "Microphone off"
    #if os(iOS)
    private var engine: AVAudioEngine?
    private var observers: [NSObjectProtocol] = []
    private var generation = 0
    private var requestingPermission = false
    private var ownsSession = false
    private var previous: (AVAudioSession.Category, AVAudioSession.Mode, AVAudioSession.CategoryOptions)?

    init() {
        for name in [AVAudioSession.routeChangeNotification, AVAudioSession.interruptionNotification,
                     AVAudioSession.mediaServicesWereResetNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                // Synchronously silence the engine on any route/interruption notification before UI work.
                MainActor.assumeIsolated { self?.stop(reason: "Audio route changed. Enable monitoring again with wired headphones.") }
            })
        }
    }
    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
        engine?.mainMixerNode.outputVolume = 0
        engine?.stop()
        if ownsSession, let previous {
            try? AVAudioSession.sharedInstance().setCategory(previous.0, mode: previous.1, options: previous.2)
        }
    }
    private var safeRoute: Bool {
        let outputs = AVAudioSession.sharedInstance().currentRoute.outputs
        return !outputs.isEmpty && outputs.allSatisfy { $0.portType == .headphones }
    }
    func start() async {
        guard !isMonitoring, !requestingPermission, !Task.isCancelled else { return }
        guard let purpose = Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") as? String,
              !purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            status = "Microphone permission description is missing from this build."; return
        }
        guard safeRoute else { status = "Connect wired headphones to prevent feedback. Speaker, USB and wireless monitoring are disabled."; return }
        generation += 1; let request = generation
        requestingPermission = true
        let allowed = await withCheckedContinuation { continuation in
            AVAudioSession.sharedInstance().requestRecordPermission { continuation.resume(returning: $0) }
        }
        guard request == generation else { return }
        requestingPermission = false
        guard !Task.isCancelled else { stop(); return }
        guard allowed else { status = "Microphone access was denied. You can allow it in Settings."; return }
        let session = AVAudioSession.sharedInstance()
        guard safeRoute else { status = "Connect wired headphones before monitoring."; return }
        previous = (session.category, session.mode, session.categoryOptions)
        ownsSession = true
        do {
            try session.setCategory(.playAndRecord, mode: .default, options: [])
            try session.setActive(true)
            guard safeRoute, session.isInputAvailable else { throw KaraokeError.unavailable }
            let audio = AVAudioEngine()
            let input = audio.inputNode
            let format = input.outputFormat(forBus: 0)
            guard format.channelCount > 0, format.sampleRate > 0 else { throw KaraokeError.unavailable }
            audio.connect(input, to: audio.mainMixerNode, format: format)
            // Conservative gain, wired output only; no acoustic feedback cancellation claim.
            audio.mainMixerNode.outputVolume = 0.1
            engine = audio
            audio.prepare(); try audio.start()
            guard request == generation, safeRoute else { stop(); return }
            isMonitoring = true; status = "Monitoring locally through wired headphones"
        } catch { stop(reason: "Microphone monitoring could not start with this audio route.") }
    }
    func stop(reason: String = "Microphone off") {
        generation += 1
        requestingPermission = false
        engine?.mainMixerNode.outputVolume = 0
        engine?.stop(); engine = nil
        isMonitoring = false; status = reason
        if ownsSession, let previous {
            do { try AVAudioSession.sharedInstance().setCategory(previous.0, mode: previous.1, options: previous.2) }
            catch { status = "Monitoring stopped. The previous audio session could not be restored." }
        }
        previous = nil; ownsSession = false
    }
    #else
    func start() async { status = "Use the iPhone microphone control on supported Apple TV with tvOS 17+. iPhone and iPad monitoring requires wired headphones." }
    func stop(reason: String = "Microphone off") { isMonitoring = false; status = reason }
    #endif
}
