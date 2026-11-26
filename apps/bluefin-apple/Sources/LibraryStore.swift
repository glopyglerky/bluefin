import Foundation
import SwiftUI
import CryptoKit

@MainActor final class LibraryStore: ObservableObject {
    @Published var session = SessionVault.load()
    @Published var preview = false
    @Published var meshInvitationCode: String?
    @Published private(set) var signingOut = false
    @Published var home = HomePage(resume: [], latest: [], nextUp: [])
    @Published private(set) var homeFromCatalog = false
    @Published var libraries: [MediaItem] = []
    @Published var loading = false
    @Published private(set) var libraryProblem: String?
    @Published var error: String?
    @Published var playback: PlaybackController?
    @Published var startingPlayback = false
    @Published var musicExpanded = false
    @Published var musicQueue: [MediaItem] = []
    @Published var musicIndex = 0
    @Published var musicShuffled = false
    @Published var musicRepeat: MusicRepeatMode = .off
    private var unshuffledQueue: [MediaItem] = []
    private var nextMusicCandidateKey: String?
    let offline = OfflineLibrary()
    let deviceMusic = DeviceMusicLibrary()
    @Published var deviceMusicVisible = false
    let audio = AudioPreferences()
    let sports = SportsStore()
    private var accountGeneration = UUID()
    private var refreshID: UUID?
    private var pendingPreparation: PendingPlayback?
    private let playbackTeardowns = PlaybackTeardowns()
    init() {
        if let reference = session?.meshSessionId {
            do {
                if try MeshPendingVault.load()?.completion?.id == reference { try MeshPendingVault.clear() }
                if try MeshRevocationVault.contains(reference) {
                    try SessionVault.clear()
                    try MeshSessionVault.clear(reference: reference)
                    session = nil
                }
            } catch {
                session = nil
                self.error = "Unlock this device to finish its saved sign-out request."
            }
        }
        if var restored = session, restored.companionGeneration == nil {
            restored.companionGeneration = UUID()
            do { try SessionVault.save(restored); session = restored }
            catch { self.error = "Watch setup could not save its session identity. Reopen Bluefin to retry." }
        }
        restoreMusicQueue()
    }

    var signedIn: Bool { session != nil || preview }
    /// Identifies this signed-in lifetime without exposing credentials to draft identities.
    var accountContext: UUID { accountGeneration }
    /// Survives process eviction, but rotates with each newly saved sign-in.
    var companionContext: UUID? { preview ? nil : session?.companionGeneration }
    func saveIrohPeer(id: String?, address: String?) throws {
        guard var updated = session, !preview else { throw BluefinError.message("Sign in before configuring your server's transport.") }
        updated.irohPeerId = id; updated.irohPeerAddress = address
        try SessionVault.save(updated)
        session = updated
        nextMusicCandidateKey = nil
        updateNextMusicCandidate()
    }
    var deviceId: String {
        if let saved = UserDefaults.standard.string(forKey: "bluefin.deviceId") { return saved }
        let value = UUID().uuidString
        UserDefaults.standard.set(value, forKey: "bluefin.deviceId")
        return value
    }

    func signIn(server: String, username: String, password: String, allowHttp: Bool) async throws {
        let result: LoginResult = try await RustBridge.call("login", fields: ["server": server, "username": username, "password": password, "deviceId": deviceId, "allowHttp": allowHttp])
        try await acceptLogin(result, server: server, allowHttp: allowHttp)
    }

    func acceptLogin(_ result: LoginResult, server: String, allowHttp: Bool) async throws {
        let newSession = AccountSession(server: server, token: result.AccessToken, userId: result.User.Id, username: result.User.Name ?? "Your account", deviceId: deviceId, allowHttp: allowHttp)
        #if os(iOS) && !targetEnvironment(macCatalyst)
        try await WatchCompanion.shared.beginAccountChange()
        defer { WatchCompanion.shared.endAccountChange() }
        #endif
        try SessionVault.save(newSession)
        #if os(tvOS)
        TopShelfSnapshot.clear()
        home = .init(resume: [], latest: [], nextUp: [])
        #endif
        accountGeneration = UUID()
        session = newSession
        preview = false
        restoreMusicQueue()
        await refresh()
    }

