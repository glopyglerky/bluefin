import Foundation
import SwiftUI

@MainActor final class LibraryStore: ObservableObject {
    @Published var session = SessionVault.load()
    @Published var preview = false
    @Published var home = HomePage(resume: [], latest: [], nextUp: [])
    @Published var libraries: [MediaItem] = []
    @Published var loading = false
    @Published var error: String?
    @Published var playback: PlaybackController?
    @Published var startingPlayback = false
    @Published var musicExpanded = false
    @Published var musicQueue: [MediaItem] = []
    @Published var musicIndex = 0
    private var accountGeneration = UUID()
    var signedIn: Bool { session != nil || preview }
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
        try SessionVault.save(newSession)
        accountGeneration = UUID()
        session = newSession
        preview = false
        await refresh()
    }

    func explore() {
        accountGeneration = UUID()
        preview = true
        error = nil
        home = .init(resume: [], latest: MediaItem.preview, nextUp: [])
        libraries = []
    }

    func refresh() async {
        guard let session, !preview else { return }
        let generation = accountGeneration
        loading = true
        defer { loading = false }
        do {
            async let homeRequest: HomePage = RustBridge.call("home", session: session)
            async let libraryRequest: ItemPage = RustBridge.call("libraries", session: session)
            let (newHome, newLibraries) = try await (homeRequest, libraryRequest)
            try Task.checkCancellation()
            guard accountGeneration == generation else { return }
            home = newHome
            libraries = newLibraries.Items
            error = nil
        } catch is CancellationError {} catch { if accountGeneration == generation { self.error = error.localizedDescription } }
    }

    func items(parent: MediaItem? = nil, query: String = "", start: Int = 0) async throws -> ItemPage {
        if preview { return .init(Items: MediaItem.preview.filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) }, TotalRecordCount: 3) }
        guard let session else { throw BluefinError.message("Sign in to browse your library.") }
        let generation = accountGeneration
        var fields: [String: Any] = ["startIndex": start, "limit": 60]
        if let parent { fields["parentId"] = parent.Id }
        if !query.isEmpty { fields["searchTerm"] = query }
        let page: ItemPage = try await RustBridge.call("items", session: session, fields: fields)
        guard accountGeneration == generation else { throw CancellationError() }
        return page
    }

    func detail(_ item: MediaItem) async throws -> MediaItem {
        guard !preview, let session else { return item }
        let generation = accountGeneration
        let detail: MediaItem = try await RustBridge.call("item", session: session, fields: ["itemId": item.Id])
        guard accountGeneration == generation else { throw CancellationError() }
        return detail
    }

    func play(_ item: MediaItem, fromBeginning: Bool = false, startAtTicks: Int64? = nil) async {
        guard !startingPlayback else { return }
        startingPlayback = true
        let generation = accountGeneration
        var pendingProxy: String?
        defer { startingPlayback = false }
        do {
            if preview {
                // Preview is explicitly separate from a connected library and never reports watch state.
                let file = item.Id == "preview-2" ? "Sintel.mp4" : item.Id == "preview-3" ? "TearsOfSteel.mp4" : "BigBuckBunny.mp4"
                let url = URL(string: "https://storage.googleapis.com/gtv-videos-bucket/sample/\(file)")!
                let controller = try PlaybackController(item: item, url: url, plan: nil, session: nil)
                playback?.stop()
                playback = controller
            } else if let session {
                let ticks: Int64 = startAtTicks ?? (fromBeginning ? 0 : item.UserData?.PlaybackPositionTicks ?? 0)
                let plan: PlaybackPlan = try await RustBridge.call("playback", session: session, fields: ["itemId": item.Id, "startTicks": ticks, "mediaType": item.Type == "Audio" ? "Audio" : "Video"])
                pendingProxy = plan.proxyId
                guard accountGeneration == generation else { throw CancellationError() }
                guard let url = URL(string: plan.url) else { throw BluefinError.message("The server did not provide a playable stream.") }
                let controller = try PlaybackController(item: item, url: url, plan: plan, session: session)
                playback?.stop()
                playback = controller
                pendingProxy = nil
                controller.onSeekOutsideWindow = { [weak self] position in Task { await self?.play(item, startAtTicks: position) } }
                if item.Type == "Audio" {
                    if let index = musicQueue.firstIndex(where: { $0.Id == item.Id }) { musicIndex = index }
                    else { musicQueue = [item]; musicIndex = 0 }
                    controller.onNext = { [weak self] in Task { await self?.skipMusic(1) } }
                    controller.onPrevious = { [weak self] in Task { await self?.skipMusic(-1) } }
                    controller.onFinish = { [weak self] in Task { await self?.skipMusic(1) } }
                }
            }
        } catch {
            if let pendingProxy { let _: EmptyValue? = try? await RustBridge.call("stopPlaybackProxy", fields: ["proxyId": pendingProxy]) }
            if !(error is CancellationError) && accountGeneration == generation { self.error = error.localizedDescription }
        }
    }

    func playMusic(_ tracks: [MediaItem], at index: Int = 0, shuffled: Bool = false) async {
        let audio = tracks.filter { $0.Type == "Audio" }
        guard !audio.isEmpty else { return }
        musicQueue = shuffled ? audio.shuffled() : audio
        musicIndex = min(max(0, index), musicQueue.count - 1)
        await play(musicQueue[musicIndex])
    }

    func skipMusic(_ delta: Int) async {
        let target = musicIndex + delta
        guard musicQueue.indices.contains(target) else { return }
        await play(musicQueue[target])
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
        home.resume = update(home.resume)
        home.latest = update(home.latest)
        home.nextUp = update(home.nextUp)
    }

    func signOut() async {
        accountGeneration = UUID()
        let previousSession = session
        playback?.stop()
        playback = nil
        SessionVault.clear()
        session = nil
        preview = false
        home = .init(resume: [], latest: [], nextUp: [])
        libraries = []
        musicQueue = []
        musicExpanded = false
        error = nil
        if let previousSession { try? await RustBridge.notify("logout", session: previousSession) }
    }
}
