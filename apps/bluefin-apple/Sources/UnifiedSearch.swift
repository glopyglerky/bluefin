import SwiftUI

enum UnifiedSearchScope: String, CaseIterable, Identifiable, Sendable {
    case all, tvShows, movies, music, liveTv
    var id: Self { self }
    var title: String {
        switch self {
        case .all: "All Media"
        case .tvShows: "TV Shows"
        case .movies: "Movies"
        case .music: "Music"
        case .liveTv: "Live TV"
        }
    }
    var includesCatalog: Bool { self == .all || self == .movies || self == .tvShows }
    var includesArtists: Bool { self == .all || self == .music }
    var includesChannels: Bool { self == .all || self == .liveTv }
    var previewTypes: String? {
        switch self {
        case .all: nil
        case .tvShows: "Series,Season,Episode"
        case .movies: "Movie"
        case .music: "Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist"
        case .liveTv: "Program,LiveTvProgram,TvChannel,LiveTvChannel"
        }
    }
}

struct UnifiedSearchFilters: Hashable, Sendable {
    var scope = UnifiedSearchScope.all
    var studioID: String?
    var tag: String?
    var collectionID: String?
    var hasFacets: Bool { studioID != nil || tag != nil || collectionID != nil }
    var count: Int { [studioID, tag, collectionID].compactMap { $0 }.count }
}

struct UnifiedSearchIdentity: Hashable {
    let query: String
    let filters: UnifiedSearchFilters
    let account: UUID
}

enum UnifiedSearchLane: String, CaseIterable, Sendable {
    case library, artists, programmes, channels
    var title: String {
        switch self {
        case .library: "Your library"
        case .artists: "Artists"
        case .programmes: "On now and upcoming"
        case .channels: "Channels"
        }
    }
}

/// Every page carries the same server filters. Guide endpoints do not support
/// library metadata facets, so refined searches use the scoped Items endpoint.
struct UnifiedSearchRequest {
    let query: String
    let filters: UnifiedSearchFilters
    let minEndDate: String
    init(query: String, filters: UnifiedSearchFilters, now: Date = Date()) {
        self.query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        self.filters = filters
        minEndDate = ISO8601DateFormatter().string(from: now)
    }
    var lanes: [UnifiedSearchLane] {
        if filters.hasFacets { return [.library] }
        if filters.scope == .liveTv { return [.programmes, .channels] }
        var values: [UnifiedSearchLane] = [.library]
        if filters.scope.includesArtists { values.append(.artists) }
        if filters.scope.includesChannels { values += [.programmes, .channels] }
        return values
    }
    var isActive: Bool { !query.isEmpty || filters.scope != .all || filters.hasFacets }
    func call(for lane: UnifiedSearchLane, start: Int) -> (operation: String, fields: [String: Any]) {
        var fields: [String: Any] = ["startIndex": start, "limit": 60]
        if !query.isEmpty { fields["searchTerm"] = query }
        switch lane {
        case .library:
            fields["searchScope"] = filters.scope.rawValue
            if let id = filters.studioID { fields["studioIds"] = [id] }
            if let tag = filters.tag { fields["tags"] = [tag] }
            if let id = filters.collectionID { fields["parentId"] = id }
            return ("items", fields)
        case .artists:
            fields["kind"] = "artists"
            return ("musicItems", fields)
        case .programmes:
            fields["minEndDate"] = minEndDate
            return ("livePrograms", fields)
        case .channels:
            return ("liveChannels", fields)
        }
    }
}

struct UnifiedSearchPage {
    private(set) var items: [MediaItem] = []
    private(set) var nextIndex = 0
    private(set) var total = 0
    var hasMore: Bool { nextIndex < total }
    mutating func append(_ page: ItemPage) {
        var existing = Set(items.map(\.Id))
        items += page.Items.filter { existing.insert($0.Id).inserted }
        nextIndex += page.Items.count
        total = page.Items.isEmpty ? nextIndex : max(nextIndex, page.TotalRecordCount ?? nextIndex)
    }
}

struct UnifiedSearchFacetOption: Decodable, Identifiable {
    let id: String
    let name: String
}
struct UnifiedSearchFacets: Decodable {
    var studios: [UnifiedSearchFacetOption] = []
    var tags: [String] = []
    var collections: [UnifiedSearchFacetOption] = []
    var truncated: Bool?
}

@MainActor final class UnifiedSearchModel: ObservableObject {
    @Published private(set) var pages: [UnifiedSearchLane: UnifiedSearchPage] = [:]
    @Published private(set) var loading = false
    @Published private(set) var loadedContext: UUID?
    @Published private(set) var failures: [String] = []
    @Published private(set) var filters = UnifiedSearchFilters()
    @Published private(set) var facets = UnifiedSearchFacets()
    @Published private(set) var facetsLoading = false
    @Published private(set) var facetsProblem: String?
    private var facetsContext: UUID?
    private var generation = UUID()
    private var request: UnifiedSearchRequest?
    private var moreTask: Task<Void, Never>?
    private enum Part { case page(UnifiedSearchLane, ItemPage), failure(String) }
    var items: [MediaItem] { pages[.library]?.items ?? [] }
    var artists: [MediaItem] { pages[.artists]?.items ?? [] }
    var channels: [MediaItem] { pages[.channels]?.items ?? [] }
    var programmes: [MediaItem] { pages[.programmes]?.items ?? [] }
    var isEmpty: Bool { pages.values.allSatisfy { $0.items.isEmpty } }

