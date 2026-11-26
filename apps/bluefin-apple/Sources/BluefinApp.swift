import SwiftUI
#if os(iOS)
import CoreSpotlight
#endif

@main struct BluefinApp: App {
    @StateObject private var store = LibraryStore()
    @AppStorage("bluefin.appearance") private var appearance = 0
    init() {
        let initialStore = LibraryStore()
        _store = StateObject(wrappedValue: initialStore)
        NativeIntegrations.attach(to: initialStore)
    }
    var body: some Scene {
        WindowGroup {
            RootView().environmentObject(store).tint(BluefinStyle.accent).preferredColorScheme(appearance == 1 ? .light : appearance == 2 ? .dark : nil)
                #if targetEnvironment(macCatalyst)
                .frame(minWidth: 680, minHeight: 480)
                #endif
        }
        #if targetEnvironment(macCatalyst)
        .commands { BluefinMacCommands(store: store) }
        #elseif os(visionOS)
        .defaultSize(width: 1100, height: 760)
        .windowResizability(.contentMinSize)
        #endif
    }
}

struct RootView: View {
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var nearby = MeshNearbyBrowser()
    var body: some View {
        Group {
            if store.signedIn { LibraryTabs() }
            else { TVWelcomeView() }
        }
        #if os(iOS) || os(visionOS)
        .background {
            NativeVideoPresentation(playback: store.playback.flatMap { $0.item.Type == "Audio" ? nil : $0 }) { playback in
                guard store.playback === playback else { return }
                store.dismissPlayback()
                Task { await store.refresh() }
            }.frame(width: 0, height: 0)
        }
        #else
        .fullScreenCover(item: Binding(get: { store.playback?.item.Type == "Audio" ? nil : store.playback }, set: { if $0 == nil && store.playback?.item.Type != "Audio" { store.dismissPlayback() } }), onDismiss: { Task { await store.refresh() } }) { PlaybackScreen(playback: $0).environmentObject(store) }
        #endif
        .sheet(isPresented: $store.musicExpanded) {
            if let playback = store.playback, playback.item.Type == "Audio" { MusicPlayer(playback: playback).environmentObject(store).id(playback.id) }
        }
        .environmentObject(nearby)
        .task {
            nearby.start()
            Task { await MeshRevocationQueue.shared.retry() }
            NativeIntegrations.attach(to: store)
            #if os(iOS)
            if #available(iOS 16.0, *) { BluefinAppShortcuts.updateAppShortcutParameters() }
            #endif
        }
        .sheet(isPresented: Binding(get: { store.meshInvitationCode != nil }, set: { if !$0 { store.meshInvitationCode = nil } })) {
            MeshPairingView(initialCode: store.meshInvitationCode).environmentObject(store).environmentObject(nearby)
        }
        .onOpenURL { url in
            guard url.scheme == "bluefin", url.host == "connect" else { return }
            guard !store.signedIn else {
                store.error = "Sign out before connecting another account."
                return
            }
            // The shared Rust parser validates the complete URL before any effect.
            store.meshInvitationCode = url.absoluteString
        }
        .onChange(of: scenePhase) { phase in
            if phase == .active { nearby.start(); Task { await MeshRevocationQueue.shared.retry() } }
            else if phase == .background { nearby.stop() }
            Task { await store.handleMeshScene(phase) }
        }
        .task(id: nearby.revision.uuidString + (store.session?.meshSessionId ?? "")) {
            await nearby.refresh(store.session)
        }
        .task(id: store.session.map(OfflineLibrary.identity)) { await store.offline.connect(store.session) }
        .task(id: store.session.map(OfflineLibrary.identity)) { await store.sports.connect(store.session) }
        #if BLUEFIN_SHAREPLAY
        .overlay(alignment: .bottom) { NativeSharePlayControls().padding() }
        #endif
        #if os(iOS)
        .onContinueUserActivity(CSSearchableItemActionType) { activity in
            Task { do { try await NativeIntegrations.shared.continueSpotlight(activity) } catch { store.error = error.localizedDescription } }
        }
        #endif
        .alert("Bluefin", isPresented: Binding(get: { store.error != nil }, set: { if !$0 { store.error = nil } })) {
            Button("OK") { store.error = nil }
        } message: { Text(store.error ?? "") }
    }
}

