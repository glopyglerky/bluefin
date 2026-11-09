import AVKit
import MediaPlayer
import SwiftUI

@MainActor final class PlaybackController: ObservableObject, Identifiable {
    let id = UUID()
    @Published private(set) var item: MediaItem
    let player: AVPlayer
    @Published var failure: String?
    @Published var reportingFailure: String?
    @Published var positionSeconds: Double = 0
    @Published var isPlaying = false
    @Published var isBuffering = false
    @Published var isFavorite = false
    var onNext: (() -> Void)? { didSet { MPRemoteCommandCenter.shared().nextTrackCommand.isEnabled = onNext != nil } }
    var onPrevious: (() -> Void)? { didSet { MPRemoteCommandCenter.shared().previousTrackCommand.isEnabled = onPrevious != nil } }
    var onFinish: (() -> Void)?
    var onReplay: (() -> Void)?
    var onFavorite: (() -> Void)? { didSet { MPRemoteCommandCenter.shared().likeCommand.isEnabled = onFavorite != nil } }
    var onSeekOutsideWindow: ((Int64) -> Void)?
    @Published var videoSelection = VideoOptionSelection()
    var onVideoOptions: ((VideoOptionSelection) -> Void)?
    @Published private(set) var presentationRate: Float = 1
    @Published private(set) var source: MediaSource?
    @Published private(set) var playMethod: String
    private var reporter: PlaybackReporter?
    private var proxyId: String?
    private let accountSession: AccountSession?
    private var currentNativeItem: AVPlayerItem?
    private var currentItemObserver: NSKeyValueObservation?
    private var candidateGeneration = 0
    private var preparationTail: Task<Void, Never>?
    private var retirement: Task<Void, Error>?
    private var candidateCleanupFailed = false
    private var nextPrepared: (item: MediaItem, native: AVPlayerItem, plan: PlaybackPlan, reporter: PlaybackReporter)?
    var onMusicAdvance: ((MediaItem) -> Void)?
    @Published private(set) var audioProcessingStatus = "Bypassed"
    @Published private(set) var preloadingFailure: String?
    private var audioSettings = NativeAudioSettings()
    private var audioGeneration = 0
    private var audioTask: Task<Void, Never>?
    private var externalObserver: NSKeyValueObservation?
    private var timeObserver: Any?
    private var statusObserver: NSKeyValueObservation?
    private var rateObserver: NSKeyValueObservation?
    private var controlObserver: NSKeyValueObservation?
    private var endObserver: NSObjectProtocol?
    private var audioObservers: [NSObjectProtocol] = []
    private var artworkTask: Task<Void, Never>?
    private var nowPlayingArtwork: MPMediaItemArtwork?
    private var resumeAfterInterruption = false
    private var commandTargets: [(MPRemoteCommand, Any)] = []
    private var stopped = false
    private var teardownTask: Task<Void, Error>?
    private var started = false
    private var initialSeekDone = false
    private var initialTicks: Int64
    private var timelineOffsetTicks: Int64
    private var lastReport: TimeInterval = 0

