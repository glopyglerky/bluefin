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
    var libraryOnly: Bool { source == .library || hasFacets || sort != .relevance }
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
    private var loadedRequest: UnifiedSearchRequest?
    private var completedInitialLanes = Set<UnifiedSearchLane>()
    typealias FetchPage = @MainActor (UnifiedSearchRequest, UnifiedSearchLane, Int, Int, LibraryStore, UUID) async throws -> ItemPage
    private let fetchPage: FetchPage
    init(fetchPage: FetchPage? = nil) { self.fetchPage = fetchPage ?? Self.fetch }
    private var unfinishedInitialLanes: [UnifiedSearchLane: PagedMediaModel] {
        laneModels.filter {
            !completedInitialLanes.contains($0.key) && $0.value.window.nextIndex == 0 && $0.value.failures[0] == nil
        }
    }
    var pages: [UnifiedSearchLane: UnifiedSearchPage] { laneModels.mapValues(\.window) }
    var items: [MediaItem] { laneModels[.library]?.items ?? [] }
    var artists: [MediaItem] { laneModels[.artists]?.items ?? [] }
    var channels: [MediaItem] { laneModels[.channels]?.items ?? [] }
    var programmes: [MediaItem] { laneModels[.programmes]?.items ?? [] }
    var isEmpty: Bool { laneModels.values.allSatisfy { $0.window.nextIndex == 0 } }

    func load(query: String, store: LibraryStore, filters: UnifiedSearchFilters = .init()) async {
        let identity = UnifiedSearchIdentity(query: query, filters: filters, account: store.accountContext)
        let resuming = loadedIdentity == identity && !laneModels.isEmpty
        if resuming && unfinishedInitialLanes.isEmpty { return }
        // A view can disappear before the first page returns. Retain completed
        // lanes, but fence the canceled batch and restart its unfinished lanes.
        cancel()
        if !resuming {
            loadedIdentity = identity
            loadedRequest = UnifiedSearchRequest(query: query, filters: filters)
            self.filters = filters; loadedQuery = loadedRequest?.query ?? ""
            loadedContext = nil; laneModels = [:]; completedInitialLanes = []
        }
        guard let request = loadedRequest, request.isActive else { return }
        let current = generation, context = store.accountContext
        loading = true
        defer { if generation == current { loading = false } }
        await store.loadSearchCapabilities()
        guard generation == current, store.accountContext == context, !Task.isCancelled else { return }
        if !resuming {
            laneModels = Dictionary(uniqueKeysWithValues: request.lanes.map { ($0, PagedMediaModel()) })
        }
        loadedContext = context
        let pending = unfinishedInitialLanes
        await withTaskGroup(of: Void.self) { group in
            for (lane, model) in pending {
                group.addTask {
                    await self.startLane(lane, model: model, request: request, store: store,
                                         context: context, generation: current)
                }
            }
            await group.waitForAll()
        }
    }

    private func startLane(_ lane: UnifiedSearchLane, model: PagedMediaModel, request: UnifiedSearchRequest,
                           store: LibraryStore, context: UUID, generation current: UUID) async {
        var finished = false
        let fetchPage = self.fetchPage
        await model.start { start, limit in
            try Task.checkCancellation()
            guard store.accountContext == context else { throw CancellationError() }
            do {
                let page = try await fetchPage(request, lane, start, limit, store, context)
                try Task.checkCancellation()
                guard store.accountContext == context else { throw CancellationError() }
                finished = true
                return page
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                finished = true
                throw error
            }
        }
        guard finished, generation == current, store.accountContext == context, !Task.isCancelled else { return }
        // Successful empty pages and visible error/retry states are completed
        // attempts too; only interrupted first loads should start again on return.
        completedInitialLanes.insert(lane)
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
                }
            } else {
                Section {
                    Label("Find your next favorite", systemImage: "magnifyingglass").font(.headline)
                    Text("Search your movies, shows, music and live TV.").foregroundStyle(.secondary)
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
                TextField("Movie or TV show title", text: $query)
                    .accessibilityIdentifier("publicCatalogQuery")
                PublicTitleSearchSection(query: query, catalog: .television)
                PublicTitleSearchSection(query: query, catalog: .film)
            }
            .navigationTitle("Search movies and shows")
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
        let sourceUrl: URL
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
                            VStack(alignment: .leading, spacing: 8) {
                                Link(destination: item.sourceUrl) {
                                    HStack(spacing: 14) {
                                        AsyncImage(url: item.artwork) { image in
                                            image.resizable().scaledToFill()
                                        } placeholder: {
                                            Image(systemName: catalog.icon).foregroundStyle(.secondary)
                                        }
                                        .frame(width: 46, height: 64).clipped().accessibilityHidden(true)
                                        VStack(alignment: .leading, spacing: 4) {
                                            Text(item.title).font(.headline)
                                            Text([item.description ?? item.year, catalog.provider].compactMap { $0 }.joined(separator: " · "))
                                                .font(.caption).foregroundStyle(.secondary)
                                        }
                                    }
                                }.accessibilityHint("Open title information on \(catalog.provider)")
                                if catalog == .film {
                                    PublicFilmProviderLinks(catalogID: item.sourceUrl.lastPathComponent, library: library)
                                }
                            }
                        }
                        Text("Streaming availability for these results is unverified.")
                            .font(.caption).foregroundStyle(.secondary)
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