struct QuickConnectView: View {
    let server: String
    let allowHttp: Bool
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var code: String?
    @State private var problem: String?
    @State private var retry = UUID()
    var body: some View {
        NavigationView {
            VStack(spacing: 24) {
                Image(systemName: "key.viewfinder").font(.system(size: 50, weight: .light)).foregroundColor(.cyan).accessibilityHidden(true)
                Text("Quick Connect").font(.title2.bold())
                Text("Enter this code in another client signed in to your server.").multilineTextAlignment(.center).foregroundStyle(.secondary)
                if let code { Text(code).font(.system(size: 42, weight: .semibold, design: .monospaced)).tracking(6).accessibilityLabel("Quick Connect code, \(code.map(String.init).joined(separator: ", "))").accessibilityIdentifier("quickConnectCode") }
                if let problem { Text(problem).foregroundColor(.red).multilineTextAlignment(.center); Button("Try again") { retry = UUID() } }
                else { ProgressView(code == nil ? "Getting a code" : "Waiting for your approval") }
                Button("Cancel") { dismiss() }
            }.padding(32).frame(maxWidth: quickConnectWidth).frame(maxWidth: .infinity, maxHeight: .infinity)
                .navigationTitle("Quick Connect")
        }.navigationViewStyle(.stack)
        .task(id: retry) { await connect() }
    }
    private var quickConnectWidth: CGFloat {
        #if os(tvOS)
        920
        #else
        550
        #endif
    }
    private func connect() async {
        problem = nil; code = nil
        let fields: [String: Any] = ["server": server, "allowHttp": allowHttp, "deviceId": store.deviceId]
        do {
            let challenge: QuickConnectChallenge = try await RustBridge.call("quickConnectInitiate", fields: fields)
            try Task.checkCancellation()
            code = challenge.Code
            for _ in 0..<100 {
                try await Task.sleep(nanoseconds: 3_000_000_000)
                var request = fields; request["secret"] = challenge.Secret
                let status: QuickConnectChallenge = try await RustBridge.call("quickConnectStatus", fields: request)
                if status.Authenticated == true {
                    let result: LoginResult = try await RustBridge.call("quickConnectAuthenticate", fields: request)
                    try await store.acceptLogin(result, server: server, allowHttp: allowHttp)
                    dismiss()
                    return
                }
            }
            problem = "This code has expired. Request a new one to sign in."
        } catch is CancellationError {} catch { problem = error.localizedDescription }
    }
}