    func load(query: String, store: LibraryStore, filters: UnifiedSearchFilters = .init()) async {
        moreTask?.cancel(); moreTask = nil
        let current = UUID(); generation = current
        let context = store.accountContext
        let request = UnifiedSearchRequest(query: query, filters: filters)
        self.request = request; self.filters = filters
        loadedContext = nil; pages = [:]; failures = []; loading = request.isActive
        guard request.isActive else { return }
        defer { if generation == current { loading = false } }
        await withTaskGroup(of: Part.self) { group in
            for lane in request.lanes {
                group.addTask {
                    do { return .page(lane, try await Self.fetch(request, lane: lane, start: 0, store: store, context: context)) }
                    catch is CancellationError { return .failure("") }
                    catch { return .failure(lane.title + ": " + error.localizedDescription) }
                }
            }
            for await part in group {
                guard generation == current, !Task.isCancelled, store.accountContext == context else {
                    group.cancelAll(); continue
                }
                loadedContext = context
                switch part {
                case .page(let lane, let page):
                    var result = UnifiedSearchPage(); result.append(page); pages[lane] = result
                case .failure(let message):
                    if !message.isEmpty { failures.append(message) }
                }
            }
        }
    }

    func loadMore(_ lane: UnifiedSearchLane, store: LibraryStore) {
        guard !loading, let request, let page = pages[lane], page.hasMore,
              loadedContext == store.accountContext else { return }
        let current = generation, context = store.accountContext
        loading = true
        moreTask = Task {
            defer { if generation == current { loading = false } }
            do {
                let next = try await Self.fetch(request, lane: lane, start: page.nextIndex, store: store, context: context)
                try Task.checkCancellation()
                guard generation == current, store.accountContext == context else { return }
                var updated = page; updated.append(next); pages[lane] = updated
            } catch is CancellationError {} catch {
                guard generation == current, store.accountContext == context else { return }
                failures.append(lane.title + ": " + error.localizedDescription)
            }
        }
    }

    func cancel() { moreTask?.cancel(); moreTask = nil; generation = UUID(); loading = false }

    private static func fetch(_ request: UnifiedSearchRequest, lane: UnifiedSearchLane, start: Int,
                              store: LibraryStore, context: UUID) async throws -> ItemPage {
        try Task.checkCancellation()
        guard store.accountContext == context else { throw CancellationError() }
        if store.preview {
            if request.filters.hasFacets { return .init(Items: [], TotalRecordCount: 0) }
            switch lane {
            case .library:
                return try await store.items(query: request.query, start: start, includeItemTypes: request.filters.scope.previewTypes)
            case .artists:
                return try await store.musicItems(type: "MusicArtist", start: start, query: request.query)
            case .programmes, .channels:
                return .init(Items: [], TotalRecordCount: 0)
            }
        }
        let call = request.call(for: lane, start: start)
        let page: ItemPage = try await store.authenticated(call.operation, fields: call.fields)
        guard store.accountContext == context else { throw CancellationError() }
        return page
    }

    func loadFacets(store: LibraryStore) async {
        let context = store.accountContext
        guard facetsContext != context else { return }
        facets = .init(); facetsProblem = nil
        guard !store.preview else { facetsContext = context; return }
        facetsLoading = true
        defer { if store.accountContext == context { facetsLoading = false } }
        do {
            let value: UnifiedSearchFacets = try await store.authenticated("searchFilters")
            try Task.checkCancellation()
            guard store.accountContext == context else { return }
            facets = value; facetsContext = context
        } catch is CancellationError {} catch {
            guard store.accountContext == context else { return }
            facetsProblem = "Search refinements could not be loaded. " + error.localizedDescription
        }
    }
}