private struct PublicFilmProviderLinks: View {
    struct Result: Decodable, Sendable {
        let checked: Bool
        let offers: [Offer]
        let sources: [URL]
    }
    struct Offer: Decodable, Identifiable, Sendable {
        let id: String
        let currency: String
        let rent: Double?
        let buy: Double?
        let destination: URL
    }
    struct Request: Hashable {
        let region: String
        let attempt: Int
    }
    struct Identity: Decodable, Sendable { let imdbIds: [String] }

    let catalogID: String
    let library: [MediaItem]
    @EnvironmentObject private var store: LibraryStore
    @State private var attempt = 0
    @State private var region = (Locale.current.regionCode ?? "US").uppercased()
    @State private var completed: Request?
    @State private var result: Result?
    @State private var identities: Set<String> = []
    private var request: Request { Request(region: region, attempt: attempt) }
    private static let regions = Locale.isoRegionCodes.compactMap { code in
        Locale.current.localizedString(forRegionCode: code).map { (code: code, name: $0) }
    }.sorted { $0.name.localizedCompare($1.name) == .orderedAscending }

    var body: some View {
        Group {
            if attempt == 0 {
                Button("Where to watch") { attempt += 1 }
            } else {
                ForEach(library.filter { media in
                    media.Type == "Movie" && (media.ProviderIds?["Imdb"] ?? media.ProviderIds?["IMDb"]).map(identities.contains) == true
                }) { media in
                    Button(media.resumeTicks > 0 ? "Resume in Bluefin" : "Play in Bluefin") {
                        Task { await store.play(media) }
                    }
                }
                Picker("Storefront region", selection: $region) {
                    ForEach(Self.regions, id: \.code) { choice in Text(choice.name).tag(choice.code) }
                }
                if completed != request {
                    ProgressView("Checking availability")
                } else if let result {
                    if result.checked {
                        Text("Apple storefront · \(Locale.current.localizedString(forRegionCode: region) ?? region)")
                        if result.offers.isEmpty {
                            Text("No rent or buy offer returned for this region.").foregroundStyle(.secondary)
                        }
                        ForEach(result.offers) { offer in
                            VStack(alignment: .leading, spacing: 4) {
                                if let rent = offer.rent {
                                    Text("Rent \(rent.formatted(.currency(code: offer.currency)))")
                                }
                                if let buy = offer.buy {
                                    Text("Buy \(buy.formatted(.currency(code: offer.currency)))")
                                }
                                Link("Continue to Apple TV", destination: offer.destination)
                            }
                        }
                    } else {
                        Text("No regional offers could be verified for this film.").foregroundStyle(.secondary)
                        ForEach(result.sources, id: \.self) { destination in
                            Link("Open title in Apple TV", destination: destination)
                        }
                    }
                } else {
                    Text("The Apple storefront could not be checked.").foregroundStyle(.secondary)
                    Button("Retry availability") { attempt += 1 }
                }
            }
        }
        .font(.caption)
        .task(id: attempt > 0 && !library.isEmpty) {
            guard attempt > 0, !library.isEmpty else { identities = []; return }
            do {
                let identity: Identity = try await RustBridge.call("publicFilmIdentity", fields: ["id": catalogID])
                try Task.checkCancellation()
                identities = Set(identity.imdbIds)
            } catch is CancellationError {} catch {
                guard !Task.isCancelled else { return }
                identities = []
            }
        }
        .task(id: request) {
            let current = request
            guard current.attempt > 0 else { return }
            do {
                let response: Result = try await RustBridge.call("publicFilmSources", fields: [
                    "id": catalogID, "region": current.region
                ])
                try Task.checkCancellation()
                result = response
                completed = current
            } catch is CancellationError {} catch {
                guard !Task.isCancelled else { return }
                result = nil
                completed = current
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
        guard let start = programme.start, let end = programme.end else { return false }
        let now = Date()
        return start <= now && now < end
    }

    var body: some View {
        Group {
            if let channel { LiveChannelView(channel: channel) }
            else {
                VStack(alignment: .leading, spacing: 16) {
                    Text(programme.title).font(.title.bold())
                    if let start = programme.start {
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