struct LibraryTabs: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var selectedTab = 0
    #if os(tvOS)
    @State private var topShelfItem: MediaItem?
    @State private var topShelfRequest: URL?
    @State private var homeVisit = UUID()
    @State private var previousTab = 0
    #endif
    var body: some View {
        Group {
            #if os(iOS)
            if #available(iOS 26.1, *) {
                tabs.tabViewBottomAccessory(isEnabled: store.playback?.item.Type == "Audio") {
                    if let playback = store.playback, playback.item.Type == "Audio" {
                        MusicNativeAccessory(playback: playback)
                    }
                }
            } else if #available(iOS 26, *) {
                tabs.tabViewBottomAccessory {
                    if let playback = store.playback, playback.item.Type == "Audio" { MusicNativeAccessory(playback: playback) }
                }
            } else { legacyPlayerTabs }
            #else
            legacyPlayerTabs
            #endif
        }
        #if os(iOS) && !targetEnvironment(macCatalyst)
        .onOpenURL { url in
            guard let route = WidgetRoute.admitted(url, authenticated: store.session != nil, preview: store.preview) else { return }
            selectedTab = route.tab
        }
        #elseif os(tvOS)
        .background { TVTopShelfSync(sports: store.sports) }
        .onChange(of: selectedTab) { next in
            if previousTab == 0 && next != 0 {
                topShelfItem = nil
                homeVisit = UUID()
            }
            previousTab = next
        }
        .onOpenURL { topShelfRequest = $0 }
        .task(id: topShelfRequest) {
            guard let url = topShelfRequest, store.session != nil, !store.preview,
                  let id = TopShelfSnapshot.itemID(for: url) else { return }
            defer { if topShelfRequest == url { topShelfRequest = nil } }
            do {
                let item = try await store.detail(MediaItem(Id: id))
                try Task.checkCancellation()
                selectedTab = 0
                topShelfItem = item
            } catch is CancellationError {} catch {
                store.error = "This title is no longer available."
            }
        }
        #endif
        .task {
            NativeIntegrations.attach(to: store)
            NativeIntegrations.shared.openLibrary = { selectedTab = 1 }
            if !store.loading { await store.refresh() }
        }
    }
    private var legacyPlayerTabs: some View {
        tabs.safeAreaInset(edge: .bottom, spacing: 0) {
            if let playback = store.playback, playback.item.Type == "Audio" { MusicMiniPlayer(playback: playback) }
        }
    }
    @ViewBuilder private var tabs: some View {
        #if os(iOS)
        if #available(iOS 18, *) {
            TabView(selection: $selectedTab) {
                Tab("Home", systemImage: "house", value: 0) { BluefinNavigation { HomeView(sports: store.sports) } }
                Tab("Library", systemImage: "square.stack", value: 1) { BluefinNavigation { LibrariesView() } }
                Tab("Music", systemImage: "music.note", value: 2) { BluefinNavigation { MusicLibraryView() } }
                Tab(value: 3, role: .search) { BluefinNavigation { SearchView() } }
                Tab("Live TV", systemImage: "dot.radiowaves.left.and.right", value: 4) { BluefinNavigation { LiveTVView() } }
            }.tabViewStyle(.sidebarAdaptable)
        } else { legacyTabs }
        #elseif os(tvOS)
        TabView(selection: $selectedTab) {
            BluefinNavigation { TVHomeView(shelfItem: $topShelfItem, sports: store.sports) }
                .id(homeVisit)
                .tabItem { Label("Home", systemImage: "house") }.tag(0)
            BluefinNavigation { TVVideoLibrary(kind: .movies) }
                .tabItem { Label("Movies", systemImage: "film") }.tag(1)
            BluefinNavigation { TVVideoLibrary(kind: .shows) }
                .tabItem { Label("TV Shows", systemImage: "tv") }.tag(5)
            BluefinNavigation { LiveGuideView() }
                .tabItem { Label("Live TV", systemImage: "dot.radiowaves.left.and.right") }.tag(4)
            BluefinNavigation { MusicLibraryView() }
                .tabItem { Label("Music", systemImage: "music.note") }.tag(2)
            BluefinNavigation { SearchView() }
                .tabItem { Label("Search", systemImage: "magnifyingglass") }.tag(3)
        }
        #else
        legacyTabs
        #endif
    }
    private var legacyTabs: some View {
        TabView(selection: $selectedTab) {
            BluefinNavigation { HomeView(sports: store.sports) }.tabItem { Label("Home", systemImage: "house") }.tag(0)
            BluefinNavigation { LibrariesView() }.tabItem { Label("Library", systemImage: "square.stack") }.tag(1)
            BluefinNavigation { MusicLibraryView() }.tabItem { Label("Music", systemImage: "music.note") }.tag(2)
            BluefinNavigation { SearchView() }.tabItem { Label("Search", systemImage: "magnifyingglass") }.tag(3)
            BluefinNavigation { LiveTVView() }.tabItem { Label("Live TV", systemImage: "dot.radiowaves.left.and.right") }.tag(4)
        }
    }
}