    func acceptMeshSession(_ mesh: MeshSavedSession, username: String) async throws {
        guard session == nil, !preview, !signingOut else {
            throw BluefinError.message("Sign out before connecting another account.")
        }
        let generation = accountGeneration
        guard try !MeshRevocationVault.contains(mesh.sessionId) else {
            throw BluefinError.message("This device connection is being signed out. Use a new invitation.")
        }
        try MeshSessionVault.save(mesh)
        var candidate = AccountSession(server: mesh.apiOrigin, token: "", userId: mesh.userId,
            username: username, deviceId: mesh.deviceId, allowHttp: false)
        candidate.meshSessionId = mesh.sessionId
        // Exercise the ordinary library entry point before making this sign-in active.
        let page: ItemPage = try await RustBridge.call("libraries", session: candidate)
        try Task.checkCancellation()
        guard session == nil, accountGeneration == generation, !signingOut else { throw CancellationError() }
        #if os(iOS) && !targetEnvironment(macCatalyst)
        try await WatchCompanion.shared.beginAccountChange()
        defer { WatchCompanion.shared.endAccountChange() }
        #endif
        try Task.checkCancellation()
        guard session == nil, !preview, accountGeneration == generation, !signingOut else { throw CancellationError() }
        try SessionVault.save(candidate)
        Artwork.clearCache()
        #if os(tvOS)
        TopShelfSnapshot.clear()
        #endif
        accountGeneration = UUID()
        session = candidate
        preview = false
        libraries = page.Items
        home = .init(resume: [], latest: [], nextUp: [])
        meshInvitationCode = nil
        restoreMusicQueue()
        await refresh()
    }

    func handleMeshScene(_ phase: ScenePhase) async {
        guard session?.meshSessionId != nil else { return }
        if phase == .active {
            if !loading { await refresh() }
        } else if phase == .background, playback == nil, !startingPlayback {
            // Background audio retains its media route. Idle routes close.
            let _: EmptyValue? = try? await RustBridge.call("meshCloseResources")
        }
    }

    func explore() {
        accountGeneration = UUID()
        preview = true
        error = nil
        libraryProblem = nil
        home = .init(resume: [], latest: MediaItem.preview, nextUp: [])
        homeFromCatalog = false
        libraries = []
    }

    func refresh() async {
        guard let session, !preview else { return }
        let generation = accountGeneration
        let request = UUID()
        refreshID = request
        loading = true
        libraryProblem = nil
        defer { if refreshID == request { loading = false } }

        func current() -> Bool {
            accountGeneration == generation && refreshID == request && !Task.isCancelled
        }

        let sections = ["homeResume", "homeLatest", "homeNextUp", "libraries"]
        var problems: [String] = []
        var homeFailed = false
        await withTaskGroup(of: (String, Result<ItemPage, Error>).self) { group in
            for section in sections {
                group.addTask {
                    do {
                        let page: ItemPage = try await RustBridge.call(section, session: session)
                        return (section, .success(page))
                    } catch {
                        return (section, .failure(error))
                    }
                }
            }
            for await (section, result) in group {
                guard current() else { group.cancelAll(); return }
                switch result {
                case .success(let page):
                    switch section {
                    case "homeResume": home.resume = page.Items
                    case "homeLatest": home.latest = page.Items; homeFromCatalog = false
                    case "homeNextUp": home.nextUp = page.Items
                    default: libraries = page.Items
                    }
                case .failure(let error):
                    if error is CancellationError { continue }
                    if section != "libraries" { homeFailed = true }
                    let title: String
                    switch section {
                    case "homeResume": title = "Continue Watching"
                    case "homeLatest": title = "Recently Added"
                    case "homeNextUp": title = "Up Next"
                    default: title = "Collections"
                    }
                    problems.append("\(title) couldn't refresh. " + error.localizedDescription)
                }
            }
        }
        if homeFailed, home.latest.isEmpty, home.resume.isEmpty, current(),
           let page = try? await items(includeItemTypes: "Movie,Series,Episode"), current() {
            home.latest = page.Items
            homeFromCatalog = true
        }
        if current() { libraryProblem = problems.isEmpty ? nil : problems.joined(separator: "\n") }
    }

