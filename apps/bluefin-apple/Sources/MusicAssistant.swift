import SwiftUI
import Security

struct MusicAssistantCapabilities: Codable, Sendable {
    let search: Bool
    let library: Bool
    let players: Bool
    let queue: Bool
    let play: Bool
    let resume: Bool
    let pause: Bool
    let next: Bool
    let previous: Bool
    let stop: Bool
}

struct MusicAssistantInfo: Codable, Sendable {
    let server: String
    let serverId: String
    let userId: String
    let name: String
    let username: String
    let version: String
    let schemaVersion: UInt64?
    let capabilities: MusicAssistantCapabilities
}

private struct MusicAssistantConnection: Codable, Sendable {
    let owner: String
    let info: MusicAssistantInfo
    let token: String
    let allowHttp: Bool
    var fields: [String: Any] {
        ["server": info.server, "token": token, "allowLocalHttp": allowHttp,
         "expectedServerId": info.serverId, "expectedUserId": info.userId]
    }
}

struct MusicAssistantTrack: Decodable, Identifiable, Sendable {
    let id: String
    let uri: String
    let title: String
    let artist: String
    let album: String
    let duration: Double?
    let available: Bool
    let provider: String
}

struct MusicAssistantTrackPage: Decodable, Sendable {
    let items: [MusicAssistantTrack]
    let nextOffset: UInt64
    let hasMore: Bool
}

struct MusicAssistantPlayer: Decodable, Identifiable, Sendable {
    let id: String
    let name: String
    let available: Bool
    let state: String
    let grouped: Bool
    let volume: UInt64?
}

struct MusicAssistantQueueItem: Decodable, Identifiable, Sendable {
    let id: String
    let title: String
    let artist: String
    let current: Bool
}

struct MusicAssistantQueue: Decodable, Sendable {
    let id: String?
    let name: String
    let state: String
    let items: [MusicAssistantQueueItem]
    let total: UInt64?
    let truncated: Bool?
}

@MainActor final class MusicAssistantModel: ObservableObject {
    @Published var server = ""
    @Published var token = ""
    @Published var allowHttp = false
    @Published var searchText = ""
    @Published private(set) var info: MusicAssistantInfo?
    @Published private(set) var players: [MusicAssistantPlayer] = []
    @Published private(set) var tracks: [MusicAssistantTrack] = []
    @Published private(set) var queue: MusicAssistantQueue?
    @Published private(set) var selectedPlayerId: String?
    @Published private(set) var ready = false
    @Published private(set) var busy = false
    @Published private(set) var accountAvailable = false
    @Published private(set) var problem: String?
    @Published private(set) var status: String?
    @Published private(set) var isSearch = false
    @Published private(set) var offset: UInt64 = 0
    @Published private(set) var hasMore = false
    private var nextOffset: UInt64 = 0
    private var connection: MusicAssistantConnection?
    private weak var store: LibraryStore?
    private var owner: String?
    private var accountGeneration: UUID?
    private var requestID: UUID?
    private var pending: Task<Void, Never>?

    private struct Request {
        let id: UUID
        let owner: String
        let generation: UUID
    }

    var connected: Bool { connection != nil }
    var selectedPlayer: MusicAssistantPlayer? {
        players.first { $0.id == selectedPlayerId && $0.available }
    }
    var canPlay: Bool { ready && !busy && selectedPlayer != nil && info?.capabilities.play == true }

    func attach(to store: LibraryStore) {
        detach()
        self.store = store
        accountGeneration = store.accountContext
        guard let session = store.session, !store.preview else { return }
        let owner = NativeIntegrations.identity(session)
        self.owner = owner
        accountAvailable = true
        do {
            if let saved = try MusicAssistantVault.load(owner: owner) {
                connection = saved
                info = saved.info
                server = saved.info.server
                allowHttp = saved.allowHttp
                reconnect()
            }
        } catch { problem = error.localizedDescription }
    }