struct HomeView: View {
    @EnvironmentObject private var store: LibraryStore
    @ObservedObject var sports: SportsStore
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 30) {
                if store.preview { Label("Preview library", systemImage: "sparkles").font(.caption.bold()).foregroundStyle(.secondary).padding(.horizontal, 24) }
                if sports.saved?.preferences.sports_only != true {
                    if let featured = store.home.resume.first ?? store.home.latest.first { FeaturedItem(item: featured) }
                    if !store.home.resume.isEmpty { MediaShelf(title: "Continue watching", items: store.home.resume) }
                    if !store.home.nextUp.isEmpty { MediaShelf(title: "Up next", items: store.home.nextUp) }
                    if !store.home.latest.isEmpty { MediaShelf(title: store.homeFromCatalog ? "Browse your library" : "Recently added", items: store.home.latest) }
                }
                SportsHomeSection(sports: sports)
                if sports.loading && sports.saved?.preferences.sports_only == true { ProgressView("Loading sports").padding(24) }
                if sports.available, let failure = sports.failure {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("Sports couldn't refresh").font(.headline)
                        Text(failure).font(.footnote).foregroundStyle(.secondary)
                        Button("Try again") { Task { await sports.connect(store.session) } }
                    }.padding(.horizontal, 24)
                }
                if let issue = store.libraryProblem {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(issue).font(.callout).foregroundStyle(.secondary)
                        Button("Refresh library") { Task { await store.refresh() } }.disabled(store.loading)
                    }.padding(.horizontal, 24)
                }
                if store.home.latest.isEmpty && store.home.resume.isEmpty && store.home.nextUp.isEmpty
                    && sports.saved?.preferences.sports_only != true {
                    if store.loading { ProgressView("Loading your library").frame(maxWidth: .infinity).padding(40) }
                    else if store.libraryProblem == nil {
                        EmptyLibrary(title: "Browse your library", message: "Open Library to see your collections.", symbol: "square.stack")
                    }
                }
            }.padding(.vertical, 16)
        }
        .background(BluefinBackground())
        .navigationTitle(sports.saved?.preferences.sports_only == true ? "Sports" : "Home")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                NavigationLink(destination: SettingsView()) { Image(systemName: "person.crop.circle").font(.title3) }.accessibilityLabel("Account and settings")
            }
        }
        .bluefinRefresh { await store.refresh(); await sports.connect(store.session) }
    }
}

struct FeaturedItem: View {
    let item: MediaItem
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.dynamicTypeSize) private var dynamicType
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            ZStack(alignment: .bottomLeading) {
                Artwork(item: item, backdrop: true,
                        thumbnailWidth: sizeClass == .compact ? 1200 : nil)
                LinearGradient(colors: [.clear, .black.opacity(0.8)], startPoint: .center, endPoint: .bottom)
                VStack(alignment: .leading, spacing: 8) {
                    Text(item.progress > 0 ? "PICK UP WHERE YOU LEFT OFF" : "IN YOUR LIBRARY").font(.caption.weight(.semibold)).tracking(1.5)
                    Text(item.title).font(sizeClass == .compact ? .title.bold() : .largeTitle.bold()).fixedSize(horizontal: false, vertical: true)
                    if !item.subtitle.isEmpty { Text(item.subtitle).font(.subheadline).opacity(0.8) }
                }.foregroundColor(.white).padding(24)
            }
            .frame(height: dynamicType.isAccessibilitySize ? 390 : sizeClass == .compact ? 310 : 430)
            .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
            HStack(spacing: 16) {
                if !item.isContainer {
                    Button { Task { await store.play(item) } } label: { Label(item.resumeTicks > 0 ? "Resume" : "Play", systemImage: "play.fill").font(.body.weight(.semibold)).padding(.horizontal, 20).padding(.vertical, 6) }
                        .buttonStyle(.borderedProminent).disabled(store.startingPlayback)
                }
                MediaNavigationLink(destination: .detail(item)) { Label("Details", systemImage: "info.circle") }.buttonStyle(.bordered)
                if store.startingPlayback { ProgressView() }
            }
        }.padding(.horizontal, 24)
    }
}

struct MediaShelf: View {
    let title: String
    let items: [MediaItem]
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(title).font(.title2.bold()).padding(.horizontal, 24).accessibilityAddTraits(.isHeader)
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: 16) {
                    ForEach(items) { item in
                        MediaNavigationLink(destination: .detail(item)) { Poster(item: item).frame(width: posterWidth) }
                            .buttonStyle(.plain)
                    }
                }.padding(.horizontal, 24).padding(.vertical, 8)
            }
        }
    }
    private var posterWidth: CGFloat {
        #if os(tvOS)
        240
        #else
        152
        #endif
    }
}