    func items(parent: MediaItem? = nil, query: String = "", start: Int = 0, includeItemTypes: String? = nil) async throws -> ItemPage {
        if preview {
            let source = query.isEmpty ? MediaItem.preview : MediaItem.preview + [.previewAudio, .previewAlbum, .previewArtist]
            let kinds = includeItemTypes.map { Set($0.split(separator: ",").map(String.init)) }
            let values = source.filter { item in
                (kinds?.contains(item.Type ?? "") ?? true) &&
                (query.isEmpty || [item.title, item.Album, item.AlbumArtist, item.Artists?.joined(separator: " ")].compactMap { $0 }.contains { $0.localizedCaseInsensitiveContains(query) })
            }
            return .init(Items: start == 0 ? values : [], TotalRecordCount: values.count)
        }
        guard let session else { throw BluefinError.message("Sign in to browse your library.") }
        let generation = accountGeneration
        var fields: [String: Any] = ["startIndex": start, "limit": 60]
        if let parent { fields["parentId"] = parent.Id }
        if let includeItemTypes { fields["includeItemTypes"] = includeItemTypes }
        if !query.isEmpty { fields["searchTerm"] = query }
        let page: ItemPage = try await RustBridge.call("items", session: session, fields: fields)
        try Task.checkCancellation()
        guard accountGeneration == generation else { throw CancellationError() }
        return page
    }

    func musicItems(type: String, parent: MediaItem? = nil, start: Int = 0, query: String = "", sort: String = "SortName", descending: Bool = false, favorites: Bool = false, fromLetter: String = "") async throws -> ItemPage {
        if preview {
            let source: [MediaItem] = type == "MusicAlbum" ? [.previewAlbum] : type == "MusicArtist" ? [.previewArtist] : type == "Audio" ? [.previewAudio] : []
            let values = source.filter {
                (!favorites || $0.UserData?.IsFavorite == true) &&
                (query.isEmpty || $0.title.localizedCaseInsensitiveContains(query)) &&
                (fromLetter.isEmpty || $0.title.localizedStandardCompare(fromLetter) != .orderedAscending)
            }
            return .init(Items: start == 0 ? values : [], TotalRecordCount: values.count)
        }
        guard let session else { throw BluefinError.message("Sign in to browse music.") }
        let generation = accountGeneration
        let kinds = ["Audio": "songs", "MusicAlbum": "albums", "MusicArtist": "artists", "Playlist": "playlists"]
        var fields: [String: Any] = ["kind": kinds[type] ?? "songs", "startIndex": start, "limit": 60, "sortBy": sort, "sortOrder": descending ? "Descending" : "Ascending"]
        if !query.isEmpty { fields["searchTerm"] = query }
        if favorites { fields["isFavorite"] = true }
        if !fromLetter.isEmpty { fields["nameStartsWithOrGreater"] = fromLetter }
        if let parent {
            switch parent.Type {
            case "MusicArtist": fields["artistId"] = parent.Id
            case "MusicAlbum": fields["albumId"] = parent.Id; fields["sortBy"] = "ParentIndexNumber,IndexNumber"
            case "Playlist": fields["kind"] = "playlistItems"; fields["playlistId"] = parent.Id
            default: fields["parentId"] = parent.Id
            }
        }
        let page: ItemPage = try await RustBridge.call("musicItems", session: session, fields: fields)
        try Task.checkCancellation()
        guard accountGeneration == generation else { throw CancellationError() }
        return page
    }

    func authenticated<T: Decodable & Sendable>(_ operation: String, fields: [String: Any] = [:]) async throws -> T {
        guard let session, !preview else { throw BluefinError.message("Connect your library to use this feature.") }
        let generation = accountGeneration
        let value: T = try await RustBridge.call(operation, session: session, fields: fields)
        try Task.checkCancellation()
        guard accountGeneration == generation else { throw CancellationError() }
        return value
    }

