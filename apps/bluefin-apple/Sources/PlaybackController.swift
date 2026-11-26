import AVKit
import Combine
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
    @Published private(set) var selectedAudioStreamIndex: Int?
    var audioSourceStream: MediaStream? {
        AudioQualitySummary.selectedStream(source: source, index: selectedAudioStreamIndex)
    }
    private(set) var selectedMediaSourceID: String?
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
    @Published private(set) var karaoke: KaraokeAudio?
    var isStopped: Bool { stopped }
    let karaokeMicrophone = KaraokeMicrophone()
    #if os(tvOS)
    private var continuityMicrophoneStorage: AnyObject?
    @available(tvOS 17.0, *)
    var continuityMicrophone: KaraokeContinuityMicrophone {
        if let existing = continuityMicrophoneStorage as? KaraokeContinuityMicrophone { return existing }
        let microphone = KaraokeContinuityMicrophone()
        continuityMicrophoneStorage = microphone
        return microphone
    }
    #endif
    func stopKaraokeMicrophones() {
        karaokeMicrophone.stop()
        #if os(tvOS)
        if #available(tvOS 17.0, *) { (continuityMicrophoneStorage as? KaraokeContinuityMicrophone)?.stop() }
        #endif
    }
    private var karaokeOriginal: AVPlayerItem?
    private var karaokeGeneration = 0
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
    private var seekingInitialPosition = false
    private var skipSegments: [MediaSkipSegment] = []
    private var segmentRequest: Task<Void, Never>?
    @Published private(set) var activeSkipSegment: MediaSkipSegment?
    private var initialTicks: Int64
    private var timelineOffsetTicks: Int64
    private var lastReport: TimeInterval = 0

    init(item: MediaItem, url: URL, plan: PlaybackPlan?, session: AccountSession?) throws {
        self.item = item
        accountSession = session
        isFavorite = item.UserData?.IsFavorite == true
        proxyId = plan?.proxyId
        let observedSource = plan?.source ?? item.MediaSources?.first ?? item.MediaStreams.map { MediaSource(MediaStreams: $0) }
        source = observedSource
        selectedAudioStreamIndex = plan?.audioStreamIndex
        selectedMediaSourceID = plan?.mediaSourceId ?? observedSource?.Id
        playMethod = plan?.playMethod ?? "DirectPlay"
        timelineOffsetTicks = max(0, plan?.startOffsetTicks ?? 0)
        initialTicks = max(0, (plan?.startTicks ?? 0) - (plan?.startOffsetTicks ?? 0))
        // Playback URLs come from the Rust planner and are pinned to the server origin.
        // No custom AVFoundation keys or third-party software decoder are used.
        let native = AVPlayerItem(url: url)
        native.externalMetadata = Self.displayMetadata(for: item)
        currentNativeItem = native
        player = item.Type == "Audio" ? AVQueuePlayer(items: [native]) : AVPlayer(playerItem: native)
        #if !os(visionOS)
        player.allowsExternalPlayback = true
        #endif
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
        #if !os(visionOS)
        externalObserver = player.observe(\.isExternalPlaybackActive, options: [.new]) { [weak self] _, _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.configureAudio(self.audioSettings)
            }
        }
        #endif
        rateObserver = player.observe(\.rate, options: [.new]) { [weak self] _, _ in
            Task { @MainActor [weak self] in
                guard let self, !self.stopped else { return }
                self.isPlaying = self.player.rate > 0
                guard self.initialSeekDone else { return }
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
                self.updateSkipSegment()
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
                if playerItem.status == .readyToPlay, !self.initialSeekDone, !self.seekingInitialPosition {
                    self.seekingInitialPosition = true
                    defer { self.seekingInitialPosition = false }
                    do { try AVAudioSession.sharedInstance().setActive(true) }
                    catch { self.failure = "The audio output could not be activated."; return }
                    if self.initialTicks > 0 {
                        self.player.pause()
                        let position = CMTime(value: self.initialTicks, timescale: 10_000_000)
                        let completed = await self.player.seek(to: position, toleranceBefore: .zero, toleranceAfter: .zero)
                        guard !self.stopped, self.currentNativeItem === playerItem, self.player.currentItem === playerItem else { return }
                        guard completed else {
                            self.failure = "The player could not reach your saved position."
                            return
                        }
                    }
                    guard !self.stopped, self.currentNativeItem === playerItem, self.player.currentItem === playerItem else { return }
                    self.initialSeekDone = true
                    self.resume()
                    self.updateSkipSegment()
                }
            }
        }
    }

    func loadSkipSegments(for session: AccountSession) {
        guard item.Type != "Audio", item.Type != "TvChannel" else { return }
        let itemID = item.Id
        segmentRequest?.cancel()
        segmentRequest = Task { [weak self] in
            struct Page: Decodable, Sendable { let Items: [MediaSkipSegment] }
            guard let page: Page = try? await RustBridge.call("mediaSegments", session: session, fields: ["itemId": itemID]),
                  !Task.isCancelled else { return }
            self?.setSkipSegments(page.Items)
        }
    }

    func setSkipSegments(_ segments: [MediaSkipSegment]) {
        guard !stopped else { return }
        skipSegments = MediaSkipSegment.admitted(segments, for: item)
        updateSkipSegment()
    }

    private func updateSkipSegment() {
        guard !skipSegments.isEmpty || activeSkipSegment != nil else { return }
        let position = ticks
        let active = initialSeekDone ? skipSegments.first { $0.contains(position) } : nil
        if activeSkipSegment != active { activeSkipSegment = active }
    }

    func skip(_ segment: MediaSkipSegment) {
        guard !stopped, initialSeekDone, activeSkipSegment == segment, segment.contains(ticks) else { return }
        activeSkipSegment = nil
        seek(seconds: Double(segment.end) / 10_000_000)
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
        guard item.Type == "Audio", !stopped, karaoke == nil, let queue = player as? AVQueuePlayer else { return }
        guard let owner = accountSession, owner.server == session.server, owner.userId == session.userId,
              owner.deviceId == session.deviceId, owner.token == session.token,
              owner.meshSessionId == session.meshSessionId else {
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
        stopKaraokeMicrophones()
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
        selectedAudioStreamIndex = next.plan.audioStreamIndex
        selectedMediaSourceID = next.plan.mediaSourceId
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
        stopKaraokeMicrophones()
        audioSettings = settings
        audioGeneration += 1
        audioTask?.cancel()
        guard let currentNativeItem else { return }
        attachAudio(to: currentNativeItem, media: item, generation: audioGeneration, isCurrent: true)
        if let nextPrepared { attachAudio(to: nextPrepared.native, media: nextPrepared.item, generation: audioGeneration, isCurrent: false) }
    }

    private func attachAudio(to native: AVPlayerItem, media: MediaItem, generation: Int, isCurrent: Bool) {
        if karaoke?.nativeItem === native { return }
        native.audioMix = nil
        let settings = audioSettings, gain = settings.gainDB(for: media)
        let publish: (String) -> Void = { [weak self, weak native] value in
            Task { @MainActor [weak self, weak native] in
                guard let self, !self.stopped, self.audioGeneration == generation, self.currentNativeItem === native else { return }
                self.audioProcessingStatus = value
            }
        }
        guard media.Type == "Audio", settings.bands.contains(where: { $0 != 0 }) || gain != 0 else { if isCurrent { audioProcessingStatus = "Bypassed" }; return }
        guard !externalPlaybackActive else { publish("Unavailable: external playback uses the receiver's audio path"); return }
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

    /// Caller admits rights, verifies files and binds this package to the current authenticated account.
    /// Original playback plan/reporter remains owned until normal teardown; no new proxy is allocated.
    func enableKaraoke(_ package: KaraokePackage, retaining resource: AnyObject? = nil,
                       isCurrent: () -> Bool = { true }) async throws {
        guard !stopped, item.Type == "Audio", karaoke == nil, timelineOffsetTicks == 0,
              !externalPlaybackActive, presentationRate == 1, initialSeekDone,
              currentNativeItem?.status == .readyToPlay else { throw KaraokeError.unavailable }
        guard let accountSession, package.itemID == item.Id, package.accountUserID == accountSession.userId, package.accountServer == accountSession.server else { throw KaraokeError.unauthorized }
        guard isCurrent() else { throw KaraokeError.stale }
        karaokeGeneration += 1; let generation = karaokeGeneration
        let original = currentNativeItem
        let prepared = try await KaraokeAudio.prepare(package)
        // Retain local inputs before publishing the item or suspending for a seek.
        if let resource { prepared.retainResources(resource) }
        if let ticks = item.RunTimeTicks {
            guard abs(Double(ticks) / 10_000_000 - prepared.duration) <= 0.05 else { throw KaraokeError.invalidStems }
        }
        try Task.checkCancellation()
        guard generation == karaokeGeneration, !stopped, currentNativeItem === original else { throw KaraokeError.stale }
        try await discardNextMusicCandidate().value
        try Task.checkCancellation()
        guard generation == karaokeGeneration, !stopped, currentNativeItem === original,
              !externalPlaybackActive, isCurrent() else { throw KaraokeError.stale }
        let position = player.currentTime(), playing = player.rate > 0
        guard position.seconds.isFinite else { throw KaraokeError.unavailable }
        player.pause()
        karaokeOriginal = original; karaoke = prepared
        currentNativeItem = prepared.nativeItem
        prepared.nativeItem.externalMetadata = Self.displayMetadata(for: item)
        initialSeekDone = true
        #if !os(visionOS)
        player.allowsExternalPlayback = false
        #endif
        player.replaceCurrentItem(with: prepared.nativeItem)
        observeCurrentStatus()
        await player.seek(to: position, toleranceBefore: .zero, toleranceAfter: .zero)
        guard !stopped, karaoke === prepared else { return }
        audioProcessingStatus = prepared.qualityDescription
        if playing { resume() }
    }

    func disableKaraoke() async {
        karaokeGeneration += 1
        stopKaraokeMicrophones()
        guard karaoke != nil, let original = karaokeOriginal, !stopped else { return }
        let position = player.currentTime(), playing = player.rate > 0
        player.pause(); karaoke = nil; karaokeOriginal = nil
        currentNativeItem = original
        player.replaceCurrentItem(with: original)
        #if !os(visionOS)
        player.allowsExternalPlayback = true
        #endif
        observeCurrentStatus(); configureAudio(audioSettings)
        await player.seek(to: position, toleranceBefore: .zero, toleranceAfter: .zero)
        if playing, !stopped, currentNativeItem === original { resume() }
    }

    private var externalPlaybackActive: Bool {
        #if os(visionOS)
        false
        #else
        player.isExternalPlaybackActive
        #endif
    }

    func toggle() {
        if stopped { onReplay?(); return }
        if player.rate > 0 { player.pause() } else { resume() }
    }
    var supportedPresentationRates: [Float] {
        if karaoke != nil { return [1] }
        guard let current = player.currentItem, current.status == .readyToPlay else { return [1] }
        return (current.canPlaySlowForward ? [0.5] : []) + [1] + (current.canPlayFastForward ? [1.25, 1.5, 2] : [])
    }
    func setPresentationRate(_ rate: Float) {
        guard supportedPresentationRates.contains(rate) else { return }
        presentationRate = rate
        if player.rate > 0 { player.rate = rate }
    }
    func resume() {
        if stopped { onReplay?() }
        else if initialSeekDone || initialTicks == 0 && !seekingInitialPosition {
            player.play()
            player.rate = presentationRate
        }
    }
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
        karaokeGeneration += 1
        stopKaraokeMicrophones()
        karaoke = nil; karaokeOriginal = nil
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
        segmentRequest?.cancel()
        activeSkipSegment = nil
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
                    self.stopKaraokeMicrophones()
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

#if os(iOS) || os(visionOS)
/// Present AVKit itself so the system owns the close button and its visibility.
struct NativeVideoPresentation: UIViewControllerRepresentable {
    let playback: PlaybackController?
    let onClose: (PlaybackController) -> Void

    func makeUIViewController(context: Context) -> Host { Host() }

    func updateUIViewController(_ host: Host, context: Context) {
        host.pending = playback
        host.onClose = onClose
        Task { @MainActor [weak host] in host?.reconcile() }
    }

    final class Host: UIViewController, AVPlayerViewControllerDelegate, UIAdaptivePresentationControllerDelegate {
        var pending: PlaybackController?
        var onClose: ((PlaybackController) -> Void)?
        private var shown: PlaybackController?
        private var native: AVPlayerViewController?
        private var transitioning = false
        private var pictureInPicture = false
        private var failureObservation: AnyCancellable?
        private var skipObservation: AnyCancellable?
        #if os(iOS)
        private var skipButton: UIButton?
        #endif

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            if let native, native.presentingViewController == nil, !transitioning, !pictureInPicture { didClose(native) }
            reconcile()
        }

        func reconcile() {
            guard !transitioning, viewIfLoaded?.window != nil else { return }
            if let native {
                if let pending {
                    if shown !== pending {
                        native.player = pending.player
                        observe(pending)
                    }
                } else {
                    transitioning = true
                    native.dismiss(animated: true) { [weak self, weak native] in
                        guard let self, let native else { return }
                        self.transitioning = false
                        self.didClose(native)
                        self.reconcile()
                    }
                }
                return
            }
            guard let pending, presentedViewController == nil else { return }
            let controller = AVPlayerViewController()
            controller.player = pending.player
            controller.modalPresentationStyle = .fullScreen
            controller.delegate = self
            #if os(iOS)
            controller.allowsPictureInPicturePlayback = true
            #endif
            native = controller
            observe(pending)
            transitioning = true
            present(controller, animated: true) { [weak self] in
                self?.transitioning = false
                self?.reconcile()
                if let message = self?.shown?.failure { self?.showFailure(message) }
            }
            controller.presentationController?.delegate = self
        }

        private func observe(_ playback: PlaybackController) {
            shown = playback
            failureObservation = playback.$failure.compactMap { $0 }.sink { [weak self] text in
                self?.showFailure(text)
            }
            skipObservation = playback.$activeSkipSegment.sink { [weak self, weak playback] segment in
                guard let self, let playback, let native = self.native else { return }
                #if os(visionOS)
                native.contextualActions = segment.flatMap { value in
                    value.kind.map { kind in UIAction(title: kind.title) { [weak playback] _ in playback?.skip(value) } }
                }.map { [$0] } ?? []
                #else
                self.skipButton?.removeFromSuperview()
                self.skipButton = nil
                guard let segment, let kind = segment.kind else { return }
                var style = UIButton.Configuration.filled()
                style.title = kind.title
                style.cornerStyle = .capsule
                let button = UIButton(configuration: style, primaryAction: UIAction { [weak playback] _ in playback?.skip(segment) })
                button.accessibilityIdentifier = "bluefinSkipSegment"
                button.translatesAutoresizingMaskIntoConstraints = false
                native.view.addSubview(button)
                let safeArea = native.view.safeAreaLayoutGuide
                NSLayoutConstraint.activate([
                    button.trailingAnchor.constraint(equalTo: safeArea.trailingAnchor, constant: -16),
                    button.bottomAnchor.constraint(equalTo: safeArea.bottomAnchor, constant: -110)
                ])
                self.skipButton = button
                #endif
            }
        }

        private func showFailure(_ text: String) {
            guard !transitioning, let native, native.viewIfLoaded?.window != nil,
                  native.presentedViewController == nil else { return }
            let alert = UIAlertController(title: "Playback interrupted", message: text, preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "Close player", style: .default) { [weak self, weak native] _ in
                guard let self, let native, native === self.native else { return }
                self.pending = nil
                self.reconcile()
            })
            native.present(alert, animated: true)
        }

        private func didClose(_ controller: UIViewController) {
            guard controller === native, let shown else { return }
            native = nil
            self.shown = nil
            failureObservation = nil
            skipObservation = nil
            #if os(iOS)
            skipButton?.removeFromSuperview()
            skipButton = nil
            #endif
            if pending === shown { pending = nil }
            onClose?(shown)
        }

        func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
            if !pictureInPicture { didClose(presentationController.presentedViewController) }
        }

        func playerViewController(_ playerViewController: AVPlayerViewController,
                                  willEndFullScreenPresentationWithAnimationCoordinator coordinator: any UIViewControllerTransitionCoordinator) {
            coordinator.animate(alongsideTransition: nil) { [weak self, weak playerViewController] context in
                guard !context.isCancelled, let self, let playerViewController, !self.pictureInPicture else { return }
                self.didClose(playerViewController)
            }
        }

        #if os(iOS)
        func playerViewControllerWillStartPictureInPicture(_ playerViewController: AVPlayerViewController) {
            pictureInPicture = true
        }

        func playerViewController(_ playerViewController: AVPlayerViewController,
                                  restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completion: @escaping (Bool) -> Void) {
            if playerViewController.presentingViewController != nil {
                completion(true)
            } else if viewIfLoaded?.window != nil {
                present(playerViewController, animated: true) { completion(true) }
            } else {
                completion(false)
            }
        }

        func playerViewControllerDidStopPictureInPicture(_ playerViewController: AVPlayerViewController) {
            pictureInPicture = false
            if playerViewController.presentingViewController == nil { didClose(playerViewController) }
        }

        func playerViewController(_ playerViewController: AVPlayerViewController, failedToStartPictureInPictureWithError error: any Error) {
            pictureInPicture = false
        }
        #endif
    }
}
#endif