struct UnifiedSearchResults: View {
    let query: String
    @EnvironmentObject private var store: LibraryStore
    @StateObject private var model = UnifiedSearchModel()
    @State private var filters: UnifiedSearchFilters
    @State private var showFilters = false
    init(query: String, initialScope: UnifiedSearchScope = .all) {
        self.query = query
        _filters = State(initialValue: .init(scope: initialScope))
    }
    private var identity: UnifiedSearchIdentity {
        .init(query: query, filters: filters, account: store.accountContext)
    }
    private var active: Bool { !query.isEmpty || filters.scope != .all || filters.hasFacets }
    private var current: Bool { model.filters == filters && model.loadedContext == store.accountContext }
    var body: some View {
        List {
            Section {
                scopePicker
                Button { showFilters.toggle() } label: {
                    Label(filters.hasFacets ? "Refine search (\(filters.count))" : "Refine search",
                          systemImage: "line.3.horizontal.decrease.circle")
                }.accessibilityIdentifier("searchRefinements")
                if showFilters { refinementControls }
                if filters.hasFacets {
                    HStack {
                        Text("Filtering your library").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Clear refinements") {
                            filters.studioID = nil; filters.tag = nil; filters.collectionID = nil
                        }
                    }
                }
            }
            if active {
                if !store.preview && !filters.hasFacets && (filters.scope == .all || filters.scope == .tvShows) {
                    PublicTitleSearchSection(query: query, catalog: .television)
                }
                if !store.preview && !filters.hasFacets && (filters.scope == .all || filters.scope == .movies) {
                    PublicTitleSearchSection(query: query, catalog: .film, library: current ? model.items : [])
                }
                if !store.preview && !filters.hasFacets && filters.scope.includesArtists {
                    PublicMusicSearchSection(query: query, kind: "artist", library: current ? model.items : [])
                    PublicMusicSearchSection(query: query, kind: "releaseGroup", library: current ? model.items : [])
                    PublicMusicSearchSection(query: query, kind: "recording", library: current ? model.items : [])
                }
                if !query.isEmpty && !filters.hasFacets {
                    if filters.scope.includesCatalog { ServerDiscoverySection(query: query, scope: filters.scope) }
                    if filters.scope == .all || filters.scope == .music { ServerDiscoverySection(query: query, scope: .music) }
                }
                if current && !query.isEmpty {
                    LocalAssistanceSearchSection(query: query, candidates: model.items + model.artists,
                                                 loadedContext: model.loadedContext)
                        .id(identity)
                }
                if current {
                    ForEach(UnifiedSearchLane.allCases, id: \.self) { lane in
                        if let page = model.pages[lane], !page.items.isEmpty {
                            Section(lane.title) {
                                ForEach(displayed(page.items, lane: lane)) { item in result(item) }
                                if page.hasMore {
                                    Button("More \(lane.title.lowercased()) results") { model.loadMore(lane, store: store) }
                                        .disabled(model.loading)
                                }
                            }
                        }
                    }
                }
                if model.loading { ProgressView("Searching") }
                ForEach(model.failures, id: \.self) { Text($0).font(.callout).foregroundStyle(.secondary) }
                if !model.loading && current && model.isEmpty {
                    Text(filters.hasFacets ? "No titles match these refinements." : "No matching titles in your connected library or guide.")
                        .foregroundStyle(.secondary)
                }
            } else {
                Section {
                    Label("Find your next favorite", systemImage: "magnifyingglass").font(.headline)
                    Text("Search for a title or choose a media type. Connected libraries and the public TV catalog can appear together.")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .task(id: identity) { await model.load(query: query, store: store, filters: filters) }
        .task(id: showFilters ? store.accountContext : nil) {
            if showFilters { await model.loadFacets(store: store) }
        }
        .onDisappear { model.cancel() }
    }

    private var scopePicker: some View {
        Group {
            #if os(tvOS)
            picker.pickerStyle(.segmented)
            #else
            picker
            #endif
        }.accessibilityIdentifier("searchScope")
    }
    private var picker: some View {
        Picker("Media type", selection: $filters.scope) {
            ForEach(UnifiedSearchScope.allCases) { scope in Text(scope.title).tag(scope) }
        }
    }
    private func selection(_ keyPath: WritableKeyPath<UnifiedSearchFilters, String?>) -> Binding<String> {
        Binding(get: { filters[keyPath: keyPath] ?? "" },
                set: { filters[keyPath: keyPath] = $0.isEmpty ? nil : $0 })
    }
    @ViewBuilder private var refinementControls: some View {
        if model.facetsLoading { ProgressView("Loading your server’s refinements") }
        Picker("Studio", selection: selection(\.studioID)) {
            Text("All studios").tag("")
            ForEach(model.facets.studios) { Text($0.name).tag($0.id) }
        }
        Picker("Tag", selection: selection(\.tag)) {
            Text("All tags").tag("")
            ForEach(model.facets.tags, id: \.self) { Text($0).tag($0) }
        }
        Picker("Collection", selection: selection(\.collectionID)) {
            Text("All collections").tag("")
            ForEach(model.facets.collections) { Text($0.name).tag($0.id) }
        }
        Text("Studios, tags, and collections come from your server. Use its tags for services or custom groups, and its collections for grouped titles. Refinements search library metadata; connected catalogs and the live guide are searched without these refinements.")
            .font(.footnote).foregroundStyle(.secondary)
        if model.facets.truncated == true {
            Text("Your server has more refinements than can be shown here. Open Collections to browse all groups.")
                .font(.footnote).foregroundStyle(.secondary)
        }
        if let problem = model.facetsProblem {
            Text(problem).font(.footnote).foregroundStyle(.secondary)
            Button("Reload refinements") { Task { await model.loadFacets(store: store) } }
        }
    }
    private func displayed(_ items: [MediaItem], lane: UnifiedSearchLane) -> [MediaItem] {
        guard lane == .artists || lane == .programmes else { return items }
        let existing = Set(model.items.map(\.Id))
        return items.filter { !existing.contains($0.Id) }
    }
    @ViewBuilder private func result(_ item: MediaItem) -> some View {
        if ["Audio", "MusicAlbum", "MusicArtist", "Playlist"].contains(item.Type ?? "") {
            if item.Type == "Audio" {
                Button {
                    Task { await store.recordHomeSearch(query: query, selectedItem: item) }
                    Task { await store.playMusic([item]) }
                } label: { MusicRow(item: item) }
            } else {
                resultLink(item) { MusicRow(item: item) }
            }
        } else if ["TvChannel", "LiveTvChannel"].contains(item.Type ?? "") {
            resultLink(item) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(item.title).font(.headline)
                    if let programme = item.CurrentProgram?.title { Text(programme).font(.subheadline).foregroundStyle(.secondary) }
                }
            }
        } else if ["Program", "LiveTvProgram"].contains(item.Type ?? "") {
            if item.ChannelId != nil {
                resultLink(item) { Label(item.title, systemImage: "calendar") }
            } else { Label(item.title, systemImage: "calendar") }
        } else {
            resultLink(item) { Label(item.title, systemImage: ["Episode", "Season", "Series"].contains(item.Type ?? "") ? "tv" : "film") }
        }
    }
    private func resultLink<LabelView: View>(_ item: MediaItem, @ViewBuilder label: @escaping () -> LabelView) -> some View {
        MediaNavigationLink(destination: .searchResult(item, query: query, accountContext: store.accountContext), label: label)
    }
}

struct PublicTitleSearchView: View {
    @State private var query = ""
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        BluefinNavigation {
            List {
                TextField("Movie, show, artist, album or song", text: $query)
                    .accessibilityIdentifier("publicCatalogQuery")
                PublicTitleSearchSection(query: query, catalog: .television)
                PublicTitleSearchSection(query: query, catalog: .film)
                PublicMusicSearchSection(query: query, kind: "artist")
                PublicMusicSearchSection(query: query, kind: "releaseGroup")
                PublicMusicSearchSection(query: query, kind: "recording")
            }
            .navigationTitle("Search media")
            .toolbar { Button("Done") { dismiss() } }
        }
    }
}

private enum PublicTitleCatalog {
    case television, film

    var operation: String { self == .television ? "publicTvSearch" : "publicFilmSearch" }
    var heading: String { self == .television ? "TV catalog" : "Film catalog" }
    var provider: String { self == .television ? "TVmaze" : "Wikidata" }
    var icon: String { self == .television ? "tv" : "film" }
}

private struct PublicTitleSearchSection: View {
    struct Page: Decodable, Sendable { let results: [Title] }
    struct Title: Decodable, Identifiable, Sendable {
        var id: URL { sourceUrl }
        let title: String
        let year: String?
        let description: String?
        let artwork: URL?
        let artworkCredit: PublicTitleProviderChoices.ArtworkCredit?
        let sourceUrl: URL
        let imdbId: String?
        let links: [PublicTitleProviderChoices.Destination]?
    }