    func lyrics(item: MediaItem) async throws -> BluefinLyrics {
        try await authenticated("lyrics", fields: ["itemId": item.Id])
    }
    func createPlaylist(name: String, items: [MediaItem] = []) async throws -> String {
        struct Created: Decodable { var Id: String }
        let result: Created = try await authenticated("createPlaylist", fields: ["name": name, "itemIds": items.map(\.Id)])
        return result.Id
    }
    func addPlaylistItems(playlist: MediaItem, items: [MediaItem]) async throws {
        let _: EmptyValue = try await authenticated("addPlaylistItems", fields: ["playlistId": playlist.Id, "itemIds": items.map(\.Id)])
    }
    func removePlaylistItems(playlist: MediaItem, entries: [MediaItem]) async throws {
        let ids = entries.compactMap(\.PlaylistItemId)
        guard ids.count == entries.count else { throw BluefinError.message("The server did not provide playlist entry IDs. Reload this playlist before removing tracks.") }
        let _: EmptyValue = try await authenticated("removePlaylistItems", fields: ["playlistId": playlist.Id, "entryIds": ids])
    }
    func renamePlaylist(_ playlist: MediaItem, name: String) async throws {
        let _: EmptyValue = try await authenticated("renamePlaylist", fields: ["playlistId": playlist.Id, "name": name])
    }
    func deletePlaylist(_ playlist: MediaItem) async throws {
        let _: EmptyValue = try await authenticated("deletePlaylist", fields: ["playlistId": playlist.Id])
    }

    func detail(_ item: MediaItem) async throws -> MediaItem {
        guard !preview, let session else { return item }
        let generation = accountGeneration
        let detail: MediaItem = try await RustBridge.call("item", session: session, fields: ["itemId": item.Id])
        guard accountGeneration == generation else { throw CancellationError() }
        return detail
    }

    func play(_ item: MediaItem, fromBeginning: Bool = false, startAtTicks: Int64? = nil, queueIndex: Int? = nil, videoOptions: VideoOptionSelection? = nil) async {
        guard !startingPlayback, !signingOut else { return }
        startingPlayback = true
        let generation = accountGeneration
        var preparation: PendingPlayback?
        defer {
            startingPlayback = false
            if pendingPreparation === preparation { pendingPreparation = nil }
        }
        do {
            try await deviceMusic.stopPlayback()?.value
            if let playback, playback.item.Type == "Audio" {
                try await playback.discardNextMusicCandidate().value
                nextMusicCandidateKey = nil
                try Task.checkCancellation()
                guard accountGeneration == generation else { throw CancellationError() }
            }
            if preview {
                // Preview is explicitly separate from a connected library and never reports watch state.
                guard let url = item.previewURL else { throw BluefinError.message("This preview is unavailable.") }
                if let playback { try await playback.stop().value }
                let controller = try PlaybackController(item: item, url: url, plan: nil, session: nil)
                playback = controller
                if item.Type == "Audio" { configureMusic(controller, item: item, queueIndex: queueIndex) }
            } else if let session, let local = offline.localURL(for: item, session: session) {
                if let playback { try await playback.stop().value }
                try Task.checkCancellation()
                guard accountGeneration == generation else { throw CancellationError() }
                let controller = try PlaybackController(item: item, url: local, plan: nil, session: nil)
                playback = controller
                configureMusic(controller, item: item, queueIndex: queueIndex)
            } else if let session {
                let ticks: Int64 = startAtTicks ?? (fromBeginning ? 0 : item.resumeTicks)
                var fields: [String: Any] = ["itemId": item.Id, "startTicks": ticks, "mediaType": item.Type == "Audio" ? "Audio" : "Video"]
                if item.Type != "Audio" {
                    // VOD seeks in the native item timeline. A served manifest's query
                    // string alone cannot prove that its media begins at that position.
                    fields["fullVideoTimeline"] = item.Type != "TvChannel"
                    fields.merge((videoOptions ?? .init()).plannerFields) { _, new in new }
                    fields["videoCapabilities"] = try VideoDeviceCapabilities.current().jsonDictionary()
                }
                func prepare(_ snapshot: [String: Any]) -> PendingPlayback {
                    PendingPlayback(request: {
                        try await RustBridge.call("playback", session: session, fields: snapshot)
                    }, cleanup: { try await RustBridge.cleanupPlaybackPlan($0, session: session) })
                }
                var pending = prepare(fields)
                preparation = pending
                pendingPreparation = pending
                var plan = try await pending.resolve()
                if plan.nativeProbeRequired == true {
                    do {
                        guard let candidate = URL(string: plan.url) else { throw BluefinError.message("The source URL is invalid.") }
                        try await NativeAssetAdmission.validate(candidate)
                    } catch is CancellationError { throw CancellationError() }
                    catch {
                        // Drain this exact candidate before one conservative retry.
                        // Keep the existing player, account, source, tracks and timeline.
                        try await pending.dispose()
                        try Task.checkCancellation()
                        guard accountGeneration == generation else { throw CancellationError() }
                        fields["videoCapabilities"] = try VideoDeviceCapabilities.conservative.jsonDictionary()
                        fields["mediaSourceId"] = plan.mediaSourceId
                        pending = prepare(fields); preparation = pending; pendingPreparation = pending
                        plan = try await pending.resolve()
                        guard plan.nativeProbeRequired != true else { throw BluefinError.message("The server did not provide a supported fallback.") }
                    }
                }
                guard accountGeneration == generation else { throw CancellationError() }
                guard let url = URL(string: plan.url) else { throw BluefinError.message("The server did not provide a playable stream.") }
                if let playback { try await playback.stop().value }
                try Task.checkCancellation()
                guard accountGeneration == generation else { throw CancellationError() }
                let controller = try PlaybackController(item: item, url: url, plan: plan, session: session)
                try pending.transfer()
                playback = controller
                controller.loadSkipSegments(for: session)
                controller.videoSelection = videoOptions ?? .init(mediaSourceID: plan.mediaSourceId, audioStreamIndex: plan.audioStreamIndex, subtitleStreamIndex: (plan.subtitleStreamIndex ?? -1) < 0 ? nil : plan.subtitleStreamIndex)
                controller.onSeekOutsideWindow = { [weak self, weak controller] position in Task { await self?.play(item, startAtTicks: position, videoOptions: controller?.videoSelection) } }
                if item.Type == "Audio" { configureMusic(controller, item: item, queueIndex: queueIndex) }
            }
            if let controller = playback, controller.item.Type != "Audio", accountGeneration == generation {
                controller.onVideoOptions = { [weak self, weak controller] selection in
                    guard let self, let controller, self.playback === controller, self.accountGeneration == generation else { return }
                    Task { await self.play(item, startAtTicks: Int64(max(0, controller.positionSeconds) * 10_000_000), videoOptions: selection) }
                }
                controller.onFinish = { [weak self, weak controller] in
                    guard let self, let controller, self.accountGeneration == generation,
                          self.playback === controller else { return }
                    self.dismissPlayback()
                }
            }
        } catch {
            var cleanupFailed = false
            do { try await preparation?.dispose() }
            catch { cleanupFailed = true }
            if accountGeneration == generation {
                if cleanupFailed { self.error = "Playback preparation cleanup could not be confirmed." }
                else if !(error is CancellationError) { self.error = error.localizedDescription }
            }
        }
    }