    init(item: MediaItem, url: URL, plan: PlaybackPlan?, session: AccountSession?) throws {
        self.item = item
        accountSession = session
        isFavorite = item.UserData?.IsFavorite == true
        proxyId = plan?.proxyId
        source = plan?.source ?? item.MediaSources?.first ?? item.MediaStreams.map { MediaSource(MediaStreams: $0) }
        playMethod = plan?.playMethod ?? "DirectPlay"
        timelineOffsetTicks = max(0, plan?.startOffsetTicks ?? 0)
        initialTicks = max(0, (plan?.startTicks ?? 0) - (plan?.startOffsetTicks ?? 0))
        // Playback URLs come from the Rust planner and are pinned to the server origin.
        // No custom AVFoundation keys or third-party software decoder are used.
        let native = AVPlayerItem(url: url)
        native.externalMetadata = Self.displayMetadata(for: item)
        currentNativeItem = native
        player = item.Type == "Audio" ? AVQueuePlayer(items: [native]) : AVPlayer(playerItem: native)
        player.allowsExternalPlayback = true
        player.automaticallyWaitsToMinimizeStalling = true
        if let plan, let session {
            reporter = PlaybackReporter(session: session, itemId: item.Id, plan: plan) { operation, session, fields in
                try await RustBridge.notify(operation, session: session, fields: fields)
            }
        }
        else { reporter = nil }
        try AVAudioSession.sharedInstance().setCategory(.playback, mode: item.Type == "Audio" ? .default : .moviePlayback, options: [])
        observeCurrentStatus()
        currentItemObserver = player.observe(\.currentItem, options: [.new]) { [weak self] _, _ in
            Task { @MainActor [weak self] in self?.adoptPreparedItem() }
        }
        externalObserver = player.observe(\.isExternalPlaybackActive, options: [.new]) { [weak self] _, _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.configureAudio(self.audioSettings)
            }
        }
        rateObserver = player.observe(\.rate, options: [.new]) { [weak self] _, _ in
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                self.isPlaying = self.player.rate > 0
                if self.player.rate > 0 { self.presentationRate = self.player.rate }
                if self.player.rate > 0 && !self.started {
                    self.started = true
                    await self.report("started")
                } else if self.started { await self.report("progress") }
                self.updateNowPlaying()
            }
        }
        controlObserver = player.observe(\.timeControlStatus, options: [.initial, .new]) { [weak self] player, _ in
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                self.isBuffering = player.timeControlStatus == .waitingToPlayAtSpecifiedRate
            }
        }
        timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(seconds: 0.5, preferredTimescale: 600), queue: .main) { [weak self] time in
            Task { @MainActor [weak self] in
                guard let self, self.started, !self.stopped, self.player.currentItem === self.currentNativeItem else { return }
                // Read the native current time after ownership reconciliation. The
                // callback's captured time can belong to the preceding queue item.
                let seconds = self.player.currentTime().seconds
                self.positionSeconds = seconds.isFinite ? max(0, seconds + Double(self.timelineOffsetTicks) / 10_000_000) : 0
                guard ProcessInfo.processInfo.systemUptime - self.lastReport >= 10 else { return }
                self.lastReport = ProcessInfo.processInfo.systemUptime
                await self.report("progress")
                self.updateNowPlaying()
            }
        }
        endObserver = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: nil, queue: .main) { [weak self] note in
            let ended = note.object as? AVPlayerItem
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                self.adoptPreparedItem(ended: ended)
                guard ended === self.currentNativeItem else { return }
                // End notification may precede AVQueuePlayer's currentItem KVO.
                // Let the native queue advance rather than tearing down its successor.
                if self.nextPrepared != nil { return }
                if let duration = self.item.RunTimeTicks { self.positionSeconds = Double(duration) / 10_000_000 }
                self.stop(); self.onFinish?()
            }
        }
        installRemoteCommands()
        installAudioObservers()
        updateNowPlaying()
        refreshArtwork()
    }

    private func observeCurrentStatus() {
        statusObserver?.invalidate()
        statusObserver = currentNativeItem?.observe(\.status, options: [.initial, .new]) { [weak self] playerItem, _ in
            Task { @MainActor [weak self] in
                guard let self, !self.stopped, self.currentNativeItem === playerItem, self.player.currentItem === playerItem else { return }
                self.isPlaying = self.player.rate > 0
                if playerItem.status == .failed { self.failure = "This stream could not be played. Try a different quality or check the server's playback settings." }
                if playerItem.status == .readyToPlay, !self.initialSeekDone {
                    self.initialSeekDone = true
                    do { try AVAudioSession.sharedInstance().setActive(true) }
                    catch { self.failure = "The audio output could not be activated."; return }
                    if self.initialTicks > 0 {
                        await self.player.seek(to: CMTime(value: self.initialTicks, timescale: 10_000_000), toleranceBefore: .zero, toleranceAfter: .zero)
                    }
                    guard !self.stopped, self.currentNativeItem === playerItem, self.player.currentItem === playerItem else { return }
                    self.resume()
                }
            }
        }
    }

    private func refreshArtwork() {
        artworkTask?.cancel()
        nowPlayingArtwork = nil
        guard let session = accountSession else { return }
        let requested = item.Id
        let item = item
        artworkTask = Task { [weak self] in
            let image = await Artwork.nativeImage(for: item, session: session)
            guard !Task.isCancelled, let self, !self.stopped, self.item.Id == requested, let image else { return }
            self.nowPlayingArtwork = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
            self.updateNowPlaying()
        }
    }

    /// Store owns ordering/repeat. This method owns only the exact supplied candidate.
    func setNextMusicCandidate(_ candidate: MediaItem?, session: AccountSession) {
        guard item.Type == "Audio", !stopped, let queue = player as? AVQueuePlayer else { return }
        guard let owner = accountSession, owner.server == session.server, owner.userId == session.userId,
              owner.deviceId == session.deviceId, owner.token == session.token else {
            preloadingFailure = "The next track belongs to a different playback account."; return
        }
        adoptPreparedItem()
        candidateGeneration += 1
        let generation = candidateGeneration
        let previous = preparationTail
        let retired = retirement
        let old = nextPrepared
        nextPrepared = nil
        if let old { queue.remove(old.native) }
        preparationTail = Task { [weak self] in
            await previous?.value
            do {
                if let old { try await old.reporter.finish(ticks: 0, wasStarted: false) }
                try await retired?.value
            } catch { self?.candidateCleanupFailed = true; self?.preloadingFailure = "Previous stream cleanup failed; preloading is paused."; return }
            guard let self, !self.stopped, self.candidateGeneration == generation, !self.candidateCleanupFailed, let candidate, candidate.Type == "Audio" else { return }
            var owned: PlaybackPlan?
            do {
                let plan: PlaybackPlan = try await RustBridge.call("playback", session: session, fields: ["itemId": candidate.Id, "startTicks": 0, "mediaType": "Audio"])
                owned = plan
                guard !self.stopped, self.candidateGeneration == generation else { throw CancellationError() }
                guard let url = URL(string: plan.url), (plan.startTicks ?? 0) == 0, (plan.startOffsetTicks ?? 0) == 0 else { throw BluefinError.message("Next stream cannot be preloaded from its beginning.") }
                let native = AVPlayerItem(url: url)
                native.externalMetadata = Self.displayMetadata(for: item)
                guard queue.canInsert(native, after: self.currentNativeItem) else { throw BluefinError.message("The next track could not be queued.") }
                let reporter = PlaybackReporter(session: session, itemId: candidate.Id, plan: plan) { operation, session, fields in
                    try await RustBridge.notify(operation, session: session, fields: fields)
                }
                self.nextPrepared = (candidate, native, plan, reporter)
                queue.insert(native, after: self.currentNativeItem)
                owned = nil
                self.preloadingFailure = nil
                self.attachAudio(to: native, media: candidate, generation: self.audioGeneration, isCurrent: false)
            } catch {
                if let owned {
                    do { try await RustBridge.cleanupPlaybackPlan(owned, session: session) }
                    catch { self.candidateCleanupFailed = true; self.preloadingFailure = "Unused stream cleanup could not be confirmed."; return }
                }
                if !(error is CancellationError) { self.preloadingFailure = error.localizedDescription }
            }
        }
    }

    /// Await before a manual replacement allocates another Rust playback plan.
    func discardNextMusicCandidate() -> Task<Void, Error> {
        if let session = accountSession { setNextMusicCandidate(nil, session: session) }
        let preparation = preparationTail, retired = retirement
        return Task {
            await preparation?.value
            try await retired?.value
            if self.candidateCleanupFailed { throw BluefinError.message("Prefetched stream cleanup could not be confirmed.") }
        }
    }

    private func adoptPreparedItem(ended: AVPlayerItem? = nil) {
        guard !stopped, let next = nextPrepared, player.currentItem === next.native || ended === next.native else { return }
        let oldReporter = reporter, oldStarted = started
        let endTicks = item.RunTimeTicks ?? Int64(positionSeconds * 10_000_000)
        let prior = retirement
        retirement = Task {
            var failure: Error?
            do { try await prior?.value } catch { failure = error }
            do { try await oldReporter?.finish(ticks: endTicks, wasStarted: oldStarted) } catch { failure = failure ?? error }
            if let failure { throw failure }
        }
        nextPrepared = nil
        currentNativeItem = next.native
        item = next.item; source = next.plan.source; playMethod = next.plan.playMethod ?? "DirectPlay"
        reporter = next.reporter; proxyId = next.plan.proxyId
        timelineOffsetTicks = 0; initialTicks = 0; initialSeekDone = true
        started = true; positionSeconds = 0; lastReport = 0
        isFavorite = item.UserData?.IsFavorite == true
        failure = nil
        observeCurrentStatus(); refreshArtwork(); updateNowPlaying()
        configureAudio(audioSettings)
        Task { await self.report("started") }
        onMusicAdvance?(item)
    }

    func configureAudio(_ settings: NativeAudioSettings) {
        audioSettings = settings
        audioGeneration += 1
        audioTask?.cancel()
        guard let currentNativeItem else { return }
        attachAudio(to: currentNativeItem, media: item, generation: audioGeneration, isCurrent: true)
        if let nextPrepared { attachAudio(to: nextPrepared.native, media: nextPrepared.item, generation: audioGeneration, isCurrent: false) }
    }

    private func attachAudio(to native: AVPlayerItem, media: MediaItem, generation: Int, isCurrent: Bool) {
        native.audioMix = nil
        let settings = audioSettings, gain = settings.gainDB(for: media)
        let publish: (String) -> Void = { [weak self, weak native] value in
            Task { @MainActor [weak self, weak native] in
                guard let self, !self.stopped, self.audioGeneration == generation, self.currentNativeItem === native else { return }
                self.audioProcessingStatus = value
            }
        }
        guard media.Type == "Audio", settings.bands.contains(where: { $0 != 0 }) || gain != 0 else { if isCurrent { audioProcessingStatus = "Bypassed" }; return }
        guard !player.isExternalPlaybackActive else { publish("Unavailable: external playback uses the receiver's audio path"); return }
        guard let url = (native.asset as? AVURLAsset)?.url, url.pathExtension.lowercased() != "m3u8", (isCurrent ? source?.Container : nextPrepared?.plan.source?.Container)?.lowercased() != "hls" else { publish("Unavailable: HLS audio processing is bypassed"); return }
        let fileContainers: Set<String> = ["mp3", "mp4", "m4a", "aac", "flac", "alac", "wav", "aiff", "aif", "caf", "ogg", "opus"]
        guard url.isFileURL || fileContainers.contains(url.pathExtension.lowercased()) || fileContainers.contains((isCurrent ? source?.Container : nextPrepared?.plan.source?.Container)?.lowercased() ?? "") else {
            publish("Unavailable: source is not identified as file/progressive audio"); return
        }
        publish("Checking decoded audio format")
        let task = Task { [weak self, weak native] in
            guard let native else { return }
            do {
                let tracks = try await native.asset.loadTracks(withMediaType: .audio)
                let protected = try await native.asset.load(.hasProtectedContent)
                let composable = try await native.asset.load(.isComposable)
                guard let self, !self.stopped, self.audioGeneration == generation,
                      self.currentNativeItem === native || self.nextPrepared?.native === native else { return }
                // A single file audio track only; playlists/protected content never receive a tap.
                guard !protected, composable, tracks.count == 1, let track = tracks.first else { publish("Unavailable: streaming, protected or multiple audio tracks; bypassed"); return }
                guard let mix = NativeAudioDSP.mix(track: track, bands: settings.bands, gainDB: gain, status: publish) else { publish("Unavailable: native audio tap could not be created"); return }
                native.audioMix = mix
            } catch { publish("Unavailable: audio format could not be loaded") }
        }
        if isCurrent { audioTask = task }
    }

    func toggle() {
        if stopped { onReplay?(); return }
        if player.rate > 0 { player.pause() } else { resume() }
    }
    var supportedPresentationRates: [Float] {
        guard let current = player.currentItem, current.status == .readyToPlay else { return [1] }
        return (current.canPlaySlowForward ? [0.5] : []) + [1] + (current.canPlayFastForward ? [1.25, 1.5, 2] : [])
    }
    func setPresentationRate(_ rate: Float) {
        guard supportedPresentationRates.contains(rate) else { return }
        presentationRate = rate
        if player.rate > 0 { player.rate = rate }
    }
    func resume() { if stopped { onReplay?() } else { player.play(); player.rate = presentationRate } }
    func seek(seconds: Double) {
        guard seconds.isFinite, seconds >= 0 else { return }
        let offset = Double(timelineOffsetTicks) / 10_000_000
        if seconds < offset {
            guard seconds <= Double(Int64.max / 10_000_000) else { return }
            onSeekOutsideWindow?(Int64(seconds * 10_000_000))
        } else { player.seek(to: CMTime(seconds: seconds - offset, preferredTimescale: 600)) }
    }

    private static func displayMetadata(for item: MediaItem) -> [AVMetadataItem] {
        [(AVMetadataIdentifier.commonIdentifierTitle, item.title),
         (AVMetadataIdentifier.commonIdentifierDescription, item.Overview ?? item.subtitle)]
            .filter { !$0.1.isEmpty }.map { identifier, text in
                let value = AVMutableMetadataItem()
                value.identifier = identifier; value.value = text as NSString
                value.extendedLanguageTag = "und"
                return value
            }
    }

    private var ticks: Int64 {
        let seconds = player.currentItem == nil ? positionSeconds - Double(timelineOffsetTicks) / 10_000_000 : player.currentTime().seconds
        return seconds.isFinite ? Int64(max(0, min(seconds * 10_000_000 + Double(timelineOffsetTicks), Double(Int64.max - 1024)))) : timelineOffsetTicks
    }

    private func report(_ operation: String) async {
        guard player.currentItem === currentNativeItem, let reporter else { return }
        do {
            try await reporter.report(operation, ticks: ticks, paused: player.rate == 0)
            reportingFailure = nil
        } catch {
            reportingFailure = "Watch progress could not sync. Check your connection before leaving playback."
        }
    }

    @discardableResult func stop() -> Task<Void, Error> {
        if let teardownTask { return teardownTask }
        adoptPreparedItem()
        stopped = true
        candidateGeneration += 1
        audioGeneration += 1
        audioTask?.cancel()
        currentItemObserver?.invalidate()
        externalObserver?.invalidate()
        let position = ticks
        player.pause()
        isPlaying = false
        isBuffering = false
        if let observer = timeObserver { player.removeTimeObserver(observer); timeObserver = nil }
        statusObserver?.invalidate()
        rateObserver?.invalidate()
        controlObserver?.invalidate()
        artworkTask?.cancel()
        for observer in audioObservers { NotificationCenter.default.removeObserver(observer) }
        audioObservers.removeAll()
        if let endObserver { NotificationCenter.default.removeObserver(endObserver) }
        for (command, target) in commandTargets { command.removeTarget(target) }
        commandTargets.removeAll()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
        let wasStarted = started, reporter = reporter, proxyId = proxyId
        let preparation = preparationTail, retirement = retirement
        let queued = nextPrepared
        nextPrepared = nil
        let task = Task {
            await preparation?.value
            var cleanupError: Error? = self.candidateCleanupFailed ? BluefinError.message("Prefetched stream cleanup could not be confirmed.") : nil
            do { try await retirement?.value } catch { cleanupError = error }
            if let queued {
                do { try await queued.reporter.finish(ticks: 0, wasStarted: false) } catch { cleanupError = cleanupError ?? error }
            }
            do {
            if let reporter { try await reporter.finish(ticks: position, wasStarted: wasStarted) }
            else if let proxyId { let _: EmptyValue = try await RustBridge.call("stopPlaybackProxy", fields: ["proxyId": proxyId]) }
            } catch { cleanupError = cleanupError ?? error }
            if let cleanupError { throw cleanupError }
        }
        teardownTask = task
        Task { [weak self] in
            do { try await task.value }
            catch { self?.reportingFailure = "Playback cleanup could not be confirmed. Check your connection." }
        }
        if let queue = player as? AVQueuePlayer { queue.removeAllItems() }
        else { player.replaceCurrentItem(with: nil) }
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        return task
    }

    private func updateNowPlaying() {
        guard !stopped else { return }
        var info: [String: Any] = [MPMediaItemPropertyTitle: item.title, MPNowPlayingInfoPropertyElapsedPlaybackTime: Double(ticks) / 10_000_000, MPNowPlayingInfoPropertyPlaybackRate: player.rate, MPNowPlayingInfoPropertyMediaType: item.Type == "Audio" ? MPNowPlayingInfoMediaType.audio.rawValue : MPNowPlayingInfoMediaType.video.rawValue]
        if let artist = item.Artists?.joined(separator: ", ") ?? item.AlbumArtist { info[MPMediaItemPropertyArtist] = artist }
        if let album = item.Album { info[MPMediaItemPropertyAlbumTitle] = album }
        if let duration = item.RunTimeTicks { info[MPMediaItemPropertyPlaybackDuration] = Double(duration) / 10_000_000 }
        if let nowPlayingArtwork { info[MPMediaItemPropertyArtwork] = nowPlayingArtwork }
        info[MPNowPlayingInfoPropertyDefaultPlaybackRate] = 1.0
        MPRemoteCommandCenter.shared().likeCommand.isActive = isFavorite
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    func updateFavorite(_ value: Bool) { isFavorite = value; updateNowPlaying() }

    private func installAudioObservers() {
        let notifications = NotificationCenter.default
        audioObservers.append(notifications.addObserver(forName: AVAudioSession.interruptionNotification, object: AVAudioSession.sharedInstance(), queue: .main) { [weak self] note in
            let type = (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init(rawValue:))
            let options = AVAudioSession.InterruptionOptions(rawValue: note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0)
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                if type == .began {
                    self.resumeAfterInterruption = self.player.rate > 0
                    self.player.pause()
                } else if type == .ended {
                    if self.resumeAfterInterruption && options.contains(.shouldResume) {
                        do { try AVAudioSession.sharedInstance().setActive(true); self.resume() }
                        catch { self.failure = "The audio output could not resume." }
                    }
                    self.resumeAfterInterruption = false
                }
            }
        })
        audioObservers.append(notifications.addObserver(forName: AVAudioSession.routeChangeNotification, object: AVAudioSession.sharedInstance(), queue: .main) { [weak self] note in
            let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt).flatMap(AVAudioSession.RouteChangeReason.init(rawValue:))
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                if reason == .oldDeviceUnavailable { self.player.pause() }
            }
        })
        audioObservers.append(notifications.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: AVAudioSession.sharedInstance(), queue: .main) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.stop()
                self.failure = "The audio service restarted. Play this title again to reconnect."
            }
        })
    }

    private func installRemoteCommands() {
        let center = MPRemoteCommandCenter.shared()
        let play = center.playCommand.addTarget { [weak self] _ in Task { @MainActor in self?.resume() }; return .success }
        let pause = center.pauseCommand.addTarget { [weak self] _ in Task { @MainActor in self?.player.pause() }; return .success }
        let position = center.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let event = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            Task { @MainActor in self?.seek(seconds: event.positionTime) }
            return .success
        }
        let next = center.nextTrackCommand.addTarget { [weak self] _ in Task { @MainActor in self?.onNext?() }; return .success }
        let previous = center.previousTrackCommand.addTarget { [weak self] _ in Task { @MainActor in self?.onPrevious?() }; return .success }
        let toggle = center.togglePlayPauseCommand.addTarget { [weak self] _ in Task { @MainActor in self?.toggle() }; return .success }
        let favorite = center.likeCommand.addTarget { [weak self] _ in Task { @MainActor in self?.onFavorite?() }; return .success }
        center.nextTrackCommand.isEnabled = onNext != nil
        center.previousTrackCommand.isEnabled = onPrevious != nil
        center.likeCommand.isEnabled = onFavorite != nil
        center.likeCommand.localizedTitle = "Favorite"
        commandTargets = [(center.playCommand, play), (center.pauseCommand, pause), (center.changePlaybackPositionCommand, position), (center.nextTrackCommand, next), (center.previousTrackCommand, previous), (center.togglePlayPauseCommand, toggle), (center.likeCommand, favorite)]
    }
}