    func detach() {
        pending?.cancel()
        pending = nil
        requestID = nil
        store = nil
        accountGeneration = nil
        owner = nil
        connection = nil
        info = nil
        players = []
        tracks = []
        queue = nil
        selectedPlayerId = nil
        token = ""
        server = ""
        searchText = ""
        allowHttp = false
        ready = false
        busy = false
        accountAvailable = false
        problem = nil
        status = nil
        offset = 0
        nextOffset = 0
        hasMore = false
        isSearch = false
    }

    private func check(_ request: Request) throws {
        guard !Task.isCancelled, requestID == request.id, owner == request.owner,
              accountGeneration == request.generation, let store,
              store.accountContext == request.generation, !store.preview,
              store.session.map(NativeIntegrations.identity) == request.owner else {
            throw CancellationError()
        }
    }

    private func run(_ body: @escaping @MainActor (Request) async throws -> Void) {
        guard !busy, pending == nil, let owner, let generation = accountGeneration,
              let store, !store.preview, store.accountContext == generation,
              store.session.map(NativeIntegrations.identity) == owner else { return }
        let request = Request(id: UUID(), owner: owner, generation: generation)
        requestID = request.id
        busy = true
        problem = nil
        status = nil
        pending = Task { [weak self] in
            guard let self else { return }
            defer {
                if self.requestID == request.id {
                    self.requestID = nil
                    self.pending = nil
                    self.busy = false
                }
            }
            do {
                try self.check(request)
                try await body(request)
            } catch is CancellationError {
                // Account changes and view closure discard all late responses.
            } catch {
                do { try self.check(request) } catch { return }
                self.problem = error.localizedDescription
            }
        }
    }

    private func call<T: Decodable & Sendable>(_ operation: String, _ request: Request,
                                               fields: [String: Any] = [:]) async throws -> T {
        try check(request)
        guard let connection, ready else {
            throw BluefinError.message("Reconnect Music Assistant before using this action.")
        }
        var input = connection.fields
        for (key, value) in fields { input[key] = value }
        // Never pass the Bluefin session: these are separate service credentials.
        let result: T = try await RustBridge.call(operation, fields: input)
        try check(request)
        return result
    }

    func connect() {
        let endpoint = server.trimmingCharacters(in: .whitespacesAndNewlines)
        let secret = token.trimmingCharacters(in: .whitespacesAndNewlines)
        let http = allowHttp
        guard !endpoint.isEmpty, !secret.isEmpty else { return }
        run { [weak self] request in
            guard let self else { return }
            defer { if self.requestID == request.id { self.token = "" } }
            let result: MusicAssistantInfo = try await RustBridge.call("musicAssistantConnect",
                fields: ["server": endpoint, "token": secret, "allowLocalHttp": http])
            try self.check(request)
            let saved = MusicAssistantConnection(owner: request.owner, info: result, token: secret, allowHttp: http)
            try MusicAssistantVault.save(saved)
            self.connection = saved
            self.info = result
            self.server = result.server
            self.ready = true
            try await self.loadPlayers(request)
            if result.capabilities.library { try await self.loadLibrary(request, offset: 0) }
        }
    }

    func reconnect() {
        guard !busy, let saved = connection else { return }
        ready = false
        players = []
        tracks = []
        queue = nil
        selectedPlayerId = nil
        run { [weak self] request in
            guard let self else { return }
            let result: MusicAssistantInfo = try await RustBridge.call("musicAssistantConnect", fields: saved.fields)
            try self.check(request)
            let updated = MusicAssistantConnection(owner: request.owner, info: result, token: saved.token, allowHttp: saved.allowHttp)
            try MusicAssistantVault.save(updated)
            self.connection = updated
            self.info = result
            self.ready = true
            try await self.loadPlayers(request)
            if result.capabilities.library { try await self.loadLibrary(request, offset: 0) }
        }
    }