    private func configureMusic(_ controller: PlaybackController, item: MediaItem, queueIndex: Int?) {
        if let queueIndex, musicQueue.indices.contains(queueIndex), musicQueue[queueIndex].Id == item.Id { musicIndex = queueIndex }
        else if !musicQueue.indices.contains(musicIndex) || musicQueue[musicIndex].Id != item.Id {
            if let index = musicQueue.firstIndex(where: { $0.Id == item.Id }) { musicIndex = index }
            else { musicQueue = [item]; unshuffledQueue = musicQueue; musicIndex = 0 }
        }
        controller.onNext = { [weak self] in Task { await self?.skipMusic(1) } }
        controller.onPrevious = { [weak self] in Task { await self?.skipMusic(-1) } }
        controller.onFinish = { [weak self] in Task { await self?.musicDidFinish() } }
        controller.onReplay = { [weak self] in Task { await self?.play(item, fromBeginning: true) } }
        controller.onFavorite = { [weak self, weak controller] in
            Task {
                guard let self, let controller else { return }
                do { try await self.setFavorite(item, value: !controller.isFavorite) }
                catch { self.error = error.localizedDescription }
            }
        }
        controller.configureAudio(audio.settings)
        controller.onMusicAdvance = { [weak self, weak controller] item in
            guard let self, let controller else { return }
            let next = self.nextMusicIndex()
            self.nextMusicCandidateKey = nil
            self.configureMusic(controller, item: item, queueIndex: next)
        }
        persistMusicQueue()
    }

