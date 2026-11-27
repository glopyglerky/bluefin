import SwiftUI

enum UnifiedSearchScope: String, CaseIterable, Identifiable, Sendable {
    case all, libraryOnly, tvShows, movies, music, people, liveTv
    var id: Self { self }
    var title: String {
        switch self {
        case .all: "All Media"
        case .libraryOnly: "Library Only"
        case .people: "People"
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
        case .all, .libraryOnly: nil
        case .people: "Person"
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
    var source = UnifiedSearchSource.allSources
    var sort = LibrarySearchSort.relevance
    var watched = LibraryWatchFilter.all
    var favoritesOnly = false
    var genre: String?
    var year: Int?
    var hasFacets: Bool {
        studioID != nil || tag != nil || collectionID != nil || watched != .all || favoritesOnly || genre != nil || year != nil
    }
    var count: Int {
        [studioID, tag, collectionID, genre].compactMap { $0 }.count +
        (watched == .all ? 0 : 1) + (favoritesOnly ? 1 : 0) + (year == nil ? 0 : 1)
    }
    var libraryOnly: Bool { scope == .libraryOnly || source == .library || hasFacets || sort != .relevance }
    func apply(to fields: inout [String: Any]) {
        sort.apply(to: &fields)
        if let id = studioID { fields["studioIds"] = [id] }
        if let tag { fields["tags"] = [tag] }
        if let id = collectionID { fields["parentId"] = id }
        if watched != .all { fields["isPlayed"] = watched == .watched }
        if favoritesOnly { fields["isFavorite"] = true }
        if let genre { fields["genres"] = [genre] }
        if let year { fields["years"] = [year] }
    }
}

enum UnifiedSearchSource: String, CaseIterable, Identifiable, Sendable {
    case allSources, library
    var id: Self { self }
    var title: String { self == .library ? "Your library" : "All sources" }
}

enum LibrarySearchSort: String, CaseIterable, Identifiable, Sendable {
    case relevance, titleAscending, titleDescending, recentlyAdded, newestReleased, oldestReleased
    var id: Self { self }
    var title: String {
        switch self {
        case .relevance: "Relevance"
        case .titleAscending: "Title A–Z"
        case .titleDescending: "Title Z–A"
        case .recentlyAdded: "Recently added"
        case .newestReleased: "Newest release"
        case .oldestReleased: "Oldest release"
        }
    }
    func apply(to fields: inout [String: Any]) {
        guard self != .relevance else { return }
        let key: String
        switch self {
        case .relevance: return
        case .titleAscending, .titleDescending: key = "SortName"
        case .recentlyAdded: key = "DateCreated"
        case .newestReleased, .oldestReleased: key = "PremiereDate"
        }
        fields["sortBy"] = key
        fields["sortOrder"] = self == .titleAscending || self == .oldestReleased ? "Ascending" : "Descending"
        fields["searchOrder"] = "SortBy"
    }
}

enum LibraryWatchFilter: String, CaseIterable, Identifiable, Sendable {
    case all, unwatched, watched
    var id: Self { self }
    var title: String {
        switch self { case .all: "All"; case .unwatched: "Unwatched"; case .watched: "Watched" }
    }
}

struct UnifiedSearchIdentity: Hashable {
    let query: String
    let filters: UnifiedSearchFilters
    let account: UUID
}

enum UnifiedSearchLane: String, CaseIterable, Sendable {
    case library, artists, people, programmes, channels
    var title: String {
        switch self {
        case .library: "Your library"
        case .artists: "Artists"
        case .people: "People in your library"
        case .programmes: "On now and upcoming"
        case .channels: "Channels"
        }
    }
}

/// Each fetched or rehydrated page carries the complete original search.
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
        if filters.libraryOnly { return [.library] }
        if filters.scope == .liveTv { return [.programmes, .channels] }
        if filters.scope == .people { return [.people] }
        var values: [UnifiedSearchLane] = [.library]
        if filters.scope.includesArtists { values.append(.artists) }
        if filters.scope == .all { values.append(.people) }
        if filters.scope.includesChannels { values += [.programmes, .channels] }
        return values
    }
    var isActive: Bool { !query.isEmpty || filters.scope != .all || filters.libraryOnly }
    func call(for lane: UnifiedSearchLane, start: Int, limit: Int = MediaPageWindow.pageSize) -> (operation: String, fields: [String: Any]) {
        var fields: [String: Any] = ["startIndex": start, "limit": limit]
        if !query.isEmpty { fields["searchTerm"] = query }
        switch lane {
        case .library:
            fields["searchScope"] = filters.scope == .libraryOnly || filters.scope == .people ? "all" : filters.scope.rawValue
            if filters.scope == .people { fields["includeItemTypes"] = "Person"; fields.removeValue(forKey: "searchScope") }
            filters.apply(to: &fields)
            return ("items", fields)
        case .artists:
            fields["kind"] = "artists"
            return ("musicItems", fields)
        case .people:
            return ("searchPeople", fields)
        case .programmes:
            fields["minEndDate"] = minEndDate
            return ("livePrograms", fields)
        case .channels:
            return ("liveChannels", fields)
        }
    }
}