struct NativePlayer: UIViewControllerRepresentable {
    @ObservedObject var playback: PlaybackController
    var onOptions: (() -> Void)? = nil
    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        controller.player = playback.player
        controller.showsPlaybackControls = true
        #if os(tvOS)
        controller.appliesPreferredDisplayCriteriaAutomatically = true
        if let onOptions {
            controller.transportBarCustomMenuItems = [UIAction(title: "Video options", image: UIImage(systemName: "slider.horizontal.3")) { _ in onOptions() }]
        }
        #endif
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
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var optionsVisible = false
    var body: some View {
        ZStack(alignment: .topTrailing) {
            NativePlayer(playback: playback, onOptions: { optionsVisible = true }).ignoresSafeArea()
            if let failure = playback.failure {
                VStack(spacing: 20) {
                    Image(systemName: "exclamationmark.triangle").font(.largeTitle)
                    Text("Playback interrupted").font(.title2.bold())
                    Text(failure).multilineTextAlignment(.center)
                    Button("Close player") { playback.stop(); dismiss() }.buttonStyle(.borderedProminent)
                }.padding(32).background(.regularMaterial).cornerRadius(20).padding()
            }
            #if os(iOS)
            HStack(spacing: 18) {
                Button { optionsVisible = true } label: { Image(systemName: "slider.horizontal.3").font(.title2).foregroundStyle(.white).frame(width: 44, height: 44).background(.black.opacity(0.5), in: Circle()) }
                    .accessibilityLabel("Video options")
                Button { playback.stop(); dismiss() } label: { Image(systemName: "xmark.circle.fill").font(.title).symbolRenderingMode(.palette).foregroundStyle(.white, .black.opacity(0.5)) }
                    .accessibilityLabel("Close player")
            }.padding(20)
            #endif
        }
        .sheet(isPresented: $optionsVisible) {
            NavigationView {
                VideoOptionsView(chapters: playback.item.Chapters ?? [], source: playback.source,
                    selection: playback.videoSelection, isRenegotiating: store.startingPlayback || store.preview,
                    supportedRates: playback.supportedPresentationRates, presentationRate: playback.presentationRate,
                    nextEpisodeTitle: nil, onSeek: { playback.seek(seconds: $0); optionsVisible = false },
                    onRenegotiate: { selection in playback.onVideoOptions?(selection); optionsVisible = false },
                    onPresentationRate: { playback.setPresentationRate($0) }, onNextEpisode: {})
                .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { optionsVisible = false } } }
            }.navigationViewStyle(.stack)
        }
        .onDisappear { playback.stop() }
        .alert("Watch progress", isPresented: Binding(get: { playback.reportingFailure != nil }, set: { if !$0 { playback.reportingFailure = nil } })) {
            Button("OK") { playback.reportingFailure = nil }
        } message: { Text(playback.reportingFailure ?? "") }
    }
}