    private func nextMusicIndex() -> Int? {
        guard !musicQueue.isEmpty else { return nil }
        if musicRepeat == .one, musicQueue.indices.contains(musicIndex) { return musicIndex }
        let next = musicIndex + 1
        if musicQueue.indices.contains(next) { return next }
        return musicRepeat == .all ? 0 : nil
    }
    private func updateNextMusicCandidate() {
        guard let playback, playback.item.Type == "Audio", let session,
              offline.localURL(for: playback.item, session: session) == nil else { return }
        let next = nextMusicIndex()
        let key = "\(playback.id)|\(next.map(String.init) ?? "end")|\(next.map { musicQueue[$0].Id } ?? "")"
        guard nextMusicCandidateKey != key else { return }
        nextMusicCandidateKey = key
        playback.setNextMusicCandidate(next.map { musicQueue[$0] }, session: session)
    }

    func playMusic(_ tracks: [MediaItem], at index: Int = 0, shuffled: Bool = false) async {
        guard !startingPlayback else { return }
        let audio = Array(tracks.filter { $0.Type == "Audio" }.prefix(500))
        guard !audio.isEmpty else { return }
        unshuffledQueue = audio
        musicShuffled = shuffled
        musicQueue = shuffled ? audio.shuffled() : audio
        musicIndex = shuffled ? 0 : min(max(0, index), musicQueue.count - 1)
        persistMusicQueue()
        await play(musicQueue[musicIndex], fromBeginning: true, queueIndex: musicIndex)
    }

    func playServerQueue(_ items: [MediaItem], index: Int, shuffled: Bool, repeatMode: String) async throws {
        guard !startingPlayback, !items.isEmpty, items.count <= 500, items.allSatisfy({ $0.Type == "Audio" }),
              items.indices.contains(index), ["off", "all", "one"].contains(repeatMode) else {
            throw BluefinError.message("This queue could not be restored.")
        }
        let generation = accountGeneration
        startingPlayback = true
        do {
            dismissPlayback()
            try await playbackTeardowns.drain().value
            try Task.checkCancellation()
            guard accountGeneration == generation else { throw CancellationError() }
            musicQueue = items; unshuffledQueue = items; musicIndex = index; musicShuffled = shuffled
            musicRepeat = repeatMode == "one" ? .one : repeatMode == "all" ? .all : .off
            persistMusicQueue()
        } catch { startingPlayback = false; throw error }
        startingPlayback = false
        await play(items[index], fromBeginning: true, queueIndex: index)
    }

    func skipMusic(_ delta: Int) async {
        guard !startingPlayback else { return }
        if delta < 0, let playback, playback.positionSeconds > 3 { playback.seek(seconds: 0); return }
        var target = musicIndex + delta
        if musicRepeat == .all && !musicQueue.isEmpty { target = (target % musicQueue.count + musicQueue.count) % musicQueue.count }
        guard musicQueue.indices.contains(target) else { return }
        await play(musicQueue[target], fromBeginning: true, queueIndex: target)
    }

    private func musicDidFinish() async {
        if musicRepeat == .one, musicQueue.indices.contains(musicIndex) {
            await play(musicQueue[musicIndex], fromBeginning: true, queueIndex: musicIndex)
        } else { await skipMusic(1) }
    }

    func enqueueMusic(_ items: [MediaItem], next: Bool) {
        let incoming = items.filter { $0.Type == "Audio" }
        guard !incoming.isEmpty else { return }
        if next { musicQueue.insert(contentsOf: incoming, at: min(musicIndex + 1, musicQueue.count)) }
        else { musicQueue.append(contentsOf: incoming) }
        musicQueue = Array(musicQueue.prefix(500))
        unshuffledQueue = musicQueue
        persistMusicQueue()
    }

    func moveMusic(from source: IndexSet, to destination: Int) {
        guard source.allSatisfy({ musicQueue.indices.contains($0) }), (0...musicQueue.count).contains(destination) else { return }
        var order = Array(musicQueue.indices)
        order.move(fromOffsets: source, toOffset: destination)
        let oldIndex = musicIndex
        musicQueue.move(fromOffsets: source, toOffset: destination)
        musicIndex = order.firstIndex(of: oldIndex) ?? oldIndex
        unshuffledQueue = musicQueue
        persistMusicQueue()
    }