struct Poster: View {
    let item: MediaItem
    #if os(tvOS)
    @Environment(\.isFocused) private var focused
    #endif
    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Artwork(item: item).aspectRatio(item.Type == "MusicAlbum" ? 1 : 2 / 3, contentMode: .fit).clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay(alignment: .bottom) { if item.progress > 0 { ProgressView(value: item.progress).tint(.cyan).padding(8).accessibilityHidden(true) } }
                #if os(tvOS)
                .overlay {
                    if focused {
                        RoundedRectangle(cornerRadius: 12).strokeBorder(.white, lineWidth: 3)
                    }
                }
                #endif
            Text(item.title).font(.subheadline.weight(.semibold)).lineLimit(2).foregroundStyle(.primary)
            if !item.subtitle.isEmpty { Text(item.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
        }.accessibilityElement(children: .ignore).accessibilityLabel(item.title + (item.subtitle.isEmpty ? "" : ", " + item.subtitle))
    }
}

struct LibrariesView: View {
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        List {
            Section("Collections") {
                if store.preview {
                    MediaNavigationLink(destination: .collection(title: "Sample movies", parent: nil, types: "Movie")) {
                        Label("Sample movies", systemImage: "film")
                    }
                }
                ForEach(store.libraries) { library in
                    MediaNavigationLink(destination: .collection(title: library.title, parent: library, types: nil)) {
                        Label(library.title, systemImage: symbol(library.CollectionType)).padding(.vertical, 12)
                    }
                }
                if !store.preview && store.libraries.isEmpty {
                    if store.loading { ProgressView("Loading collections") }
                    else if let problem = store.libraryProblem {
                        Text(problem).foregroundStyle(.secondary)
                        Button("Refresh library") { Task { await store.refresh() } }
                    } else { Text("No libraries are available to this account.").foregroundStyle(.secondary) }
                }
            }
            Section("More") {
                NavigationLink(destination: DownloadsView(offline: store.offline).bluefinBackNavigation()) { Label("Downloads", systemImage: "arrow.down.circle") }
                NavigationLink(destination: ConnectedAppsView().bluefinBackNavigation()) { Label("Connected apps", systemImage: "arrow.up.forward.app") }
                if !store.preview {
                    MediaNavigationLink(destination: .trailerCatalog) { Label("Trailers", systemImage: "film.stack") }
                    MediaNavigationLink(destination: .channels) { Label("Live TV", systemImage: "dot.radiowaves.left.and.right") }
                }
            }
        }.navigationTitle("Library")
    }
    private func symbol(_ collection: String?) -> String {
        switch collection { case "movies": return "film"; case "tvshows": return "tv"; case "music": return "music.note"; case "livetv": return "dot.radiowaves.left.and.right"; case "books": return "book"; default: return "square.stack" }
    }
}

struct ItemGrid: View {
    let title: String
    let parent: MediaItem?
    var query: String = ""
    var includeItemTypes: String? = nil
    @EnvironmentObject private var store: LibraryStore
    @State private var items: [MediaItem] = []
    @State private var offset = 0
    @State private var requestID: UUID?
    @State private var total = 0
    @State private var loading = false
    @State private var problem: String?
    private var columns: [GridItem] {
        #if os(tvOS)
        [GridItem(.adaptive(minimum: 230, maximum: 300), spacing: 30, alignment: .top)]
        #else
        [GridItem(.adaptive(minimum: 140, maximum: 220), spacing: 18, alignment: .top)]
        #endif
    }
    var body: some View {
        ScrollView {
            #if os(tvOS)
            Text(title).font(.largeTitle.bold()).frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 24).accessibilityAddTraits(.isHeader)
            #endif
            LazyVGrid(columns: columns, alignment: .leading, spacing: 26) {
                ForEach(items) { item in MediaNavigationLink(destination: .detail(item)) { Poster(item: item) }.buttonStyle(.plain) }
            }.padding(24)
            if loading { ProgressView().padding(30) }
            if let problem { EmptyLibrary(title: "Couldn't load this library", message: problem, symbol: "wifi.exclamationmark"); Button("Try again") { Task { await load(reset: items.isEmpty) } } }
            if !loading && problem == nil && items.isEmpty { EmptyLibrary(title: "Nothing here yet", message: query.isEmpty ? "This collection has no items available to your account." : "Try another title, artist, or show.", symbol: "magnifyingglass") }
            if offset < total && !loading && problem == nil { Button("Load more") { Task { await load(reset: false) } }.padding(30) }
        }
        #if !os(tvOS)
        .navigationTitle(title)
        #endif
        .task(id: store.accountContext) { await load(reset: true) }
            .onChange(of: query) { _ in Task { await load(reset: true) } }
    }
    private func load(reset: Bool) async {
        guard reset || !loading else { return }
        let id = UUID()
        requestID = id
        loading = true; problem = nil
        if reset { items = []; offset = 0; total = 0 }
        defer { if requestID == id { loading = false } }
        do {
            let page = try await store.items(parent: parent, query: query, start: offset, includeItemTypes: includeItemTypes)
            try Task.checkCancellation()
            guard requestID == id else { return }
            var existing = Set(items.map(\.Id))
            items.append(contentsOf: page.Items.filter { existing.insert($0.Id).inserted })
            offset += page.Items.count
            total = page.Items.isEmpty ? offset : page.TotalRecordCount ?? offset
        } catch is CancellationError {} catch {
            if requestID == id { problem = error.localizedDescription }
        }
    }
}

