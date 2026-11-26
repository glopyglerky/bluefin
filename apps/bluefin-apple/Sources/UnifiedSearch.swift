import SwiftUI

@MainActor final class UnifiedSearchModel: ObservableObject {
    @Published private(set) var items: [MediaItem] = []
    @Published private(set) var artists: [MediaItem] = []
    @Published private(set) var channels: [MediaItem] = []
    @Published private(set) var total = 0
    @Published private(set) var loading = false
    @Published private(set) var loadedContext: UUID?
    @Published private(set) var failures: [String] = []
    private var generation = UUID()
    private var channelCache: (account: String, date: Date, values: [MediaItem])?
    private enum Part { case library(ItemPage), artists(ItemPage), guide([MediaItem]), failure(String) }

    func load(query: String, store: LibraryStore) async {
        let current = UUID(); generation = current
        let accountContext = store.accountContext
        loadedContext = nil
        let account = store.session.map(OfflineLibrary.identity)
        let cached = channelCache.flatMap { $0.account == account && Date().timeIntervalSince($0.date) < 60 ? $0.values : nil }
        if channelCache?.account != account { channelCache = nil }
        items = []; artists = []; channels = []; failures = []; total = 0; loading = true
        defer { if generation == current { loading = false } }
        await withTaskGroup(of: Part.self) { group in
            group.addTask {
                do { return .library(try await store.items(query: query)) }
                catch is CancellationError { return .failure("") }
                catch { return .failure("Library: " + error.localizedDescription) }
            }
            group.addTask {
                do { return .artists(try await store.musicItems(type: "MusicArtist", query: query)) }
                catch is CancellationError { return .failure("") }
                catch { return .failure("Artists: " + error.localizedDescription) }
            }
            if let session = store.session {
                group.addTask {
                    do {
                        if let cached { return .guide(cached) }
                        var values: [MediaItem] = []; var start = 0
                        // Bounded pages avoid one huge guide response. Do not silently
                        // label a partial guide complete if the server exceeds the bound.
                        while start < 10_000 {
                            try Task.checkCancellation()
                            let page: ItemPage = try await RustBridge.call("liveChannels", session: session, fields: ["startIndex": start, "limit": 200])
                            values += page.Items; start += page.Items.count
                            if page.Items.isEmpty || start >= (page.TotalRecordCount ?? start) { return .guide(values) }
                        }
                        return .failure("The channel guide is larger than this search can load. Open Live TV to browse all channels.")
                    } catch is CancellationError { return .failure("") }
                    catch { return .failure("Live guide: " + error.localizedDescription) }
                }
            }
            for await part in group {
                guard generation == current, !Task.isCancelled, store.accountContext == accountContext, store.session.map(OfflineLibrary.identity) == account else { group.cancelAll(); continue }
                loadedContext = accountContext
                switch part {
                case .library(let page): items = page.Items; total = page.TotalRecordCount ?? page.Items.count
                case .artists(let page): artists = page.Items
                case .guide(let values):
                    if let account { channelCache = (account, Date(), values) }
                    channels = values.filter {
                        [$0.title, $0.Number, $0.CurrentProgram?.Name, $0.CurrentProgram?.EpisodeTitle]
                            .compactMap { $0 }.contains { $0.localizedCaseInsensitiveContains(query) }
                    }
                case .failure(let message): if !message.isEmpty { failures.append(message) }
                }
            }
        }
    }
    func more(query: String, store: LibraryStore) async {
        guard !loading else { return }
        let current = generation; let account = store.session.map(OfflineLibrary.identity)
        loading = true
        defer { if current == generation { loading = false } }
        do {
            let page = try await store.items(query: query, start: items.count)
            try Task.checkCancellation()
            guard current == generation, store.session.map(OfflineLibrary.identity) == account else { return }
            let existing = Set(items.map(\.Id)); items += page.Items.filter { !existing.contains($0.Id) }
            total = page.TotalRecordCount ?? items.count
        } catch is CancellationError {} catch { if current == generation { failures.append(error.localizedDescription) } }
    }
}

struct UnifiedSearchResults: View {
    let query: String
    @EnvironmentObject private var store: LibraryStore
    @StateObject private var model = UnifiedSearchModel()
    private var identity: String { query + "|" + (store.session.map(OfflineLibrary.identity) ?? "preview") }
    var body: some View {
        List {
            ServerDiscoverySection(query: query)
            LocalAssistanceSearchSection(query: query, candidates: model.items + model.artists,
                loadedContext: model.loadedContext)
                .id(query + "|" + (model.loadedContext?.uuidString ?? "loading"))
            if !model.items.isEmpty {
                Section("Your library") {
                    ForEach(model.items) { item in result(item) }
                    if model.items.count < model.total { Button("More library results") { Task { await model.more(query: query, store: store) } }.disabled(model.loading) }
                }
            }
            if !model.artists.isEmpty {
                Section("Artists") {
                    ForEach(model.artists.filter { artist in !model.items.contains { $0.Id == artist.Id } }) { artist in
                        MediaNavigationLink(destination: .musicCollection(artist, type: "MusicAlbum")) { MusicRow(item: artist) }
                    }
                }
            }
            if !model.channels.isEmpty {
                Section("Channels and current programmes") {
                    ForEach(model.channels) { channel in
                        MediaNavigationLink(destination: .channel(channel)) {
                            VStack(alignment: .leading, spacing: 5) {
                                Text(channel.CurrentProgram?.title ?? channel.title).font(.headline)
                                Text([channel.Number, channel.title].compactMap { $0 }.joined(separator: " · ")).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            if model.loading { ProgressView("Searching library and guide") }
            ForEach(model.failures, id: \.self) { Text($0).font(.callout).foregroundStyle(.secondary) }
            if !model.loading && model.items.isEmpty && model.artists.isEmpty && model.channels.isEmpty {
                Text("No results in your authorized library or current channel guide.").foregroundStyle(.secondary)
            }
            Section {
                Text("Guide results come from your server's connected sources. Provider links on media details open the official app; availability and subscription access are separate.").font(.footnote).foregroundStyle(.secondary)
            }
        }.task(id: identity) { await model.load(query: query, store: store) }
    }
    @ViewBuilder private func result(_ item: MediaItem) -> some View {
        if ["Audio", "MusicAlbum", "MusicArtist", "Playlist"].contains(item.Type ?? "") {
            if item.Type == "Audio" {
                Button { Task { await store.playMusic([item]) } } label: { MusicRow(item: item) }
            } else {
                MediaNavigationLink(destination: .musicCollection(item, type: item.Type == "MusicArtist" ? "MusicAlbum" : "Audio")) { MusicRow(item: item) }
            }
        } else if item.Type == "TvChannel" {
            MediaNavigationLink(destination: .channel(item)) { Label(item.title, systemImage: "dot.radiowaves.left.and.right") }
        } else { MediaNavigationLink(destination: .detail(item)) { Label(item.title, systemImage: item.Type == "Episode" ? "tv" : "film") } }
    }
}