    func removeMusic(at offsets: IndexSet) {
        let valid = offsets.intersection(IndexSet(integersIn: musicQueue.indices))
        let removedBefore = valid.filter { $0 < musicIndex }.count
        let removedCurrent = valid.contains(musicIndex)
        musicQueue.remove(atOffsets: valid)
        musicIndex -= removedBefore + (removedCurrent ? 1 : 0)
        musicIndex = min(musicIndex, max(-1, musicQueue.count - 1))
        unshuffledQueue = musicQueue
        persistMusicQueue()
    }

    func clearMusicQueue() {
        musicQueue = playback.map { $0.item.Type == "Audio" ? [$0.item] : [] } ?? []
        unshuffledQueue = musicQueue
        musicIndex = 0
        persistMusicQueue()
    }

    func toggleMusicShuffle() {
        let current = playback?.item
        if musicShuffled {
            musicQueue = unshuffledQueue
            musicIndex = current.flatMap { item in musicQueue.firstIndex { $0.Id == item.Id } } ?? 0
        } else {
            unshuffledQueue = musicQueue
            var remaining = musicQueue
            if remaining.indices.contains(musicIndex), current?.Id == remaining[musicIndex].Id { remaining.remove(at: musicIndex) }
            musicQueue = current.map { [$0] + remaining.shuffled() } ?? remaining.shuffled()
            musicIndex = 0
        }
        musicShuffled.toggle()
        persistMusicQueue()
    }

    func cycleMusicRepeat() {
        switch musicRepeat { case .off: musicRepeat = .all; case .all: musicRepeat = .one; case .one: musicRepeat = .off }
        persistMusicQueue()
    }

    private struct QueueSnapshot: Codable {
        var items: [MediaItem]
        var index: Int
        var shuffled: Bool
        var repeatMode: String
        var original: [MediaItem]?
    }
    private var queueKey: String? {
        session.map { "bluefin.queue." + SHA256.hash(data: Data(($0.server + "\u{0}" + $0.userId).utf8)).map { String(format: "%02x", $0) }.joined() }
    }
    private func persistMusicQueue() {
        updateNextMusicCandidate()
        guard let key = queueKey else { return }
        let mode = musicRepeat == .one ? "one" : musicRepeat == .all ? "all" : "off"
        let snapshot = QueueSnapshot(items: Array(musicQueue.prefix(500)), index: musicIndex, shuffled: musicShuffled, repeatMode: mode, original: Array(unshuffledQueue.prefix(500)))
        guard let data = try? JSONEncoder().encode(snapshot), data.count <= 1_048_576 else { return }
        UserDefaults.standard.set(data, forKey: key)
    }
    private func restoreMusicQueue() {
        musicQueue = []; musicIndex = 0; musicShuffled = false; musicRepeat = .off; unshuffledQueue = []
        guard let key = queueKey, let data = UserDefaults.standard.data(forKey: key), data.count <= 1_048_576,
              let saved = try? JSONDecoder().decode(QueueSnapshot.self, from: data) else { return }
        musicQueue = Array(saved.items.filter { $0.Type == "Audio" }.prefix(500))
        musicIndex = min(max(0, saved.index), max(0, musicQueue.count - 1))
        musicShuffled = saved.shuffled
        musicRepeat = saved.repeatMode == "one" ? .one : saved.repeatMode == "all" ? .all : .off
        unshuffledQueue = Array((saved.original ?? musicQueue).filter { $0.Type == "Audio" }.prefix(500))
    }

    func setFavorite(_ item: MediaItem, value: Bool) async throws {
        guard let session, !preview else { throw BluefinError.message("Connect your library to save favorites.") }
        let generation = accountGeneration
        try await RustBridge.notify("favorite", session: session, fields: ["itemId": item.Id, "favorite": value])
        guard accountGeneration == generation else { throw CancellationError() }
        func update(_ entries: [MediaItem]) -> [MediaItem] {
            entries.map { existing in
                guard existing.Id == item.Id else { return existing }
                var changed = existing
                if changed.UserData == nil { changed.UserData = UserData() }
                changed.UserData?.IsFavorite = value
                return changed
            }
        }
        musicQueue = update(musicQueue)
        unshuffledQueue = update(unshuffledQueue)
        if playback?.item.Id == item.Id { playback?.updateFavorite(value) }
        persistMusicQueue()
        home.resume = update(home.resume)
        home.latest = update(home.latest)
        home.nextUp = update(home.nextUp)
    }