struct SearchView: View {
    @State private var query = ""
    @State private var committed = ""
    var body: some View {
        Group {
            if committed.isEmpty { EmptyLibrary(title: "Find your next favorite", message: "Search the movies, shows, and music available to your account.", symbol: "magnifyingglass") }
            else { UnifiedSearchResults(query: committed) }
        }
        .navigationTitle("Search").searchable(text: $query, prompt: Text("Movies, shows, music and live TV"))
        .task(id: query) {
            do { try await Task.sleep(nanoseconds: 350_000_000); try Task.checkCancellation(); committed = query.trimmingCharacters(in: .whitespacesAndNewlines) } catch {}
        }
    }
}

struct DetailView: View {
    let item: MediaItem
    var onBack: (() -> Void)? = nil
    @EnvironmentObject private var store: LibraryStore
    #if os(iOS) || os(visionOS)
    @State private var videoOptionsVisible = false
    @State private var videoSelection = VideoOptionSelection()
    @State private var startAtTicks: Int64?
    #endif
    #if os(tvOS)
    @Namespace private var detailFocus
    #endif
    @State private var detail: MediaItem?
    @State private var problem: String?
    @State private var favorite = false
    @State private var favoriteBusy = false
    private var current: MediaItem { detail ?? item }
    private var resumeTicks: Int64 { current.resumeTicks > 0 ? current.resumeTicks : item.resumeTicks }
    private var overview: String? {
        [current.Overview, item.Overview].compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                #if os(tvOS)
                HStack(alignment: .center, spacing: 52) {
                    Artwork(item: current, backdrop: true).aspectRatio(16 / 9, contentMode: .fit)
                        .frame(maxWidth: 640).clipShape(RoundedRectangle(cornerRadius: 20))
                    VStack(alignment: .leading, spacing: 18) {
                        Text(current.title).font(.largeTitle.bold()).lineLimit(3).accessibilityAddTraits(.isHeader)
                        if !current.subtitle.isEmpty { Text(current.subtitle).foregroundStyle(.secondary) }
                        if let overview { Text(overview).lineLimit(3).foregroundStyle(.secondary) }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
                HStack(spacing: 22) {
                    if !current.isContainer { playActions }
                    if let overview {
                        MediaNavigationLink(destination: .description(title: current.title, text: overview)) {
                            TVDetailAction(title: "Full description", symbol: "text.alignleft")
                        }.buttonStyle(.plain)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading).focusSection()
                if current.Type == "Series" || current.Type == "Season" {
                    TVEpisodeShelf(parent: current)
                } else if current.isContainer {
                    MediaNavigationLink(destination: .collection(title: current.title, parent: current, types: nil)) {
                        TVDetailAction(title: "Browse collection", symbol: "square.grid.2x2")
                    }.buttonStyle(.plain)
                }
                #else
                Artwork(item: current, backdrop: true).aspectRatio(16 / 9, contentMode: .fit).clipShape(RoundedRectangle(cornerRadius: 20))
                Text(current.title).font(.largeTitle.bold()).accessibilityAddTraits(.isHeader)
                if !current.subtitle.isEmpty { Text(current.subtitle).foregroundStyle(.secondary) }
                if current.isContainer {
                    MediaNavigationLink(destination: .collection(title: current.title, parent: current, types: nil)) { Label("Browse \(current.Type == "Series" ? "episodes" : "collection")", systemImage: "square.grid.2x2") }.buttonStyle(.borderedProminent)
                } else { playActions }
                if let overview {
                    Text(overview).font(.body).lineSpacing(5).fixedSize(horizontal: false, vertical: true)
                    LocalAssistanceSummary(item: current, description: overview, context: store.accountContext)
                }
                #endif
                NativeIntegrationActions(item: current)
                if !store.preview && ["Movie", "Series"].contains(current.Type ?? "") {
                    #if os(tvOS)
                    MediaNavigationLink(destination: .trailers(current.Id)) {
                        TVDetailAction(title: "Trailers", symbol: "play.rectangle")
                    }.buttonStyle(.plain)
                    #else
                    MediaNavigationLink(destination: .trailers(current.Id)) { Label("Trailers", systemImage: "play.rectangle") }.buttonStyle(.bordered)
                    #endif
                }
                if let problem { Label(problem, systemImage: "exclamationmark.circle").font(.callout).foregroundStyle(.secondary) }
            }.padding(24).frame(maxWidth: detailWidth).frame(maxWidth: .infinity)
        }
        .task {
            favorite = item.UserData?.IsFavorite == true
            do { detail = try await store.detail(item); favorite = current.UserData?.IsFavorite == true }
            catch { problem = error.localizedDescription }
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    Task {
                        favoriteBusy = true; defer { favoriteBusy = false }
                        do { try await store.setFavorite(current, value: !favorite); favorite.toggle() }
                        catch { problem = error.localizedDescription }
                    }
                } label: { Image(systemName: favorite ? "star.fill" : "star") }
                .accessibilityLabel(favorite ? "Remove favorite" : "Add favorite").disabled(favoriteBusy)
            }
        }
        .navigationTitle("")
        #if os(tvOS)
        .focusScope(detailFocus)
        #endif
        .bluefinBackNavigation(onBack)
    }

    private var detailWidth: CGFloat {
        #if os(tvOS)
        1600
        #else
        1100
        #endif
    }

    @ViewBuilder private var playActions: some View {
        HStack(spacing: 20) {
            #if os(tvOS)
            Button { playDetail() } label: {
                TVDetailAction(title: resumeTicks > 0 ? "Resume" : "Play", symbol: "play.fill", prominent: true)
            }.buttonStyle(.plain).disabled(store.startingPlayback).prefersDefaultFocus(in: detailFocus)
            if resumeTicks > 0 {
                Button { playDetail(fromBeginning: true) } label: {
                    TVDetailAction(title: "Start over", symbol: "arrow.counterclockwise")
                }.buttonStyle(.plain).disabled(store.startingPlayback)
            }
            #else
            Button { playDetail() } label: {
                Label(resumeTicks > 0 ? "Resume" : "Play", systemImage: "play.fill")
            }.buttonStyle(.borderedProminent).disabled(store.startingPlayback)
            if resumeTicks > 0 {
                Button("Start over") { playDetail(fromBeginning: true) }.disabled(store.startingPlayback)
            }
            #endif
            if store.startingPlayback { ProgressView() }
        }
        #if os(iOS) || os(visionOS)
        if current.Type != "Audio" && !store.preview {
            Button("Video options") { videoOptionsVisible = true }.buttonStyle(.bordered)
                .sheet(isPresented: $videoOptionsVisible) {
                    BluefinNavigation {
                        VideoOptionsView(chapters: current.Chapters ?? [], source: current.MediaSources?.first,
                            selection: videoSelection, isRenegotiating: false, supportedRates: [], presentationRate: 1,
                            nextEpisodeTitle: nil, onSeek: { seconds in
                                startAtTicks = Int64(exactly: (seconds * 10_000_000).rounded())
                                videoOptionsVisible = false
                            }, onRenegotiate: { videoSelection = $0 }, onPresentationRate: { _ in }, onNextEpisode: {})
                            .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { videoOptionsVisible = false } } }
                    }
                }
        }
        #endif
    }

    private func playDetail(fromBeginning: Bool = false) {
        Task {
            #if os(iOS) || os(visionOS)
            let ticks = fromBeginning ? 0 : startAtTicks ?? resumeTicks
            startAtTicks = nil
            await store.play(current, fromBeginning: fromBeginning, startAtTicks: ticks, videoOptions: videoSelection)
            #else
            await store.play(current, fromBeginning: fromBeginning, startAtTicks: fromBeginning ? 0 : resumeTicks)
            #endif
        }
    }
}

