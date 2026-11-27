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
    var hasFacets: Bool { studioID != nil || tag != nil || collectionID != nil }
    var count: Int { [studioID, tag, collectionID].compactMap { $0 }.count }
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
        if filters.scope == .people { return [.people] }
        var values: [UnifiedSearchLane] = [.library]
        if filters.scope.includesArtists { values.append(.artists) }
        if filters.scope == .all { values.append(.people) }
        if filters.scope.includesChannels { values += [.programmes, .channels] }
        return values
    }
    var isActive: Bool { !query.isEmpty || filters.scope != .all || filters.hasFacets }
    func call(for lane: UnifiedSearchLane, start: Int) -> (operation: String, fields: [String: Any]) {
        var fields: [String: Any] = ["startIndex": start, "limit": 60]
        if !query.isEmpty { fields["searchTerm"] = query }
        switch lane {
        case .library:
            fields["searchScope"] = filters.scope == .libraryOnly || filters.scope == .people ? "all" : filters.scope.rawValue
            if let id = filters.studioID { fields["studioIds"] = [id] }
            if let tag = filters.tag { fields["tags"] = [tag] }
            if let id = filters.collectionID { fields["parentId"] = id }
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
            case .people, .programmes, .channels:
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
    @State private var publicSearch: UnifiedSearchIdentity?
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
                if !store.preview && query.count >= 2 && !filters.hasFacets &&
                    [.all, .tvShows, .movies].contains(filters.scope) {
                    if publicSearch == identity {
                        if filters.scope == .all || filters.scope == .tvShows {
                            PublicTitleSearchSection(query: query, catalog: .television)
                        }
                        if filters.scope == .all || filters.scope == .movies {
                            PublicTitleSearchSection(query: query, catalog: .film, library: current ? model.items : [])
                        }
                    } else {
                        Button {
                            publicSearch = identity
                        } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(filters.scope == .tvShows ? "Search public TV titles on TVmaze" :
                                     filters.scope == .movies ? "Search public movies on Wikidata" :
                                     "Search public movies and TV shows")
                                if filters.scope == .all {
                                    Text("Send this search to TVmaze and Wikidata.").font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
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
                    Text("Search your connected media. You can also choose to search public movies and TV shows.")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .task(id: identity) { await model.load(query: query, store: store, filters: filters) }
        .task(id: showFilters ? store.accountContext : nil) {
            if showFilters { await model.loadFacets(store: store) }
        }
        .onChange(of: identity) { _ in publicSearch = nil }
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
