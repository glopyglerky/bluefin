import AVKit
import MediaPlayer
import SwiftUI

@MainActor final class PlaybackController: ObservableObject, Identifiable {
    let id = UUID()
    let item: MediaItem
    let player: AVPlayer
    @Published var failure: String?
    @Published var reportingFailure: String?
    @Published var positionSeconds: Double = 0
    @Published var isPlaying = false
    var onNext: (() -> Void)?
    var onPrevious: (() -> Void)?
    var onFinish: (() -> Void)?
    var onSeekOutsideWindow: ((Int64) -> Void)?
    let source: MediaSource?
    let playMethod: String
    private let reporter: PlaybackReporter?
    private let proxyId: String?
    private var timeObserver: Any?
    private var statusObserver: NSKeyValueObservation?
    private var rateObserver: NSKeyValueObservation?
    private var endObserver: NSObjectProtocol?
    private var commandTargets: [(MPRemoteCommand, Any)] = []
    private var stopped = false
    private var teardownTask: Task<Void, Error>?
    private var started = false
    private var initialSeekDone = false
    private var initialTicks: Int64
    private let timelineOffsetTicks: Int64
    private var lastReport: TimeInterval = 0

    init(item: MediaItem, url: URL, plan: PlaybackPlan?, session: AccountSession?) throws {
        self.item = item
        proxyId = plan?.proxyId
        source = plan?.source
        playMethod = plan?.playMethod ?? "DirectPlay"
        timelineOffsetTicks = max(0, plan?.startOffsetTicks ?? 0)
        initialTicks = max(0, (plan?.startTicks ?? 0) - (plan?.startOffsetTicks ?? 0))
        // Playback URLs come from the Rust planner and are pinned to the server origin.
        // No custom AVFoundation keys or third-party software decoder are used.
        player = AVPlayer(url: url)
        player.allowsExternalPlayback = true
        player.automaticallyWaitsToMinimizeStalling = true
        if let plan, let session {
            reporter = PlaybackReporter(session: session, itemId: item.Id, plan: plan) { operation, session, fields in
                try await RustBridge.notify(operation, session: session, fields: fields)
            }
        }
        else { reporter = nil }
        try AVAudioSession.sharedInstance().setCategory(.playback, mode: item.Type == "Audio" ? .default : .moviePlayback, options: [])
        statusObserver = player.currentItem?.observe(\.status, options: [.initial, .new]) { [weak self] playerItem, _ in
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                self.isPlaying = self.player.rate > 0
                if playerItem.status == .failed { self.failure = "This stream could not be played. Try a different quality or check the server's playback settings." }
                if playerItem.status == .readyToPlay, !self.initialSeekDone {
                    self.initialSeekDone = true
                    do { try AVAudioSession.sharedInstance().setActive(true) }
                    catch { self.failure = "The audio output could not be activated."; return }
                    if self.initialTicks > 0 {
                        await self.player.seek(to: CMTime(value: self.initialTicks, timescale: 10_000_000), toleranceBefore: .zero, toleranceAfter: .zero)
                    }
                    guard !self.stopped else { return }
                    self.player.play()
                }
            }
        }
        rateObserver = player.observe(\.rate, options: [.new]) { [weak self] _, _ in
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                self.isPlaying = self.player.rate > 0
                if self.player.rate > 0 && !self.started {
                    self.started = true
                    await self.report("started")
                } else if self.started { await self.report("progress") }
                self.updateNowPlaying()
            }
        }
        timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(seconds: 0.5, preferredTimescale: 600), queue: .main) { [weak self] time in
            Task { @MainActor [weak self] in
                guard let self, self.started, !self.stopped else { return }
                self.positionSeconds = time.seconds.isFinite ? max(0, time.seconds + Double(self.timelineOffsetTicks) / 10_000_000) : 0
                guard ProcessInfo.processInfo.systemUptime - self.lastReport >= 10 else { return }
                self.lastReport = ProcessInfo.processInfo.systemUptime
                await self.report("progress")
                self.updateNowPlaying()
            }
        }
        endObserver = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: player.currentItem, queue: .main) { [weak self] _ in
            Task { @MainActor [weak self] in self?.stop(); self?.onFinish?() }
        }
        installRemoteCommands()
        updateNowPlaying()
    }

    func toggle() { if player.rate > 0 { player.pause() } else { player.play() } }
    func seek(seconds: Double) {
        guard seconds.isFinite, seconds >= 0 else { return }
        let offset = Double(timelineOffsetTicks) / 10_000_000
        if seconds < offset {
            guard seconds <= Double(Int64.max / 10_000_000) else { return }
            onSeekOutsideWindow?(Int64(seconds * 10_000_000))
        } else { player.seek(to: CMTime(seconds: seconds - offset, preferredTimescale: 600)) }
    }

    private var ticks: Int64 {
        let seconds = player.currentTime().seconds
        return seconds.isFinite ? Int64(max(0, min(seconds * 10_000_000 + Double(timelineOffsetTicks), Double(Int64.max - 1024)))) : timelineOffsetTicks
    }

    private func report(_ operation: String) async {
        guard let reporter else { return }
        do {
            try await reporter.report(operation, ticks: ticks, paused: player.rate == 0)
            reportingFailure = nil
        } catch {
            reportingFailure = "Watch progress could not sync. Check your connection before leaving playback."
        }
    }

    @discardableResult func stop() -> Task<Void, Error> {
        if let teardownTask { return teardownTask }
        stopped = true
        let position = ticks
        player.pause()
        isPlaying = false
        if let observer = timeObserver { player.removeTimeObserver(observer); timeObserver = nil }
        statusObserver?.invalidate()
        rateObserver?.invalidate()
        if let endObserver { NotificationCenter.default.removeObserver(endObserver) }
        for (command, target) in commandTargets { command.removeTarget(target) }
        commandTargets.removeAll()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
        let wasStarted = started, reporter = reporter, proxyId = proxyId
        let task = Task {
            if let reporter { try await reporter.finish(ticks: position, wasStarted: wasStarted) }
            else if let proxyId { let _: EmptyValue = try await RustBridge.call("stopPlaybackProxy", fields: ["proxyId": proxyId]) }
        }
        teardownTask = task
        Task { [weak self] in
            do { try await task.value }
            catch { self?.reportingFailure = "Playback cleanup could not be confirmed. Check your connection." }
        }
        player.replaceCurrentItem(with: nil)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        return task
    }

    private func updateNowPlaying() {
        var info: [String: Any] = [MPMediaItemPropertyTitle: item.title, MPNowPlayingInfoPropertyElapsedPlaybackTime: Double(ticks) / 10_000_000, MPNowPlayingInfoPropertyPlaybackRate: player.rate, MPNowPlayingInfoPropertyMediaType: item.Type == "Audio" ? MPNowPlayingInfoMediaType.audio.rawValue : MPNowPlayingInfoMediaType.video.rawValue]
        if let artist = item.Artists?.joined(separator: ", ") ?? item.AlbumArtist { info[MPMediaItemPropertyArtist] = artist }
        if let album = item.Album { info[MPMediaItemPropertyAlbumTitle] = album }
        if let duration = item.RunTimeTicks { info[MPMediaItemPropertyPlaybackDuration] = Double(duration) / 10_000_000 }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    private func installRemoteCommands() {
        let center = MPRemoteCommandCenter.shared()
        let play = center.playCommand.addTarget { [weak self] _ in Task { @MainActor in self?.player.play() }; return .success }
        let pause = center.pauseCommand.addTarget { [weak self] _ in Task { @MainActor in self?.player.pause() }; return .success }
        let position = center.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let event = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            Task { @MainActor in self?.seek(seconds: event.positionTime) }
            return .success
        }
        let next = center.nextTrackCommand.addTarget { [weak self] _ in Task { @MainActor in self?.onNext?() }; return .success }
        let previous = center.previousTrackCommand.addTarget { [weak self] _ in Task { @MainActor in self?.onPrevious?() }; return .success }
        commandTargets = [(center.playCommand, play), (center.pauseCommand, pause), (center.changePlaybackPositionCommand, position), (center.nextTrackCommand, next), (center.previousTrackCommand, previous)]
    }
}