typealias UnifiedSearchPage = MediaPageWindow

struct UnifiedSearchFacetOption: Decodable, Identifiable {
    let id: String
    let name: String
}
struct UnifiedSearchFacets: Decodable {
    var studios: [UnifiedSearchFacetOption] = []
    var tags: [String] = []
    var collections: [UnifiedSearchFacetOption] = []
    var genres: [String]?
    var years: [Int]?
    var truncated: Bool?
}

@MainActor final class UnifiedSearchModel: ObservableObject {
    @Published private(set) var laneModels: [UnifiedSearchLane: PagedMediaModel] = [:]
    @Published private(set) var loading = false
    @Published private(set) var loadedContext: UUID?
    @Published private(set) var loadedQuery = ""
    @Published private(set) var filters = UnifiedSearchFilters()
    @Published private(set) var facets = UnifiedSearchFacets()
    @Published private(set) var facetsLoading = false
    @Published private(set) var facetsProblem: String?
    private var facetsContext: UUID?
    private var generation = UUID()
    private var loadedIdentity: UnifiedSearchIdentity?
    var pages: [UnifiedSearchLane: UnifiedSearchPage] { laneModels.mapValues(\.window) }
    var items: [MediaItem] { laneModels[.library]?.items ?? [] }
    var artists: [MediaItem] { laneModels[.artists]?.items ?? [] }
    var channels: [MediaItem] { laneModels[.channels]?.items ?? [] }
    var programmes: [MediaItem] { laneModels[.programmes]?.items ?? [] }
    var isEmpty: Bool { laneModels.values.allSatisfy { $0.window.nextIndex == 0 } }

    func load(query: String, store: LibraryStore, filters: UnifiedSearchFilters = .init()) async {
        let identity = UnifiedSearchIdentity(query: query, filters: filters, account: store.accountContext)
        if loadedIdentity == identity, !laneModels.isEmpty { return }
        cancel()
        loadedIdentity = identity
        let current = generation, context = store.accountContext
        let request = UnifiedSearchRequest(query: query, filters: filters)
        self.filters = filters; loadedQuery = request.query
        loadedContext = nil; laneModels = [:]; loading = request.isActive
        guard request.isActive else { return }
        await store.loadSearchCapabilities()
        guard generation == current, store.accountContext == context, !Task.isCancelled else { return }
        laneModels = Dictionary(uniqueKeysWithValues: request.lanes.map { ($0, PagedMediaModel()) })
        loadedContext = context
        defer { if generation == current { loading = false } }
        await withTaskGroup(of: Void.self) { group in
            for (lane, model) in laneModels {
                group.addTask {
                    await model.start { start, limit in
                        try Task.checkCancellation()
                        guard store.accountContext == context else { throw CancellationError() }
                        return try await Self.fetch(request, lane: lane, start: start, limit: limit, store: store, context: context)
                    }
                }
            }
            await group.waitForAll()
        }
        guard generation == current, store.accountContext == context, !Task.isCancelled else { return }
    }