struct NativePlayer: UIViewControllerRepresentable {
    @ObservedObject var playback: PlaybackController
    var onOptions: (() -> Void)? = nil
    final class Coordinator {
        var segment: MediaSkipSegment?
        weak var playback: PlaybackController?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

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
    func updateUIViewController(_ controller: AVPlayerViewController, context: Context) {
        if controller.player !== playback.player { controller.player = playback.player }
        #if os(tvOS)
        let segment = playback.activeSkipSegment
        guard context.coordinator.segment != segment || context.coordinator.playback !== playback else { return }
        context.coordinator.segment = segment
        context.coordinator.playback = playback
        if let segment, let kind = segment.kind {
            controller.contextualActions = [UIAction(title: kind.title) { [weak playback] _ in
                playback?.skip(segment)
            }]
        } else {
            controller.contextualActions = []
        }
        #endif
    }
}

struct PlaybackScreen: View {
    @ObservedObject var playback: PlaybackController
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var optionsVisible = false
    var body: some View {
        ZStack {
            NativePlayer(playback: playback, onOptions: { optionsVisible = true }).ignoresSafeArea()
            if let failure = playback.failure {
                VStack(spacing: 20) {
                    Text("Playback interrupted").font(.title2.bold())
                    Text(failure).multilineTextAlignment(.center)
                    Button("Close player") { dismiss() }.buttonStyle(.borderedProminent).tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary)
                }.padding(32).background(.regularMaterial).cornerRadius(20).padding()
            }
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
        .alert("Watch progress", isPresented: Binding(get: { playback.reportingFailure != nil }, set: { if !$0 { playback.reportingFailure = nil } })) {
            Button("OK") { playback.reportingFailure = nil }
        } message: { Text(playback.reportingFailure ?? "") }
    }
}