    /// One native audio owner at a time. The local catalog remains independent
    /// of this store's account; this gate only serializes player replacement.
    func withDeviceMusicPlayback(_ operation: @MainActor () async throws -> Void) async throws {
        guard !startingPlayback, !signingOut else { throw BluefinError.message("Wait for playback to finish changing.") }
        startingPlayback = true
        defer { startingPlayback = false }
        let generation = accountGeneration
        let pending = pendingPreparation
        pending?.cancel()
        dismissPlayback()
        let teardown = playbackTeardowns.drain()
        try await pending?.dispose()
        try await teardown.value
        try Task.checkCancellation()
        guard accountGeneration == generation, !signingOut else { throw CancellationError() }
        try await operation()
        guard accountGeneration == generation, !signingOut else {
            deviceMusic.stopPlayback()
            throw CancellationError()
        }
    }

    func dismissPlayback() {
        guard let playback else { return }
        let generation = accountGeneration
        playbackTeardowns.retain(playback.stop()) { [weak self] in
            guard let self, self.accountGeneration == generation else { return }
            self.error = "Playback cleanup could not be confirmed. Check your connection."
        }
        self.playback = nil
        nextMusicCandidateKey = nil
    }

    func signOut() async {
        guard !signingOut else { return }
        signingOut = true
        defer { signingOut = false }
        let previousSession = session
        let deviceTeardown = deviceMusic.stopPlayback()

        // Stop local work before any Keychain or network operation can fail.
        accountGeneration = UUID()
        let generation = accountGeneration
        let pending = pendingPreparation
        pending?.cancel()
        dismissPlayback()
        let teardown = playbackTeardowns.drain()
        offline.disconnect()
        sports.disconnect()
        Artwork.clearCache()
        #if os(tvOS)
        TopShelfSnapshot.clear()
        #endif
        home = .init(resume: [], latest: [], nextUp: [])
        libraries = []
        musicQueue = []
        unshuffledQueue = []
        musicIndex = 0
        musicShuffled = false
        musicRepeat = .off
        musicExpanded = false
        error = nil
        var cleanupFailed = false
        do { try await deviceTeardown?.value } catch { cleanupFailed = true }
        do { try await pending?.dispose() } catch { cleanupFailed = true }
        do { try await teardown.value } catch { cleanupFailed = true }
        if let previousSession, previousSession.meshSessionId != nil {
            // Local shutdown needs no device key. It must work while Keychain is
            // locked or while persisting a remote-revocation receipt fails.
            do {
                let _: EmptyValue = try await RustBridge.call("meshLocalSignOut", fields: [
                    "server": previousSession.server, "userId": previousSession.userId,
                    "allowLocalHttp": previousSession.allowHttp])
            } catch { cleanupFailed = true }
        } else {
            let _: EmptyValue? = try? await RustBridge.call("meshCloseResources")
        }

        #if os(iOS) && !targetEnvironment(macCatalyst)
        var watchChange = false
        do { try await WatchCompanion.shared.beginAccountChange(); watchChange = true }
        catch { cleanupFailed = true }
        defer { if watchChange { WatchCompanion.shared.endAccountChange() } }
        #endif
        do {
            if let previousSession { try await MeshRevocationQueue.shared.prepare(previousSession) }
            try SessionVault.clear()
        } catch {
            // Keep cleanup credentials if they could not be durably transferred.
            // The active media route has already been stopped above.
            self.error = "Playback stopped, but sign-out could not be saved. Unlock this device and retry. Remote access is unconfirmed."
            return
        }
        session = nil
        preview = false
        if let previousSession {
            if let reference = previousSession.meshSessionId {
                do {
                    if try MeshPendingVault.load()?.completion?.id == reference { try MeshPendingVault.clear() }
                    try MeshSessionVault.clear(reference: reference)
                } catch { cleanupFailed = true }
                await MeshRevocationQueue.shared.retry()
                if MeshRevocationQueue.shared.count > 0 { cleanupFailed = true }
            } else {
                do { try await RustBridge.notify("logout", session: previousSession) }
                catch { cleanupFailed = true }
            }
        }
        if cleanupFailed && accountGeneration == generation {
            error = "Signed out on this device. Some remote cleanup is unconfirmed and will retry when Bluefin reconnects."
        }
    }
}