    func loadMore(_ lane: UnifiedSearchLane, store: LibraryStore) {
        guard loadedContext == store.accountContext else { return }
        laneModels[lane]?.loadNext()
    }

    func cancel() {
        generation = UUID()
        laneModels.values.forEach { $0.cancel() }
        loading = false
    }

    private static func fetch(_ request: UnifiedSearchRequest, lane: UnifiedSearchLane, start: Int, limit: Int,
                              store: LibraryStore, context: UUID) async throws -> ItemPage {
        try Task.checkCancellation()
        guard store.accountContext == context else { throw CancellationError() }
        if store.preview {
            switch lane {
            case .library:
                return try await store.items(query: request.query, start: start,
                    includeItemTypes: request.filters.scope.previewTypes, limit: limit, filters: request.filters)
            case .artists:
                return try await store.musicItems(type: "MusicArtist", start: start, query: request.query, limit: limit)
            case .people, .programmes, .channels:
                return .init(Items: [], TotalRecordCount: 0)
            }
        }
        if !request.query.isEmpty, request.filters.sort != .relevance, !store.supportsExplicitSearchSort {
            throw BluefinError.message("This server does not support sorting search results. Choose Relevance.")
        }
        var call = request.call(for: lane, start: start, limit: limit)
        if request.query.isEmpty { call.fields.removeValue(forKey: "searchOrder") }
        let page: ItemPage = try await store.authenticated(call.operation, fields: call.fields)
        try Task.checkCancellation()
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

struct SearchLaneRows<Row: View>: View {
    let title: String
    @ObservedObject var model: PagedMediaModel
    @ViewBuilder var row: (MediaItem) -> Row
    @ScaledMetric(relativeTo: .body) private var rowHeight: CGFloat = 78
    var body: some View {
        Section(title) {
            ForEach(model.window.pages) { page in
                ForEach(page.range, id: \.self) { index in
                    Group {
                        if model.window.isDuplicate(at: index) {
                            Color.clear.accessibilityHidden(true)
                        } else if let item = model.window.item(at: index) {
                            row(item)
                        } else if let problem = model.failures[page.start] {
                            MediaLoadingRow(message: problem, retry: { model.retry(page.start) })
                        } else if page.items != nil {
                            Text("This title is no longer available.").foregroundStyle(.secondary)
                        } else {
                            MediaLoadingRow()
                        }
                    }
                    .frame(height: rowHeight, alignment: .leading)
                    .onAppear { model.appear(index) }
                    .onDisappear { model.disappear(index) }
                }
            }
            if let problem = model.failures[model.window.nextIndex] {
                MediaLoadingRow(message: problem, retry: { model.retry(model.window.nextIndex) })
            } else if model.loading {
                ProgressView("Loading results")
            } else if model.window.nextIndex > 0 && !model.window.hasMore {
                Text("All results loaded").font(.caption).foregroundStyle(.secondary)
            }
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
        _filters = State(initialValue: .init(scope: initialScope == .libraryOnly ? .all : initialScope,
            source: initialScope == .libraryOnly ? .library : .allSources))
    }
    private var identity: UnifiedSearchIdentity {
        .init(query: query, filters: filters, account: store.accountContext)
    }
    private var active: Bool { !query.isEmpty || filters.scope != .all || filters.libraryOnly }
    private var current: Bool {
        model.filters == filters && model.loadedContext == store.accountContext &&
        model.loadedQuery == query.trimmingCharacters(in: .whitespacesAndNewlines)
    }
    var body: some View {
        List {
            Section {
                scopePicker
                Picker("Sources", selection: Binding(get: { filters.source }, set: { source in
                    if source == .allSources { filters = .init(scope: filters.scope, source: source) }
                    else { filters.source = source }
                })) {
                    ForEach(UnifiedSearchSource.allCases) { Text($0.title).tag($0) }
                }.accessibilityIdentifier("searchSources")
                if filters.source == .library {
                    Picker("Sort", selection: $filters.sort) {
                        ForEach(LibrarySearchSort.allCases.filter { query.isEmpty || store.supportsExplicitSearchSort || $0 == .relevance }) {
                            Text($0.title).tag($0)
                        }
                    }.accessibilityIdentifier("searchSort")
                    if !query.isEmpty && !store.supportsExplicitSearchSort {
                        Text("This server supports relevance search. Other search orders are unavailable.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                } else {
                    HStack { Text("Sort"); Spacer(); Text("Relevance").foregroundStyle(.secondary) }
                }
                Button { showFilters.toggle() } label: {
                    Label(filters.hasFacets ? "Refine search (\(filters.count))" : "Refine search",
                          systemImage: "line.3.horizontal.decrease.circle")
                }.accessibilityIdentifier("searchRefinements")
                if showFilters { refinementControls }
                if filters.hasFacets { activeFilterChips }
                if filters.libraryOnly {
                    HStack {
                        Text("Showing matching titles in your library.").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Clear") { filters = .init(scope: filters.scope, source: filters.source) }
                    }
                }
            }
            if active {
                if current {
                    ForEach(UnifiedSearchLane.allCases, id: \.self) { lane in
                        if let page = model.laneModels[lane] {
                            SearchLaneRows(title: lane.title, model: page) { item in result(item) }
                        }
                    }
                }
                if model.loading && !current { ProgressView("Searching") }
                if !model.loading && current && model.isEmpty {
                    Text(filters.hasFacets ? "No titles match these refinements." : "No matching titles in your connected library or guide.")
                        .foregroundStyle(.secondary)
                }
                if current && !query.isEmpty {
                    LocalAssistanceSearchSection(query: query, candidates: model.items + model.artists,
                                                 loadedContext: model.loadedContext)
                        .id(identity)
                }
                if !query.isEmpty && !filters.libraryOnly {
                    if filters.scope.includesCatalog { ServerDiscoverySection(query: query, scope: filters.scope) }
                    if filters.scope == .all || filters.scope == .music { ServerDiscoverySection(query: query, scope: .music) }
                }
                if !store.preview && query.count >= 2 && !filters.libraryOnly {
                    if filters.scope == .all || filters.scope == .tvShows {
                        PublicTitleSearchSection(query: query, catalog: .television)
                    }
                    if filters.scope == .all || filters.scope == .movies {
                        PublicTitleSearchSection(query: query, catalog: .film, library: current ? model.items : [])
                    }
                    if filters.scope.includesArtists {
                        PublicMusicSearchSection(query: query, kind: "artist", library: current ? model.items : [])
                        PublicMusicSearchSection(query: query, kind: "releaseGroup", library: current ? model.items : [])
                        PublicMusicSearchSection(query: query, kind: "recording", library: current ? model.items : [])
                    }
                }
            } else {
                Section {
                    Label("Find your next favorite", systemImage: "magnifyingglass").font(.headline)
                    Text("Search your connected media and public catalogs. Choose Your library to search only your connected media.")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .id(identity)
        .task(id: identity) { await model.load(query: query, store: store, filters: filters) }
        .task(id: showFilters ? store.accountContext : nil) {
            if showFilters { await model.loadFacets(store: store) }
        }
        .onChange(of: filters.hasFacets) { hasFacets in
            if hasFacets { filters.source = .library }
        }
        .onChange(of: store.supportsExplicitSearchSort) { supported in
            if !supported && !query.isEmpty { filters.sort = .relevance }
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
            ForEach(UnifiedSearchScope.allCases.filter { $0 != .libraryOnly }) { scope in Text(scope.title).tag(scope) }
        }
    }
    private func selection(_ keyPath: WritableKeyPath<UnifiedSearchFilters, String?>) -> Binding<String> {
        Binding(get: { filters[keyPath: keyPath] ?? "" },
                set: { filters[keyPath: keyPath] = $0.isEmpty ? nil : $0 })
    }
    private func filterChip(_ title: String, clear: @escaping () -> Void) -> some View {
        Button(action: clear) { Label(title, systemImage: "xmark.circle.fill") }
            .font(.caption).buttonStyle(.bordered).accessibilityLabel("Remove filter: " + title)
    }
    private var activeFilterChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack {
                if filters.watched != .all { filterChip(filters.watched.title) { filters.watched = .all } }
                if filters.favoritesOnly { filterChip("Favorites") { filters.favoritesOnly = false } }
                if let genre = filters.genre { filterChip(genre) { filters.genre = nil } }
                if let year = filters.year { filterChip(String(year)) { filters.year = nil } }
                if let tag = filters.tag { filterChip(tag) { filters.tag = nil } }
                if let id = filters.studioID {
                    filterChip(model.facets.studios.first { $0.id == id }?.name ?? "Studio") { filters.studioID = nil }
                }
                if let id = filters.collectionID {
                    filterChip(model.facets.collections.first { $0.id == id }?.name ?? "Collection") { filters.collectionID = nil }
                }
            }
        }
    }
    @ViewBuilder private var refinementControls: some View {
        Picker("Watch status", selection: $filters.watched) {
            ForEach(LibraryWatchFilter.allCases) { Text($0.title).tag($0) }
        }
        Toggle("Favorites only", isOn: $filters.favoritesOnly)
        if model.facetsLoading { ProgressView("Loading refinements") }
        if let genres = model.facets.genres, !genres.isEmpty {
            Picker("Genre", selection: selection(\.genre)) {
                Text("All genres").tag("")
                ForEach(genres, id: \.self) { Text($0).tag($0) }
            }
        }
        if let years = model.facets.years, !years.isEmpty {
            Picker("Release year", selection: Binding(get: { filters.year ?? 0 }, set: { filters.year = $0 == 0 ? nil : $0 })) {
                Text("All years").tag(0)
                ForEach(years, id: \.self) { Text(String($0)).tag($0) }
            }
        }
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
        if model.facets.truncated == true {
            Text("Some refinements are available by opening their collection.")
                .font(.footnote).foregroundStyle(.secondary)
        }
        if let problem = model.facetsProblem {
            Text(problem).font(.footnote).foregroundStyle(.secondary)
            Button("Reload refinements") { Task { await model.loadFacets(store: store) } }
        }
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
        } else {
            resultLink(item) {
                HStack(spacing: 14) {
                    Artwork(item: item, thumbnailWidth: 128)
                        .aspectRatio(2 / 3, contentMode: .fit).frame(width: 42, height: 63)
                        .clipShape(RoundedRectangle(cornerRadius: 6)).accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.displayTitle).font(.headline).lineLimit(1)
                        if !item.displaySubtitle.isEmpty {
                            Text(item.displaySubtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                }
            }
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
            var ids: [Entry] = []
            if let value = provider("Tmdb"),
               value.range(of: #"^[1-9][0-9]{0,11}$"#, options: .regularExpression) != nil {
                ids.append(Entry(source: "tmdb", id: value))
            }
            if let value = provider("Imdb"),
               value.range(of: #"^tt[0-9]{7,12}$"#, options: .regularExpression) != nil {
                ids.append(Entry(source: "imdb", id: value))
            }
            guard !ids.isEmpty else { return nil }
            return Self(kind: item.Type == "Movie" ? "movie" : "series", ids: ids)
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
    var expired: Bool {
        evidence.expiresAt.map { Date(timeIntervalSince1970: TimeInterval($0)) <= Date() } ?? false
    }
    var activeDestinations: [Destination] {
        guard !expired else { return [] }
        return destinations.filter {
            $0.url.scheme?.lowercased() == "https" && $0.url.host != nil &&
                $0.url.user == nil && $0.url.password == nil
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
struct PublicCatalogAccount: Hashable, Sendable {
    let generation: UUID
    let paired: Bool
}
struct PublicCatalogAccessRequest: Hashable, Sendable {
    let identity: PublicCatalogIdentity
    let account: PublicCatalogAccount
    let country: String
    let attempt: Int
}
@MainActor enum PublicCatalogAPI {
    static func account(_ store: LibraryStore) -> PublicCatalogAccount {
        .init(generation: store.accountContext, paired: store.session != nil && !store.preview)
    }
    private static func lookup<T: Decodable & Sendable>(
        _ operation: String, fields: [String: Any], store: LibraryStore,
        account: PublicCatalogAccount
    ) async throws -> T {
        guard account == Self.account(store) else { throw CancellationError() }
        let result: T
        if account.paired {
            result = try await store.authenticated(operation, fields: fields)
        } else {
            result = try await RustBridge.call(operation, fields: fields)
        }
        try Task.checkCancellation()
        guard account == Self.account(store) else { throw CancellationError() }
        return result
    }
    static func offers(_ identity: PublicCatalogIdentity, country: String?,
                       store: LibraryStore, account: PublicCatalogAccount) async throws -> PublicCatalogOffers {
        var fields: [String: Any] = ["identity": identity.fields]
        let selected = country.flatMap { $0.isEmpty ? nil : $0.uppercased() }
        if let selected { fields["country"] = selected }
        let result: PublicCatalogOffers = try await lookup(
            "publicCatalogOffers", fields: fields, store: store, account: account)
        guard result.schemaVersion == 1, result.identity == identity, result.country == selected,
              result.offers.allSatisfy({ $0.identity == identity }) else {
            throw BluefinError.message("Availability returned a different media identity.")
        }
        return result
    }
    static func countries(_ identity: PublicCatalogIdentity, store: LibraryStore,
                          account: PublicCatalogAccount) async throws -> PublicCatalogCountries {
        try await lookup("publicCatalogCountries", fields: ["identity": identity.fields],
                         store: store, account: account)
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
                ForEach(source.attributions, id: \.self) { credit in
                    if credit.url.scheme?.lowercased() == "https", credit.url.host != nil,
                       credit.url.user == nil, credit.url.password == nil {
                        Link(credit.name, destination: credit.url).font(.caption)
                    }
                }
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
    @EnvironmentObject private var store: LibraryStore
    @AppStorage("publicCatalogCountry") private var country = PublicCatalogRegion.initial
    @State private var offers: [PublicCatalogOffer] = []
    @State private var observations: [Observation] = []
    @State private var attempt = 0
    @State private var loading = false
    @State private var total = 0
    @State private var completed = 0
    @State private var message: String?
    @State private var loadedFor: PublicCatalogAccessRequest?
    private var requestKey: PublicCatalogAccessRequest {
        .init(identity: identity, account: PublicCatalogAPI.account(store), country: country, attempt: attempt)
    }
    private var activeOffers: [PublicCatalogOffer] { loadedFor == requestKey ? offers : [] }
    private var activeObservations: [Observation] { loadedFor == requestKey ? observations : [] }
    private var activeLoading: Bool { loadedFor != requestKey || loading }
    var body: some View {
        Section(identity.music ? "Listen or buy" : "Where to watch") {
            Picker("Region", selection: $country) {
                Text("All reported regions").tag("")
                ForEach(PublicCatalogRegion.codes, id: \.self) { code in Text(PublicCatalogRegion.name(code)).tag(code) }
            }
            if activeLoading {
                if loadedFor == requestKey && total > 0 { ProgressView("Checking regions \(completed) of \(total)", value: Double(completed), total: Double(total)) }
                else { ProgressView("Checking provider availability") }
            }
            if loadedFor == requestKey, let message { Text(message).font(.caption).foregroundStyle(.secondary) }
            ForEach(activeOffers) { offer in
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
                    ForEach(offer.activeDestinations) { destination in
                        Link((destination.kind == "player" ? "Open player on " : "Open on ") + offer.service.name, destination: destination.url)
                    }
                    if offer.expired { Text("This reported offer has expired.").font(.caption) }
                    else if offer.activeDestinations.isEmpty {
                        Text("Source reports availability without a direct destination.").font(.caption)
                    }
                    if offer.evidence.observedAt > 0 {
                        Text("Reported " + Date(timeIntervalSince1970: TimeInterval(offer.evidence.observedAt)).formatted(date: .abbreviated, time: .shortened))
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
            if !activeLoading && activeOffers.isEmpty {
                Text(activeObservations.contains { !["complete", "noOffers", "notFound"].contains($0.value.status) }
                    ? "Availability could not be confirmed for every source. Check coverage or retry."
                    : "No offers were reported for this selection.").foregroundStyle(.secondary)
            }
            if !activeLoading { Button("Refresh availability") { attempt += 1 } }
            DisclosureGroup("Sources and coverage") {
                ForEach(activeObservations) { observation in
                    if let code = observation.country { Text(PublicCatalogRegion.name(code)).font(.caption.bold()) }
                    PublicCatalogCoverage(sources: [observation.value])
                }
                Text("Provider links do not confirm your account's playback rights.").font(.caption).foregroundStyle(.secondary)
            }
        }
        .task(id: requestKey) {
            let requested = requestKey
            loadedFor = requested
            offers = []; observations = []; message = nil; completed = 0; total = 0; loading = true
            let selected = requested.country
            let catalogStore = store
            defer { if !Task.isCancelled && requested == requestKey { loading = false } }
            do {
                // The paired instance supplies all reported regions in one
                // country-less request. A provider registry is not offer evidence.
                if requested.account.paired || !selected.isEmpty || requested.identity.music {
                    let result = try await PublicCatalogAPI.offers(
                        requested.identity, country: selected.isEmpty ? nil : selected,
                        store: catalogStore, account: requested.account)
                    try Task.checkCancellation()
                    guard requested == requestKey else { return }
                    merge(result, requested: requested)
                    if requested.identity.music && selected.isEmpty && !requested.account.paired {
                        message = "MusicBrainz links do not include regional availability. Select a country to check exact store listings."
                    }
                    return
                }
                let registry = try await PublicCatalogAPI.countries(
                    requested.identity, store: catalogStore, account: requested.account)
                try Task.checkCancellation()
                guard requested == requestKey else { return }
                observations.append(contentsOf: registry.sources.map { Observation(country: nil, value: $0) })
                var seen = Set<String>()
                let regions = registry.countries.map(\.code).filter { seen.insert($0).inserted }
                total = regions.count
                guard !regions.isEmpty else {
                    message = "This source did not supply a country list."
                    return
                }
                try await withThrowingTaskGroup(of: PublicCatalogOffers.self) { group in
                    var next = 0
                    for region in regions.prefix(3) {
                        next += 1
                        group.addTask {
                            try await PublicCatalogAPI.offers(requested.identity, country: region,
                                                              store: catalogStore, account: requested.account)
                        }
                    }
                    while let result = try await group.next() {
                        try Task.checkCancellation()
                        guard requested == requestKey else { group.cancelAll(); return }
                        merge(result, requested: requested)
                        completed += 1
                        if result.sources.contains(where: { $0.interrupted }) {
                            message = "The source paused or failed. Loaded regions remain visible; refresh to continue."
                            group.cancelAll()
                            break
                        }
                        if next < regions.count && !Task.isCancelled {
                            let region = regions[next]; next += 1
                            group.addTask {
                                try await PublicCatalogAPI.offers(requested.identity, country: region,
                                                                  store: catalogStore, account: requested.account)
                            }
                        }
                    }
                }
            } catch is CancellationError {
            } catch {
                guard !Task.isCancelled && requested == requestKey else { return }
                message = "Provider availability is temporarily unavailable."
            }
        }
    }
    private func merge(_ response: PublicCatalogOffers, requested: PublicCatalogAccessRequest) {
        guard requested == requestKey, loadedFor == requested,
              response.schemaVersion == 1, response.identity == requested.identity else { return }
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