    let query: String
    let catalog: PublicTitleCatalog
    var library: [MediaItem] = []
    @State private var page: Page?
    @State private var failed = false
    @State private var completedQuery = ""
    private var term: String { String(query.trimmingCharacters(in: .whitespacesAndNewlines).prefix(160)) }

    var body: some View {
        Group {
            if term.count >= 2 {
                Section(catalog.heading) {
                    if completedQuery != term {
                        ProgressView("Searching \(catalog.heading.lowercased())")
                    } else if failed {
                        Text("Could not load \(catalog.provider) results.").foregroundStyle(.secondary)
                    } else if let page, page.results.isEmpty {
                        Text("No matches from \(catalog.provider).").foregroundStyle(.secondary)
                    } else if let page {
                        ForEach(page.results) { item in
                            NavigationLink {
                                PublicTitleProviderChoices(
                                catalogID: item.sourceUrl.lastPathComponent,
                                kind: catalog == .film ? "movie" : "series",
                                imdbId: item.imdbId,
                                title: item.title,
                                subtitle: [item.description ?? item.year, catalog.provider].compactMap { $0 }.joined(separator: " · "),
                                artwork: item.artwork,
                                artworkCredit: item.artworkCredit,
                                overview: item.description,
                                initialLinks: item.links ?? [],
                                icon: catalog.icon,
                                library: library
                                )
                            } label: {
                                HStack(spacing: 14) {
                                    AsyncImage(url: item.artwork) { image in image.resizable().scaledToFill() }
                                    placeholder: { Image(systemName: catalog.icon).foregroundStyle(.secondary) }
                                        .frame(width: 46, height: 64).clipped().accessibilityHidden(true)
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text(item.title).font(.headline)
                                        Text([item.year, catalog.provider].compactMap { $0 }.joined(separator: " · "))
                                            .font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        .task(id: term) {
            let requested = term
            guard requested.count >= 2 else { page = nil; failed = false; completedQuery = ""; return }
            do {
                try await Task.sleep(nanoseconds: 650_000_000)
                let response: Page = try await RustBridge.call(catalog.operation, fields: [
                    "query": requested, "language": Locale.current.identifier.lowercased().replacingOccurrences(of: "_", with: "-")
                ])
                try Task.checkCancellation()
                page = response
                failed = false
                completedQuery = requested
            } catch is CancellationError {} catch {
                guard !Task.isCancelled else { return }
                page = nil
                failed = true
                completedQuery = requested
            }
        }
    }
}

private struct PublicTitleProviderChoices: View {
    struct Result: Decodable, Sendable {
        let links: [Destination]
        let imdbIds: [String]
        let description: String?
        let credits: [Credit]?
        let catalogIdentity: PublicCatalogIdentity?
    }
    struct Credit: Decodable, Sendable, Hashable { let name: String; let role: String }
    struct ArtworkCredit: Decodable, Sendable { let author: String; let url: URL; let license: String; let licenseUrl: URL? }
    struct Destination: Decodable, Sendable, Identifiable {
        let provider: String
        let url: URL
        let kind: String
        var id: URL { url }
        var action: String { (kind == "player" ? "Watch on " : "Open on ") + provider }
    }

    let catalogID: String
    let kind: String
    let imdbId: String?
    let title: String
    let subtitle: String
    let artwork: URL?
    let artworkCredit: ArtworkCredit?
    let overview: String?
    let initialLinks: [Destination]
    let icon: String
    let library: [MediaItem]
    @EnvironmentObject private var store: LibraryStore
    @State private var attempt = 1
    @State private var completed = 0
    @State private var result: Result?
    private var destinations: [Destination] {
        var seen = Set<URL>()
        return ((result?.links ?? []) + initialLinks).filter { seen.insert($0.url).inserted }
    }

    private var playableLibraryItems: [MediaItem] {
        guard kind == "movie" else { return [] }
        return library.filter { media in
            guard media.Type == "Movie", let id = media.ProviderIds?["Imdb"] ?? media.ProviderIds?["IMDb"] else {
                return false
            }
            return id == imdbId || result?.imdbIds.contains(id) == true
        }
    }

    var body: some View {
        List {
            Section {
                HStack(spacing: 14) {
                    AsyncImage(url: artwork) { image in
                        image.resizable().scaledToFill()
                    } placeholder: {
                        Image(systemName: icon).foregroundStyle(.secondary)
                    }
                    .frame(width: 100, height: 145).clipped().accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(title).font(.headline)
                        Text(subtitle).font(.caption).foregroundStyle(.secondary)
                        if let text = result?.description ?? overview, !text.isEmpty { Text(text) }
                    }
                }
                if let artworkCredit {
                    Link(artworkCredit.author, destination: artworkCredit.url)
                    if let license = artworkCredit.licenseUrl { Link(artworkCredit.license, destination: license) }
                }
            }
            if !playableLibraryItems.isEmpty {
                Section {
                    ForEach(playableLibraryItems) { media in
                        Button(media.resumeTicks > 0 ? "Resume in Bluefin" : "Play in Bluefin") {
                            Task { await store.play(media) }
                        }
                    }
                }
            }
            if let identity = result?.catalogIdentity { PublicCatalogAccessSection(identity: identity) }
            Section("Catalog links") {
                ForEach(destinations) { destination in
                    Link(destination.action, destination: destination.url)
                }
                if attempt > completed { ProgressView("Looking up providers") }
                else if result == nil { Button("Retry provider lookup") { attempt += 1 } }
                else if destinations.isEmpty { Text("No direct provider destinations found.").foregroundStyle(.secondary) }
            }
            if let credits = result?.credits, !credits.isEmpty {
                Section("Credits") {
                    ForEach(credits.indices, id: \.self) { index in
                        VStack(alignment: .leading) {
                            Text(credits[index].name)
                            Text(credits[index].role).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .navigationTitle(title)
        .task(id: attempt) {
            let request = attempt
            guard request > 0 else { return }
            var fields = ["id": catalogID, "kind": kind]
            fields["imdbId"] = imdbId
            do {
                let response: Result = try await RustBridge.call("publicTitleSources", fields: fields)
                try Task.checkCancellation()
                result = response
                completed = request
            } catch is CancellationError {} catch {
                guard !Task.isCancelled else { return }
                result = nil
                completed = request
            }
        }
    }
}

/// Navigation activates this screen for every supported input method. A merely
/// rendered search result does not write a preference or visit the owner.
struct HomeSearchDestination: View {
    let item: MediaItem
    let query: String
    let accountContext: UUID
    var onBack: (() -> Void)? = nil
    @EnvironmentObject private var store: LibraryStore
    @State private var recorded = false
    private var destination: MediaDestination {
        if ["MusicAlbum", "MusicArtist", "Playlist"].contains(item.Type ?? "") {
            return .musicCollection(item, type: item.Type == "MusicArtist" ? "MusicAlbum" : "Audio")
        }
        if ["TvChannel", "LiveTvChannel"].contains(item.Type ?? "") { return .channel(item) }
        return .detail(item)
    }
    var body: some View {
        Group {
            if ["Program", "LiveTvProgram"].contains(item.Type ?? "") {
                SearchProgrammeDestination(programme: item, onBack: onBack)
            } else { destination.screen(back: onBack) }
        }.task {
            guard !recorded, store.accountContext == accountContext else { return }
            recorded = true
            await store.recordHomeSearch(query: query, selectedItem: item)
        }
    }
}

private struct SearchProgrammeDestination: View {
    let programme: MediaItem
    var onBack: (() -> Void)?
    @EnvironmentObject private var store: LibraryStore
    @State private var channel: MediaItem?
    @State private var problem: String?
    @State private var opening = false
    private var isAiring: Bool {
        guard let start = programme.airingStart, let end = programme.airingEnd else { return false }
        let now = Date()
        return start <= now && now < end
    }

    var body: some View {
        Group {
            if let channel { LiveChannelView(channel: channel) }
            else {
                VStack(alignment: .leading, spacing: 16) {
                    Text(programme.title).font(.title.bold())
                    if let start = programme.airingStart {
                        Text(start.formatted(date: .abbreviated, time: .shortened)).foregroundStyle(.secondary)
                    }
                    if let overview = programme.Overview, !overview.isEmpty { Text(overview) }
                    if let problem { Text(problem).foregroundStyle(.secondary) }
                    if opening { ProgressView("Opening channel") }
                    else if isAiring && programme.ChannelId != nil {
                        Button("Watch live channel") { opening = true }
                    }
                }.padding()
            }
        }
        .bluefinBackNavigation(onBack)
        .task(id: opening) {
            guard opening else { return }
            defer { opening = false }
            let context = store.accountContext
            problem = nil
            guard isAiring, let id = programme.ChannelId else { return }
            do {
                let value = try await store.detail(MediaItem(Id: id))
                try Task.checkCancellation()
                guard store.accountContext == context, isAiring else { return }
                guard value.Id.lowercased() == id.lowercased(),
                      ["TvChannel", "LiveTvChannel"].contains(value.Type ?? "") else {
                    problem = "This channel is no longer available."; return
                }
                channel = value
            } catch is CancellationError {} catch {
                if store.accountContext == context { problem = error.localizedDescription }
            }
        }
    }
}

struct HomeHistorySettings: View {
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        Section("Home personalization") {
            Text("Your viewing progress and favorites shape Home on your account's server.")
                .font(.footnote).foregroundStyle(.secondary)
            if store.searchHistoryAvailable {
                Text("Submitted searches and selected results also help. Search history stays with this account for up to 90 days, with at most 64 recent events.")
                    .font(.footnote).foregroundStyle(.secondary)
                Button(role: .destructive) { Task { await store.clearHomeSearchHistory() } } label: {
                    Label(store.clearingHomeHistory ? "Clearing search history…" : "Clear search history", systemImage: "clock.arrow.circlepath")
                }.disabled(store.clearingHomeHistory)
            } else {
                Text("Search personalization becomes available when this server supports account search history.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            if let problem = store.homeHistoryProblem {
                Text(problem).font(.footnote).foregroundStyle(.secondary)
            }
        }
    }
}


// MARK: - Public catalog contracts

struct PublicCatalogIdentity: Codable, Hashable, Sendable {
    struct Entry: Codable, Hashable, Sendable { let source: String; let id: String }
    let kind: String
    let ids: [Entry]
    var fields: [String: Any] { ["kind": kind, "ids": ids.map { ["source": $0.source, "id": $0.id] }] }
    var music: Bool { ["artist", "release", "releaseGroup", "recording", "musicVideo", "work"].contains(kind) }
    var key: String { kind + ":" + ids.map { $0.source + ":" + $0.id }.sorted().joined(separator: "|") }
}
extension PublicCatalogIdentity {
    static func library(_ item: MediaItem) -> Self? {
        func provider(_ key: String) -> String? {
            item.ProviderIds?.first { $0.key.caseInsensitiveCompare(key) == .orderedSame }?.value
        }
        func music(_ kind: String, key: String) -> Self? {
            guard let value = provider(key), let uuid = UUID(uuidString: value),
                  uuid.uuidString != "00000000-0000-0000-0000-000000000000" else { return nil }
            return Self(kind: kind, ids: [Entry(source: "musicbrainz", id: uuid.uuidString.lowercased())])
        }
        switch item.Type {
        case "Movie", "Series":
            guard let value = provider("Tmdb"),
                  value.range(of: #"^[1-9][0-9]{0,11}$"#, options: .regularExpression) != nil else { return nil }
            return Self(kind: item.Type == "Movie" ? "movie" : "series", ids: [Entry(source: "tmdb", id: value)])
        case "MusicArtist": return music("artist", key: "MusicBrainzArtist")
        case "MusicAlbum":
            return music("releaseGroup", key: "MusicBrainzReleaseGroup") ?? music("release", key: "MusicBrainzAlbum")
        case "Audio": return music("recording", key: "MusicBrainzRecording")
        default: return nil
        }
    }
}
struct PublicCatalogAttribution: Decodable, Sendable, Hashable { let name: String; let url: URL }
struct PublicCatalogSource: Decodable, Sendable {
    let source: String
    let status: String
    let observedAt: UInt64
    let scope: String
    let coverage: String
    let catalogUrl: URL?
    let attributions: [PublicCatalogAttribution]
    var interrupted: Bool { ["unavailable", "rateLimited", "denied", "invalidResponse", "identityMismatch"].contains(status) }
    var description: String {
        switch status {
        case "complete": "Source checked"
        case "partial": "Partial source coverage"
        case "noOffers": "No offers reported"
        case "notFound": "Title not found by this source"
        case "unsupportedIdentity": "This source does not cover this media identity"
        case "unsupportedCountry": "This source cannot confirm this region"
        case "missingCredentials": "Source not configured"
        case "rateLimited": "Source rate limit reached"
        case "denied": "Source declined the request"
        case "identityMismatch": "Source identity did not match"
        case "invalidResponse": "Source returned an unreadable result"
        default: "Source temporarily unavailable"
        }
    }
}
struct PublicCatalogService: Decodable, Sendable { let source: String; let id: String; let name: String }
struct PublicCatalogOffer: Decodable, Identifiable, Sendable {
    struct Destination: Decodable, Sendable, Identifiable { let kind: String; let url: URL; var id: String { kind + url.absoluteString } }
    struct Price: Decodable, Sendable { let amount: String?; let currency: String?; let formatted: String? }
    struct Evidence: Decodable, Sendable {
        let source: String; let observedAt: UInt64; let availableSince: UInt64?
        let expiresAt: UInt64?; let expiresSoon: Bool; let regionVerified: Bool
    }
    let id: String
    let identity: PublicCatalogIdentity
    let country: String?
    let service: PublicCatalogService
    let accessType: String
    let rawAccessType: String?
    let addon: PublicCatalogService?
    let destinations: [Destination]
    let price: Price?
    let quality: String?
    let evidence: Evidence
    var accessLabel: String {
        switch accessType {
        case "subscription": "Subscription"
        case "rent": "Rent"
        case "buy": "Buy"
        case "free": "Free"
        case "ads": "With ads"
        case "addon": "Add-on"
        case "download": "Download"
        default: rawAccessType ?? "Provider listing"
        }
    }
    var priceLabel: String? {
        if let formatted = price?.formatted, !formatted.isEmpty { return formatted }
        guard let amount = price?.amount else { return nil }
        return [amount, price?.currency].compactMap { $0 }.joined(separator: " ")
    }
}
struct PublicCatalogOffers: Decodable, Sendable {
    let schemaVersion: Int
    let identity: PublicCatalogIdentity
    let country: String?
    let offers: [PublicCatalogOffer]
    let sources: [PublicCatalogSource]
}
struct PublicCatalogCountries: Decodable, Sendable {
    struct Country: Decodable, Sendable { let code: String; let name: String }
    let countries: [Country]
    let sources: [PublicCatalogSource]
}
enum PublicCatalogAPI {
    static func offers(_ identity: PublicCatalogIdentity, country: String?) async throws -> PublicCatalogOffers {
        var fields: [String: Any] = ["identity": identity.fields]
        if let country, !country.isEmpty { fields["country"] = country }
        return try await RustBridge.call("publicCatalogOffers", fields: fields)
    }
    static func countries(_ identity: PublicCatalogIdentity) async throws -> PublicCatalogCountries {
        try await RustBridge.call("publicCatalogCountries", fields: ["identity": identity.fields])
    }
}
enum PublicCatalogRegion {
    static var codes: [String] { Locale.isoRegionCodes.sorted { name($0).localizedCaseInsensitiveCompare(name($1)) == .orderedAscending } }
    static var initial: String {
        let code = Locale.current.regionCode?.uppercased() ?? ""
        return Locale.isoRegionCodes.contains(code) ? code : ""
    }
    static func name(_ code: String) -> String { Locale.current.localizedString(forRegionCode: code) ?? code }
}
struct PublicCatalogCoverage: View {
    let sources: [PublicCatalogSource]
    var body: some View {
        ForEach(sources.indices, id: \.self) { index in
            let source = sources[index]
            VStack(alignment: .leading, spacing: 4) {
                Text(source.source + " · " + source.description).font(.caption)
                if source.observedAt > 0 {
                    Text("Checked " + Date(timeIntervalSince1970: TimeInterval(source.observedAt)).formatted(date: .abbreviated, time: .shortened))
                        .font(.caption2).foregroundStyle(.secondary)
                }
                ForEach(source.attributions, id: \.self) { credit in Link(credit.name, destination: credit.url).font(.caption) }
            }
        }
    }
}
struct PublicCatalogAccessSection: View {
    private struct Observation: Identifiable {
        let id = UUID()
        let country: String?
        let value: PublicCatalogSource
    }
    let identity: PublicCatalogIdentity
    @AppStorage("publicCatalogCountry") private var country = PublicCatalogRegion.initial
    @State private var offers: [PublicCatalogOffer] = []
    @State private var observations: [Observation] = []
    @State private var attempt = 0
    @State private var loading = false
    @State private var total = 0
    @State private var completed = 0
    @State private var message: String?
    private var requestKey: String { identity.key + ":" + country + ":" + String(attempt) }
    var body: some View {
        Section(identity.music ? "Listen or buy" : "Where to watch") {
            Picker("Region", selection: $country) {
                Text("All reported regions").tag("")
                ForEach(PublicCatalogRegion.codes, id: \.self) { code in Text(PublicCatalogRegion.name(code)).tag(code) }
            }
            if loading {
                if total > 0 { ProgressView("Checking regions \(completed) of \(total)", value: Double(completed), total: Double(total)) }
                else { ProgressView("Checking provider availability") }
            }
            if let message { Text(message).font(.caption).foregroundStyle(.secondary) }
            ForEach(offers) { offer in
                VStack(alignment: .leading, spacing: 5) {
                    Text(offer.service.name + (offer.addon.map { " · " + $0.name } ?? "")).font(.headline)
                    Text([offer.accessLabel, offer.priceLabel, offer.quality].compactMap { $0 }.joined(separator: " · "))
                    Text(offer.evidence.regionVerified
                        ? offer.country.map(PublicCatalogRegion.name) ?? "Region not supplied"
                        : "Region not confirmed").font(.caption).foregroundStyle(.secondary)
                    if let start = offer.evidence.availableSince {
                        Text("Available from " + Date(timeIntervalSince1970: TimeInterval(start)).formatted(date: .abbreviated, time: .omitted)).font(.caption)
                    }
                    if let end = offer.evidence.expiresAt {
                        Text("Available until " + Date(timeIntervalSince1970: TimeInterval(end)).formatted(date: .abbreviated, time: .omitted)).font(.caption)
                    } else if offer.evidence.expiresSoon { Text("Leaving soon").font(.caption) }
                    ForEach(offer.destinations) { destination in
                        Link((destination.kind == "player" ? "Open player on " : "Open on ") + offer.service.name, destination: destination.url)
                    }
                    if offer.destinations.isEmpty { Text("Source reports availability without a direct destination.").font(.caption) }
                    if offer.evidence.observedAt > 0 {
                        Text("Reported " + Date(timeIntervalSince1970: TimeInterval(offer.evidence.observedAt)).formatted(date: .abbreviated, time: .shortened))
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
            if !loading && offers.isEmpty {
                Text(observations.contains { !["complete", "noOffers", "notFound"].contains($0.value.status) }
                    ? "Availability could not be confirmed for every source. Check coverage or retry."
                    : "No offers were reported for this selection.").foregroundStyle(.secondary)
            }
            if !loading { Button("Refresh availability") { attempt += 1 } }
            DisclosureGroup("Sources and coverage") {
                ForEach(observations) { observation in
                    if let code = observation.country { Text(PublicCatalogRegion.name(code)).font(.caption.bold()) }
                    PublicCatalogCoverage(sources: [observation.value])
                }
                Text("Provider links do not confirm your account's playback rights.").font(.caption).foregroundStyle(.secondary)
            }
        }
        .task(id: requestKey) {
            offers = []; observations = []; message = nil; completed = 0; total = 0; loading = true
            let selected = country
            defer { if !Task.isCancelled { loading = false } }
            do {
                if !selected.isEmpty || identity.music {
                    let result = try await PublicCatalogAPI.offers(identity, country: selected.isEmpty ? nil : selected)
                    try Task.checkCancellation()
                    merge(result)
                    if identity.music && selected.isEmpty {
                        message = "MusicBrainz links do not include regional availability. Select a country to check exact store listings."
                    }
                    return
                }
                let registry = try await PublicCatalogAPI.countries(identity)
                try Task.checkCancellation()
                observations.append(contentsOf: registry.sources.map { Observation(country: nil, value: $0) })
                var seen = Set<String>()
                let regions = registry.countries.map(\.code).filter { seen.insert($0).inserted }
                total = regions.count
                guard !regions.isEmpty else {
                    message = "This source did not supply a country list."
                    return
                }
                let requestedIdentity = identity
                try await withThrowingTaskGroup(of: PublicCatalogOffers.self) { group in
                    var next = 0
                    for region in regions.prefix(3) {
                        next += 1
                        group.addTask { try await PublicCatalogAPI.offers(requestedIdentity, country: region) }
                    }
                    while let result = try await group.next() {
                        try Task.checkCancellation()
                        merge(result)
                        completed += 1
                        if result.sources.contains(where: { $0.interrupted }) {
                            message = "The source paused or failed. Loaded regions remain visible; refresh to continue."
                            group.cancelAll()
                            break
                        }
                        if next < regions.count && !Task.isCancelled {
                            let region = regions[next]; next += 1
                            group.addTask { try await PublicCatalogAPI.offers(requestedIdentity, country: region) }
                        }
                    }
                }
            } catch is CancellationError {
            } catch {
                guard !Task.isCancelled else { return }
                message = "Provider availability is temporarily unavailable."
            }
        }
    }
    private func merge(_ response: PublicCatalogOffers) {
        guard response.identity == identity else { return }
        var seen = Set(offers.map(\.id))
        offers.append(contentsOf: response.offers.filter { seen.insert($0.id).inserted })
        offers.sort { ($0.service.name, $0.country ?? "", $0.accessLabel, $0.id) < ($1.service.name, $1.country ?? "", $1.accessLabel, $1.id) }
        observations.append(contentsOf: response.sources.map { Observation(country: response.country, value: $0) })
    }
}
struct PublicMusicItem: Decodable, Identifiable, Sendable {
    struct ArtworkCredit: Decodable, Sendable { let author: String; let url: URL; let license: String; let licenseUrl: URL? }
    let identity: PublicCatalogIdentity
    let title: String
    let subtitle: String
    let artwork: URL?
    let artworkCredit: ArtworkCredit?
    let sourceUrl: URL
    let number: String?
    var id: String { identity.key }
}
struct PublicMusicSearchPage: Decodable, Sendable {
    let results: [PublicMusicItem]
    let total: Int
    let offset: Int
    let sources: [PublicCatalogSource]
}
struct PublicMusicDetail: Decodable, Sendable {
    struct Fact: Decodable, Identifiable, Sendable { let label: String; let value: String; var id: String { label } }
    let item: PublicMusicItem?
    let description: String?
    let facts: [Fact]
    let related: [PublicMusicItem]
    let tracks: [PublicMusicItem]?
    let trackEdition: PublicMusicItem?
    let sources: [PublicCatalogSource]
}
struct PublicMusicSearchSection: View {
    let query: String
    let kind: String
    var library: [MediaItem] = []
    @State private var items: [PublicMusicItem] = []
    @State private var total = 0
    @State private var offset = 0
    @State private var retry = 0
    @State private var failed = false
    @State private var loading = false
    @State private var completedQuery = ""
    private var term: String { String(query.trimmingCharacters(in: .whitespacesAndNewlines).prefix(160)) }
    private var heading: String { kind == "artist" ? "Artists" : kind == "releaseGroup" ? "Albums" : "Songs and recordings" }
    var body: some View {
        Group {
            if term.count >= 2 {
                Section(heading) {
                    if completedQuery != term || loading { ProgressView("Searching \(heading.lowercased())") }
                    if completedQuery == term {
                        ForEach(items) { item in
                            NavigationLink { PublicMusicDetailView(initial: item, library: library) } label: {
                                HStack(spacing: 12) {
                                    AsyncImage(url: item.artwork) { image in image.resizable().scaledToFill() }
                                        placeholder: { Image(systemName: kind == "artist" ? "person.crop.square" : "music.note") }
                                        .frame(width: 56, height: 56).clipped().accessibilityHidden(true)
                                    VStack(alignment: .leading) { Text(item.title); Text(item.subtitle).font(.caption).foregroundStyle(.secondary) }
                                }
                            }
                        }
                        if failed {
                            Text("MusicBrainz is temporarily unavailable.").foregroundStyle(.secondary)
                            Button("Retry music search") { retry += 1 }
                        }
                        else if items.isEmpty { Text("No matches from MusicBrainz.").foregroundStyle(.secondary) }
                        if items.count < total && !loading { Button("Load more \(heading.lowercased())") { offset += 20 } }
                        Text("MusicBrainz · \(items.count) of \(total) results").font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .task(id: term + ":" + String(offset) + ":" + String(retry)) {
            let requested = term
            guard requested.count >= 2 else { items = []; completedQuery = ""; return }
            let requestedOffset = completedQuery == requested ? offset : 0
            if completedQuery != requested { items = []; total = 0; offset = 0 }
            loading = true
            do {
                if requestedOffset == 0 { try await Task.sleep(nanoseconds: 650_000_000) }
                let result: PublicMusicSearchPage = try await RustBridge.call("publicMusicSearch", fields: ["query": requested, "kind": kind, "offset": requestedOffset])
                try Task.checkCancellation()
                var seen = Set(items.map(\.id))
                items.append(contentsOf: result.results.filter { seen.insert($0.id).inserted })
                total = result.total
                failed = result.sources.contains(where: { $0.interrupted })
                completedQuery = requested; loading = false
            } catch is CancellationError {} catch {
                guard !Task.isCancelled else { return }
                failed = true; completedQuery = requested; loading = false
            }
        }
    }
}
struct PublicMusicDetailView: View {
    let initial: PublicMusicItem
    var library: [MediaItem] = []
    @EnvironmentObject private var store: LibraryStore
    @State private var detail: PublicMusicDetail?
    @State private var failed = false
    @State private var attempt = 0
    private var item: PublicMusicItem { detail?.item ?? initial }
    private var playable: [MediaItem] {
        guard item.identity.kind == "recording", let id = item.identity.ids.first(where: { $0.source == "musicbrainz" })?.id else { return [] }
        return library.filter { $0.Type == "Audio" && ($0.ProviderIds?["MusicBrainzTrack"] == id || $0.ProviderIds?["MusicBrainzRecording"] == id) }
    }
    var body: some View {
        List {
            Section {
                HStack(alignment: .top, spacing: 16) {
                    AsyncImage(url: item.artwork) { image in image.resizable().scaledToFit() }
                        placeholder: { Image(systemName: item.identity.kind == "artist" ? "person.crop.square" : "music.note") }
                        .frame(width: 120, height: 120).accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 6) { Text(item.title).font(.title2); Text(item.subtitle).foregroundStyle(.secondary) }
                }
                if let credit = item.artworkCredit {
                    Link(credit.author, destination: credit.url)
                    if let license = credit.licenseUrl { Link(credit.license, destination: license) }
                } else if item.artwork != nil {
                    Link("Cover Art Archive", destination: URL(string: "https://coverartarchive.org/")!)
                }
                if let description = detail?.description, !description.isEmpty { Text(description) }
                ForEach(detail?.facts ?? []) { fact in LabeledContent(fact.label, value: fact.value) }
                if detail == nil && !failed { ProgressView("Loading music details") }
                if failed { Button("Retry music details") { attempt += 1 } }
            }
            if !playable.isEmpty {
                Section {
                    ForEach(playable) { media in Button(media.resumeTicks > 0 ? "Resume in Bluefin" : "Play in Bluefin") { Task { await store.play(media) } } }
                }
            }
            PublicCatalogAccessSection(identity: item.identity)
            if let tracks = detail?.tracks, !tracks.isEmpty {
                Section("Tracks") {
                    if let edition = detail?.trackEdition { Text("Edition: " + edition.title + " · " + edition.subtitle).font(.caption).foregroundStyle(.secondary) }
                    ForEach(tracks.indices, id: \.self) { index in
                        let track = tracks[index]
                        NavigationLink { PublicMusicDetailView(initial: track, library: library) } label: {
                            Text([track.number, track.title].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                        }
                    }
                }
            }
            if let related = detail?.related, !related.isEmpty {
                Section("Related music") {
                    ForEach(related) { relatedItem in
                        NavigationLink { PublicMusicDetailView(initial: relatedItem, library: library) } label: {
                            VStack(alignment: .leading) { Text(relatedItem.title); Text(relatedItem.subtitle).font(.caption).foregroundStyle(.secondary) }
                        }
                    }
                }
            }
            Section("Catalog source") {
                Link("View on MusicBrainz", destination: item.sourceUrl)
                if let detail { PublicCatalogCoverage(sources: detail.sources) }
            }
        }
        .navigationTitle(item.title)
        .task(id: initial.id + ":" + String(attempt)) {
            do {
                let result: PublicMusicDetail = try await RustBridge.call("publicMusicDetail", fields: ["identity": initial.identity.fields])
                try Task.checkCancellation()
                detail = result
                failed = result.item == nil
            } catch is CancellationError {} catch {
                guard !Task.isCancelled else { return }
                failed = true
            }
        }
    }
}