struct NativePlayer: UIViewControllerRepresentable {
    @ObservedObject var playback: PlaybackController
    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        controller.player = playback.player
        controller.showsPlaybackControls = true
        #if os(iOS)
        controller.allowsPictureInPicturePlayback = true
        controller.canStartPictureInPictureAutomaticallyFromInline = true
        #endif
        return controller
    }
    func updateUIViewController(_ controller: AVPlayerViewController, context: Context) { controller.player = playback.player }
}

struct PlaybackScreen: View {
    @ObservedObject var playback: PlaybackController
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        ZStack(alignment: .topTrailing) {
            NativePlayer(playback: playback).ignoresSafeArea()
            if let failure = playback.failure {
                VStack(spacing: 20) {
                    Image(systemName: "exclamationmark.triangle").font(.largeTitle)
                    Text("Playback interrupted").font(.title2.bold())
                    Text(failure).multilineTextAlignment(.center)
                    Button("Close player") { playback.stop(); dismiss() }.buttonStyle(.borderedProminent)
                }.padding(32).background(.regularMaterial).cornerRadius(20).padding()
            }
            #if os(iOS)
            Button { playback.stop(); dismiss() } label: { Image(systemName: "xmark.circle.fill").font(.title).symbolRenderingMode(.palette).foregroundStyle(.white, .black.opacity(0.5)) }
                .accessibilityLabel("Close player").padding(20)
            #endif
        }
        .onDisappear { playback.stop() }
        .alert("Watch progress", isPresented: Binding(get: { playback.reportingFailure != nil }, set: { if !$0 { playback.reportingFailure = nil } })) {
            Button("OK") { playback.reportingFailure = nil }
        } message: { Text(playback.reportingFailure ?? "") }
    }
}