    func disconnect() {
        guard !busy, let owner else { return }
        ready = false
        do {
            try MusicAssistantVault.clear(owner: owner)
            connection = nil
            info = nil
            players = []
            tracks = []
            queue = nil
            selectedPlayerId = nil
            token = ""
            problem = nil
            status = nil
            hasMore = false
        } catch { problem = error.localizedDescription }
    }

    private func loadPlayers(_ request: Request) async throws {
        let result: [MusicAssistantPlayer] = try await call("musicAssistantPlayers", request)
        players = result
        if let selectedPlayerId, !result.contains(where: { $0.id == selectedPlayerId && $0.available }) {
            self.selectedPlayerId = nil
            queue = nil
        }
        if selectedPlayerId == nil {
            let available = result.filter(\.available)
            if available.count == 1 { selectedPlayerId = available[0].id }
        }
        if selectedPlayerId != nil, info?.capabilities.queue == true { try await loadQueue(request) }
    }

    func refreshPlayers() {
        run { [weak self] request in
            guard let self else { return }
            try await self.loadPlayers(request)
        }
    }

    func selectPlayer(_ id: String?) {
        guard !busy else { return }
        selectedPlayerId = id
        queue = nil
        if selectedPlayer != nil, info?.capabilities.queue == true { refreshQueue() }
    }

    private func loadLibrary(_ request: Request, offset: UInt64) async throws {
        let page: MusicAssistantTrackPage = try await call("musicAssistantLibrary", request,
            fields: ["limit": 25, "offset": offset])
        tracks = page.items
        self.offset = offset
        nextOffset = page.nextOffset
        hasMore = page.hasMore
        isSearch = false
    }

    func libraryPage(_ offset: UInt64 = 0) {
        guard info?.capabilities.library == true else { return }
        run { [weak self] request in
            guard let self else { return }
            try await self.loadLibrary(request, offset: offset)
        }
    }
    func nextPage() { if hasMore { libraryPage(nextOffset) } }
    func previousPage() { libraryPage(offset > 25 ? offset - 25 : 0) }

    func search() {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty, info?.capabilities.search == true else { return }
        run { [weak self] request in
            guard let self else { return }
            let page: MusicAssistantTrackPage = try await self.call("musicAssistantSearch", request,
                fields: ["query": query, "limit": 25])
            self.tracks = page.items
            self.offset = 0
            self.nextOffset = 0
            self.hasMore = false
            self.isSearch = true
        }
    }

    private func loadQueue(_ request: Request) async throws {
        guard let player = selectedPlayer else { queue = nil; return }
        let result: MusicAssistantQueue = try await call("musicAssistantQueue", request, fields: ["playerId": player.id])
        queue = result
    }

    func refreshQueue() {
        guard info?.capabilities.queue == true else { return }
        run { [weak self] request in
            guard let self else { return }
            try await self.loadQueue(request)
        }
    }

    func play(_ track: MusicAssistantTrack, mode: String) {
        guard canPlay, track.available, tracks.contains(where: { $0.id == track.id }),
              let player = selectedPlayer, ["play", "add"].contains(mode) else { return }
        run { [weak self] request in
            guard let self else { return }
            let _: EmptyValue = try await self.call("musicAssistantPlay", request,
                fields: ["playerId": player.id, "uri": track.uri, "mode": mode])
            self.status = mode == "add" ? "Add requested on \(player.name)." : "Playback requested on \(player.name)."
            if self.info?.capabilities.queue == true { try await self.loadQueue(request) }
        }
    }

    func control(_ action: String) {
        guard ready, let queueID = queue?.id else { return }
        run { [weak self] request in
            guard let self else { return }
            let _: EmptyValue = try await self.call("musicAssistantControl", request,
                fields: ["queueId": queueID, "action": action])
            try await self.loadQueue(request)
        }
    }
}