struct SettingsView: View {
    @AppStorage("bluefin.appearance") private var appearance = 0
    #if os(tvOS)
    @AppStorage(TopShelfSnapshot.preference) private var showTopShelf = true
    #endif
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        Form {
            Section("Your library") {
                LabeledContentCompat("Account", value: store.preview ? "Preview" : store.session?.username ?? "")
                if let session = store.session { Text(session.server).font(.footnote).foregroundStyle(.secondary) }
                if !store.preview { NavigationLink(destination: MeshConnectionsView()) { Label("Connections and invitations", systemImage: "qrcode") } }
                Button(store.preview ? "Leave preview" : "Sign out", role: .destructive) { Task { Artwork.clearCache(); await store.signOut() } }
            }
            Section("Appearance") {
                Picker("Appearance", selection: $appearance) {
                    Text("System").tag(0)
                    Text("Light").tag(1)
                    Text("Dark").tag(2)
                }
            }
            Section("Playback") {
                NavigationLink(destination: AudioSettingsView(preferences: store.audio)) { Label("Audio and equalizer", systemImage: "slider.horizontal.3") }
                if !store.preview { NavigationLink(destination: IrohSettingsView()) { Label("Direct Iroh transport", systemImage: "network") } }
                Label("Apple native player", systemImage: "play.rectangle")
                Text("AirPlay, audio and subtitle menus, accessibility controls, and supported Picture in Picture use the system player. Available formats depend on your device and server.").font(.footnote).foregroundStyle(.secondary)
            }
            Section("Home integrations") {
                #if os(tvOS)
                if TopShelfSnapshot.directory != nil {
                    Toggle("Show recently added titles on the Apple TV Home Screen", isOn: $showTopShelf)
                }
                #endif
                if store.sports.available { NavigationLink(destination: SportsSettingsView(sports: store.sports)) { Label("Sports preferences", systemImage: "sportscourt") } }
                NavigationLink(destination: HomeAssistantView()) { Label("Home Assistant", systemImage: "house") }
            }
            Section("Bluefin") {
                NavigationLink(destination: AuthorizedUseView()) { Label("Libraries and rights", systemImage: "checkmark.shield") }
                LabeledContentCompat("Version", value: "0.1.0 · Development")
                Text("Development build. No analytics or advertising. Bluefin is an independent client for Bluefin, Ferrofin and Jellyfin servers.").font(.footnote).foregroundStyle(.secondary)
                NavigationLink("Software licenses", destination: SoftwareLicensesView())
                Link("Project source", destination: URL(string: "https://github.com/glopyglerky/bluefin")!)
            }
        }.navigationTitle("Settings")
    }
}