struct MusicAssistantView: View {
    @EnvironmentObject private var store: LibraryStore
    @StateObject private var model = MusicAssistantModel()

    var body: some View {
        Form {
            if !model.accountAvailable {
                Section { Text("Sign in to your Bluefin account to save its Music Assistant connection.") }
            } else if !model.connected {
                Section("Connect Music Assistant") {
                    TextField("https://music.example.com", text: $model.server).bluefinAddressInput()
                    SecureField("Music Assistant access token", text: $model.token)
                    if model.server.lowercased().hasPrefix("http:") {
                        Toggle("Allow trusted local HTTP", isOn: $model.allowHttp)
                    }
                    Text("Create an access token in your Music Assistant profile. Bluefin keeps this connection in this device's Keychain for the current Bluefin account.")
                        .font(.footnote).foregroundStyle(.secondary)
                    Button("Connect") { model.connect() }
                        .disabled(model.busy || model.server.isEmpty || model.token.isEmpty)
                }
            } else {
                Section("Connection") {
                    if let info = model.info {
                        LabeledContentCompat("Account", value: info.username)
                        LabeledContentCompat("Server", value: info.name)
                    }
                    if !model.ready { Button("Reconnect") { model.reconnect() }.disabled(model.busy) }
                    Button("Disconnect Music Assistant", role: .destructive) { model.disconnect() }.disabled(model.busy)
                }
                if model.ready {
                    players
                    library
                    if model.info?.capabilities.queue == true { queue }
                }
            }
            if model.busy { Section { ProgressView("Contacting Music Assistant") } }
            if let status = model.status { Section { Text(status).foregroundStyle(.secondary) } }
            if let problem = model.problem { Section { Text(problem).foregroundColor(.red) } }
        }
        .navigationTitle("Music Assistant")
        .task(id: store.accountContext) { model.attach(to: store) }
        .onDisappear { model.detach() }
    }

    private var players: some View {
        Section("Play on") {
            Picker("Player", selection: Binding(get: { model.selectedPlayerId }, set: { model.selectPlayer($0) })) {
                Text("Select a player").tag(nil as String?)
                ForEach(model.players.filter(\.available)) { player in
                    Text(player.name).tag(Optional(player.id))
                }
            }.disabled(model.busy)
            if model.players.filter(\.available).isEmpty {
                Text("No available players are visible to this Music Assistant account.").foregroundStyle(.secondary)
            }
            if model.selectedPlayer?.grouped == true {
                Text("This player belongs to a Music Assistant group. Playback may use its grouped speakers.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            Button("Refresh players") { model.refreshPlayers() }.disabled(model.busy)
        }
    }

    private var library: some View {
        Section(model.isSearch ? "Search results" : "Music Assistant library") {
            if model.info?.capabilities.search == true {
                TextField("Search your Music Assistant library", text: $model.searchText)
                    .onSubmit { model.search() }
                HStack {
                    Button("Search") { model.search() }
                        .disabled(model.busy || model.searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    if model.info?.capabilities.library == true {
                        Button("Show library") { model.libraryPage() }.disabled(model.busy)
                    }
                }
            }
            ForEach(model.tracks) { track in
                VStack(alignment: .leading, spacing: 8) {
                    Text(track.title).font(.headline)
                    if !track.artist.isEmpty { Text(track.artist).font(.subheadline).foregroundStyle(.secondary) }
                    HStack {
                        Button("Play") { model.play(track, mode: "play") }
                        Button("Add to queue") { model.play(track, mode: "add") }
                    }.disabled(!model.canPlay || !track.available)
                    if !track.available { Text("Unavailable in Music Assistant").font(.caption).foregroundStyle(.secondary) }
                }.padding(.vertical, 4)
            }
            if model.tracks.isEmpty && !model.busy {
                Text(model.isSearch ? "No matching tracks in this account's library." : "No tracks on this page.")
                    .foregroundStyle(.secondary)
            }
            if !model.isSearch {
                HStack {
                    Button("Previous page") { model.previousPage() }.disabled(model.busy || model.offset == 0)
                    Spacer()
                    Button("Next page") { model.nextPage() }.disabled(model.busy || !model.hasMore)
                }
            }
            Text("Music Assistant checks this account's sources and plays on the selected player. This connection does not download or share streaming-service audio.")
                .font(.footnote).foregroundStyle(.secondary)
        }
    }

    private var queue: some View {
        Section("Queue") {
            if let queue = model.queue, queue.id != nil {
                HStack(spacing: 20) {
                    if model.info?.capabilities.previous == true {
                        Button { model.control("previous") } label: { Image(systemName: "backward.end.fill") }
                            .accessibilityLabel("Previous track")
                    }
                    if queue.state == "playing", model.info?.capabilities.pause == true {
                        Button { model.control("pause") } label: { Image(systemName: "pause.fill") }.accessibilityLabel("Pause")
                    } else if model.info?.capabilities.resume == true {
                        Button { model.control("play") } label: { Image(systemName: "play.fill") }.accessibilityLabel("Resume")
                    }
                    if model.info?.capabilities.next == true {
                        Button { model.control("next") } label: { Image(systemName: "forward.end.fill") }.accessibilityLabel("Next track")
                    }
                    if model.info?.capabilities.stop == true {
                        Button { model.control("stop") } label: { Image(systemName: "stop.fill") }.accessibilityLabel("Stop")
                    }
                }.buttonStyle(.borderless).disabled(model.busy)
                ForEach(queue.items) { item in
                    HStack {
                        if item.current { Image(systemName: "speaker.wave.2.fill").accessibilityLabel("Current track") }
                        VStack(alignment: .leading) {
                            Text(item.title)
                            if !item.artist.isEmpty { Text(item.artist).font(.caption).foregroundStyle(.secondary) }
                        }
                    }
                }
                if queue.truncated == true { Text("Showing the first 50 queued items.").font(.footnote).foregroundStyle(.secondary) }
            } else {
                Text(model.selectedPlayer == nil ? "Select a player to see its queue." : "This player has no active Music Assistant queue.")
                    .foregroundStyle(.secondary)
            }
            Button("Refresh queue") { model.refreshQueue() }.disabled(model.busy || model.selectedPlayer == nil)
        }
    }
}

private enum MusicAssistantVault {
    private static let service = "org.glopyglerky.bluefin.music-assistant"
    private static func query(owner: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
         kSecAttrAccount as String: owner, kSecAttrSynchronizable as String: false]
    }

    static func load(owner: String) throws -> MusicAssistantConnection? {
        var request = query(owner: owner)
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else {
            throw BluefinError.message("Unlock this device to read its saved Music Assistant connection.")
        }
        let connection = try JSONDecoder().decode(MusicAssistantConnection.self, from: data)
        guard connection.owner == owner else { throw BluefinError.message("The saved Music Assistant connection belongs to another account.") }
        return connection
    }

    static func save(_ connection: MusicAssistantConnection) throws {
        let data = try JSONEncoder().encode(connection)
        let key = query(owner: connection.owner)
        let attributes = [kSecValueData as String: data,
                          kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly] as [String: Any]
        let status = SecItemUpdate(key as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var request = key
            for (key, value) in attributes { request[key] = value }
            guard SecItemAdd(request as CFDictionary, nil) == errSecSuccess else {
                throw BluefinError.message("Music Assistant could not be saved in this device's Keychain.")
            }
        } else if status != errSecSuccess {
            throw BluefinError.message("Keychain could not update this Music Assistant connection.")
        }
    }

    static func clear(owner: String) throws {
        let status = SecItemDelete(query(owner: owner) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw BluefinError.message("The saved Music Assistant connection could not be removed. Unlock and retry.")
        }
    }
}