struct LabeledContentCompat: View {
    let title: String
    let value: String
    init(_ title: String, value: String) { self.title = title; self.value = value }
    var body: some View { HStack { Text(title); Spacer(); Text(value).foregroundStyle(.secondary) } }
}

struct EmptyLibrary: View {
    let title: String
    let message: String
    let symbol: String
    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: symbol).font(.system(size: 44, weight: .light)).foregroundStyle(.secondary).accessibilityHidden(true)
            Text(title).font(.title2.bold())
            Text(message).foregroundStyle(.secondary).multilineTextAlignment(.center).frame(maxWidth: 420)
        }.padding(40).frame(maxWidth: .infinity, minHeight: 320)
    }
}

extension View {
    @ViewBuilder func bluefinTextFields() -> some View {
        #if os(iOS)
        self.textFieldStyle(.roundedBorder)
        #else
        self
        #endif
    }
    @ViewBuilder func bluefinAddressInput() -> some View {
        #if os(iOS)
        self.textInputAutocapitalization(.never).disableAutocorrection(true).keyboardType(.URL)
        #else
        self
        #endif
    }
    @ViewBuilder func bluefinRefresh(_ action: @escaping () async -> Void) -> some View {
        #if os(iOS)
        self.refreshable { await action() }
        #else
        self
        #endif
    }
}
