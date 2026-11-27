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
        BluefinStyle.configureChrome()
    }
    var body: some Scene {
        WindowGroup {
            RootView().environmentObject(store).foregroundStyle(BluefinStyle.text).tint(BluefinStyle.accent).preferredColorScheme(appearance == 1 ? .light : appearance == 2 ? .dark : nil)
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
        #if !os(tvOS)
        .sheet(isPresented: $store.deviceMusicVisible) { DeviceMusicView().environmentObject(store) }
        .safeAreaInset(edge: .bottom) { DeviceMusicAccessory(model: store.deviceMusic).environmentObject(store) }
        #endif
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
        .task(id: store.initialHomeLoaded ? store.session.map(OfflineLibrary.identity) : nil) {
            guard store.initialHomeLoaded else { return }
            await store.sports.connect(store.session)
        }
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
                Image(systemName: "key.viewfinder").font(.system(size: 50, weight: .light)).foregroundColor(BluefinStyle.accent).accessibilityHidden(true)
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

enum BluefinCategory: Int, CaseIterable, Identifiable {
    case home = 0, sports = 6, shows = 5, movies = 1, liveTV = 4, music = 2
    var id: Int { rawValue }
    var title: String {
        switch self {
        case .home: "Home"
        case .sports: "Sports"
        case .shows: "TV Shows"
        case .movies: "Movies"
        case .liveTV: "Live TV"
        case .music: "Music"
        }
    }
    var symbol: String {
        switch self {
        case .home: "house"
        case .sports: "sportscourt"
        case .shows: "tv"
        case .movies: "film"
        case .liveTV: "dot.radiowaves.left.and.right"
        case .music: "music.note"
        }
    }
    var videoItemType: String? {
        switch self { case .shows: "Series"; case .movies: "Movie"; default: nil }
    }
    var searchScope: UnifiedSearchScope {
        switch self {
        case .shows: .tvShows
        case .movies: .movies
        case .music: .music
        case .liveTV: .liveTv
        case .home, .sports: .all
        }
    }
}

private enum BluefinUtility: String, Identifiable {
    case search, collections, account
    var id: String { rawValue }
}

struct LibraryTabs: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var selectedTab = BluefinCategory.home.rawValue
    @State private var visitedTabs: Set<Int> = [BluefinCategory.home.rawValue]
    @State private var homeVisit = UUID()
    @State private var utility: BluefinUtility?
    @State private var searchScope = UnifiedSearchScope.all
    #if os(tvOS)
    @State private var topShelfItem: MediaItem?
    @State private var topShelfRequest: URL?
    @State private var previousTab = BluefinCategory.home.rawValue
    #endif

    private var tabSelection: Binding<Int> {
        Binding(get: { selectedTab }, set: select)
    }

    private func select(_ tab: Int) {
        guard BluefinCategory(rawValue: tab) != nil else { return }
        visitedTabs.insert(tab)
        if tab == BluefinCategory.home.rawValue { showHome() }
        else { selectedTab = tab }
    }

    private func openSearch(category: BluefinCategory? = nil) {
        searchScope = category?.searchScope ?? .all
        utility = .search
    }

    private func showHome() {
        utility = nil
        #if os(tvOS)
        topShelfRequest = nil
        topShelfItem = nil
        #endif
        homeVisit = UUID()
        selectedTab = BluefinCategory.home.rawValue
    }

    var body: some View {
        navigation
            .environment(\.bluefinShowHome, showHome)
            .sheet(item: $utility) { destination in
                BluefinNavigation {
                    #if os(tvOS)
                    utilityContent(destination).bluefinBackNavigation { utility = nil }
                    #else
                    utilityContent(destination)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("Done") { utility = nil }
                            }
                        }
                    #endif
                }
                .environment(\.bluefinShowHome, showHome)
                .tint(BluefinStyle.accent)
                .foregroundStyle(BluefinStyle.text)
            }
            #if os(iOS) && !targetEnvironment(macCatalyst)
            .onOpenURL { url in
                guard let route = WidgetRoute.admitted(url, authenticated: store.session != nil, preview: store.preview) else { return }
                switch route {
                case .library: utility = .collections
                case .search: openSearch(category: .home)
                case .music, .liveTV: select(route.tab)
                }
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
                    selectedTab = BluefinCategory.home.rawValue
                    topShelfItem = item
                } catch is CancellationError {} catch {
                    store.error = "This title is no longer available."
                }
            }
            #endif
            .onChange(of: store.accountContext) { _ in
                visitedTabs = [BluefinCategory.home.rawValue]
                showHome()
            }
            .task {
                NativeIntegrations.attach(to: store)
                NativeIntegrations.shared.openLibrary = { utility = .collections }
                if !store.loading { await store.refresh() }
            }
    }

    @ViewBuilder private var navigation: some View {
        #if os(tvOS)
        HStack(alignment: .top, spacing: 0) {
            televisionSidebar
            ZStack {
                ForEach(BluefinCategory.allCases) { category in
                    if visitedTabs.contains(category.rawValue) {
                        categoryNavigation(category)
                            .opacity(selectedTab == category.rawValue ? 1 : 0)
                            .allowsHitTesting(selectedTab == category.rawValue)
                            .accessibilityHidden(selectedTab != category.rawValue)
                            .disabled(selectedTab != category.rawValue)
                            .zIndex(selectedTab == category.rawValue ? 1 : 0)
                    }
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(BluefinStyle.background)
        .safeAreaInset(edge: .bottom, spacing: 0) { miniPlayer }
        #else
        GeometryReader { geometry in
            let sidebar = geometry.size.width >= 760
            HStack(spacing: 0) {
                if sidebar { categorySidebar }
                ZStack {
                    ForEach(BluefinCategory.allCases) { category in
                        if visitedTabs.contains(category.rawValue) {
                            categoryNavigation(category)
                                .opacity(selectedTab == category.rawValue ? 1 : 0)
                                .allowsHitTesting(selectedTab == category.rawValue)
                                .accessibilityHidden(selectedTab != category.rawValue)
                                .disabled(selectedTab != category.rawValue)
                                .zIndex(selectedTab == category.rawValue ? 1 : 0)
                        }
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .background(BluefinStyle.background)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(spacing: 0) {
                    miniPlayer
                    if !sidebar { compactCategories }
                }
            }
        }
        #endif
    }

    private func categoryNavigation(_ category: BluefinCategory) -> some View {
        BluefinNavigation {
            #if os(tvOS)
            if category == .home { categoryContent(category) }
            else {
                categoryContent(category)
                    .safeAreaInset(edge: .top, spacing: 0) {
                        HStack(spacing: 18) { Spacer(); utilityButtons }
                            .buttonStyle(.bordered).padding(.horizontal, 32).padding(.vertical, 10)
                            .background(BluefinStyle.background).focusSection()
                    }
            }
            #else
            categoryContent(category)
                .toolbar {
                    ToolbarItemGroup(placement: .primaryAction) { utilityButtons }
                }
            #endif
        }
        .id(category == .home ? homeVisit.uuidString : category.title)
        .environment(\.bluefinCategoryIsActive, selectedTab == category.rawValue)
    }

    @ViewBuilder private func categoryContent(_ category: BluefinCategory) -> some View {
        switch category {
        case .home:
            #if os(tvOS)
            TVHomeView(shelfItem: $topShelfItem, sports: store.sports,
                       openSearch: { openSearch(category: .home) }, openCollections: { utility = .collections },
                       openAccount: { utility = .account })
            #else
            HomeView(sports: store.sports)
            #endif
        case .sports:
            SportsDashboard(sports: store.sports)
        case .shows:
            #if os(tvOS)
            TVVideoLibrary(kind: .shows)
            #else
            VideoCategoryView(category: .shows)
            #endif
        case .movies:
            #if os(tvOS)
            TVVideoLibrary(kind: .movies)
            #else
            VideoCategoryView(category: .movies)
            #endif
        case .liveTV:
            #if os(tvOS)
            LiveGuideView()
            #else
            LiveTVView()
            #endif
        case .music:
            MusicHubView()
        }
    }

    @ViewBuilder private func utilityContent(_ destination: BluefinUtility) -> some View {
        switch destination {
        case .search: SearchView(initialScope: searchScope)
        case .collections: LibrariesView()
        case .account: SettingsView()
        }
    }

    private var utilityButtons: some View {
        Group {
            #if os(tvOS)
            Button { openSearch() } label: { Label("Search", systemImage: "magnifyingglass") }
                .accessibilityIdentifier("openSearch")
            Button { utility = .collections } label: { Label("Collections", systemImage: "square.stack") }
                .accessibilityIdentifier("openCollections")
            Button { utility = .account } label: { Label("Account", systemImage: "person.crop.circle") }
                .accessibilityIdentifier("openAccount")
            #else
            Button { openSearch() } label: { Image(systemName: "magnifyingglass") }
                .accessibilityLabel("Search all media").accessibilityIdentifier("openSearch")
            Button { utility = .collections } label: { Image(systemName: "square.stack") }
                .accessibilityLabel("Collections and libraries").accessibilityIdentifier("openCollections")
            Button { utility = .account } label: { Image(systemName: "person.crop.circle") }
                .accessibilityLabel("Account and settings").accessibilityIdentifier("openAccount")
            #endif
        }
    }

    @ViewBuilder private var miniPlayer: some View {
        if let playback = store.playback, playback.item.Type == "Audio" {
            MusicMiniPlayer(playback: playback)
        }
    }

    private var compactCategories: some View {
        HStack(spacing: 0) {
            ForEach(BluefinCategory.allCases) { category in
                Button { select(category.rawValue) } label: {
                    VStack(spacing: 4) {
                        Image(systemName: category.symbol)
                            .font(.system(size: 20, weight: selectedTab == category.rawValue ? .semibold : .regular))
                        Text(category.title).font(.system(size: 10, weight: .semibold))
                            .lineLimit(2).multilineTextAlignment(.center)
                    }
                    .frame(maxWidth: .infinity, minHeight: 56)
                    .foregroundStyle(selectedTab == category.rawValue ? BluefinStyle.accent : BluefinStyle.text)
                    .background {
                        if selectedTab == category.rawValue {
                            Capsule().fill(BluefinStyle.text.opacity(0.10))
                        }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selectedTab == category.rawValue ? .isSelected : [])
                .accessibilityIdentifier("category-\(category.rawValue)")
            }
        }
        .padding(5)
        .modifier(BluefinGlassSurface(cornerRadius: 38))
        .padding(.horizontal, 10).padding(.top, 5).padding(.bottom, 7)
        .accessibilityElement(children: .contain)
    }

    #if os(tvOS)
    private var televisionSidebar: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(spacing: 6) {
                BluefinMark().frame(width: 40, height: 40)
                Text("Bluefin").font(.system(size: 26, weight: .bold, design: .rounded))
            }.padding(.bottom, 24)
            ForEach(BluefinCategory.allCases) { category in
                Button { select(category.rawValue) } label: {
                    Label(category.title, systemImage: category.symbol)
                        .font(.system(size: 20, weight: .semibold))
                        .frame(width: 178, height: 40, alignment: .leading)
                }
                .buttonStyle(.bordered)
                .tint(selectedTab == category.rawValue ? BluefinStyle.primaryFill : BluefinStyle.secondaryBackground)
                .foregroundStyle(selectedTab == category.rawValue ? BluefinStyle.onPrimary : BluefinStyle.text)
                .accessibilityAddTraits(selectedTab == category.rawValue ? .isSelected : [])
                .accessibilityIdentifier("category-\(category.rawValue)")
            }
            Spacer()
        }
        .padding(.horizontal, 20).padding(.top, 38).frame(width: 260)
        .frame(maxHeight: .infinity).focusSection()
        .background(BluefinStyle.secondaryBackground.opacity(0.4))
    }
    #endif

    private var categorySidebar: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                BluefinMark().frame(width: 38, height: 38)
                Text("Bluefin").font(.system(size: 23, weight: .bold, design: .rounded))
            }.padding(.horizontal, 8).padding(.vertical, 18)
            ForEach(BluefinCategory.allCases) { category in
                Button { select(category.rawValue) } label: {
                    Label(category.title, systemImage: category.symbol)
                        .font(.body.weight(selectedTab == category.rawValue ? .semibold : .regular))
                        .frame(maxWidth: .infinity, minHeight: 46, alignment: .leading)
                        .padding(.horizontal, 12)
                        .foregroundStyle(selectedTab == category.rawValue ? BluefinStyle.accent : BluefinStyle.text)
                        .background {
                            if selectedTab == category.rawValue {
                                RoundedRectangle(cornerRadius: 10).fill(BluefinStyle.brand.opacity(0.15))
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selectedTab == category.rawValue ? .isSelected : [])
                .accessibilityIdentifier("category-\(category.rawValue)")
            }
            Spacer(minLength: 12)
        }
        .padding(12).frame(width: 210)
        .background(BluefinStyle.secondaryBackground)
        .overlay(alignment: .trailing) { Rectangle().fill(BluefinStyle.decorativeLine).frame(width: 1) }
    }
}

private struct VideoCategoryView: View {
    @EnvironmentObject private var store: LibraryStore
    let category: BluefinCategory
    private var collectionType: String { category == .movies ? "movies" : "tvshows" }
    var body: some View {
        let collections = store.libraries.filter { $0.CollectionType == collectionType }
        ItemGrid(title: category.title, parent: nil, includeItemTypes: category.videoItemType)
            .safeAreaInset(edge: .top) {
                if collections.count > 1 {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 12) {
                            ForEach(collections) { collection in
                                MediaNavigationLink(destination: .collection(title: collection.title, parent: collection, types: category.videoItemType)) {
                                    Text(collection.title)
                                }.buttonStyle(.bordered)
                            }
                        }.padding(.horizontal, 24).padding(.vertical, 8)
                    }.background(BluefinStyle.background)
                }
            }
            .background(BluefinBackground())
    }
}

struct HomeView: View {
    @EnvironmentObject private var store: LibraryStore
    @ObservedObject var sports: SportsStore
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 30) {
                if store.preview { Label("Preview library", systemImage: "sparkles").font(.caption.bold()).foregroundStyle(.secondary).padding(.horizontal, 24) }
                if let featured = store.home.featured { FeaturedItem(item: featured) }
                if !store.home.resume.isEmpty { MediaShelf(title: "Continue watching", items: store.home.resume, landscape: true) }
                if !store.home.nextUp.isEmpty { MediaShelf(title: "Up next", items: store.home.nextUp, landscape: true) }
                ForEach(store.home.recommendations) { shelf in
                    if !shelf.items.isEmpty { MediaShelf(title: shelf.title, items: shelf.items) }
                }
                if !store.home.latest.isEmpty { MediaShelf(title: store.homeFromCatalog ? "Browse your library" : "Recently added", items: store.home.latest) }
                if let issue = store.libraryProblem {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(issue).font(.callout).foregroundStyle(.secondary)
                        Button("Refresh library") { Task { await store.refresh() } }.disabled(store.loading)
                    }.padding(.horizontal, 24)
                }
                if store.home.latest.isEmpty && store.home.resume.isEmpty && store.home.nextUp.isEmpty {
                    if store.loading || (!store.initialHomeLoaded && !store.preview) { ProgressView("Loading your library").frame(maxWidth: .infinity).padding(40) }
                    else if store.libraryProblem == nil {
                        EmptyLibrary(title: "Browse your library", message: "Open Collections to see your libraries.", symbol: "square.stack")
                    }
                }
                if !store.preview {
                    if sports.feed == nil {
                        NavigationLink(destination: SportsDashboard(sports: sports).bluefinBackNavigation()) {
                            Label("Sports", systemImage: "sportscourt")
                        }.padding(.horizontal, 24)
                    }
                    SportsHomeSection(sports: sports)
                }
            }.padding(.vertical, 16)
        }
        .background(BluefinBackground())
        .navigationTitle("Home")
        #if os(iOS)
        .navigationBarTitleDisplayMode(.large)
        #endif
        .bluefinRefresh { await store.refresh(); await sports.connect(store.session) }
    }
}

struct FeaturedItem: View {
    let item: MediaItem
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.dynamicTypeSize) private var dynamicType
    var body: some View {
        ZStack(alignment: .bottomLeading) {
            Artwork(item: item, backdrop: true,
                    thumbnailWidth: sizeClass == .compact ? 1200 : nil)
            LinearGradient(colors: [.clear, .black.opacity(0.88)],
                           startPoint: .top, endPoint: .bottom)
            VStack(alignment: .leading, spacing: 10) {
                Text("FEATURED").font(.caption2.weight(.semibold)).tracking(1.5)
                Text(item.title)
                    .font(sizeClass == .compact ? .title.bold() : .largeTitle.bold())
                    .lineLimit(dynamicType.isAccessibilitySize ? nil : 2)
                    .fixedSize(horizontal: false, vertical: true)
                if !item.subtitle.isEmpty {
                    Text(item.subtitle).font(.subheadline).foregroundStyle(.white.opacity(0.85))
                        .lineLimit(dynamicType.isAccessibilitySize ? nil : 2)
                }
                HStack(spacing: 12) {
                    if !item.isContainer {
                        Button { Task { await store.play(item) } } label: {
                            Label(item.resumeTicks > 0 ? "Resume" : "Play", systemImage: "play.fill")
                                .font(.body.weight(.semibold)).padding(.horizontal, 12).padding(.vertical, 4)
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary)
                        .disabled(store.startingPlayback)
                        .accessibilityIdentifier("featuredPlay")
                    }
                    MediaNavigationLink(destination: .detail(item)) {
                        Label("Details", systemImage: "info.circle")
                    }
                    .buttonStyle(.bordered).tint(.white).foregroundStyle(.white)
                    .accessibilityIdentifier("featuredDetails")
                    if store.startingPlayback { ProgressView().tint(.white) }
                }
            }
            .foregroundStyle(.white).padding(22)
        }
        .frame(height: dynamicType.isAccessibilitySize ? 430 : sizeClass == .compact ? 270 : 370)
        .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay { RoundedRectangle(cornerRadius: 20).strokeBorder(BluefinStyle.decorativeLine.opacity(0.5)) }
        .padding(.horizontal, 24)
    }
}

struct MediaShelf: View {
    let title: String
    let items: [MediaItem]
    var landscape = false
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(title).font(.title2.bold()).padding(.horizontal, 24).accessibilityAddTraits(.isHeader)
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: 16) {
                    ForEach(items) { item in
                        MediaNavigationLink(destination: .detail(item)) {
                            if landscape { LandscapeMediaTile(item: item).frame(width: landscapeWidth) }
                            else { Poster(item: item).frame(width: posterWidth) }
                        }
                            .buttonStyle(.plain)
                    }
                }.padding(.horizontal, 24).padding(.vertical, 8)
            }
        }
    }
    private var landscapeWidth: CGFloat {
        #if os(tvOS)
        390
        #else
        280
        #endif
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
    @Environment(\.dynamicTypeSize) private var dynamicType
    #if os(tvOS)
    @Environment(\.isFocused) private var focused
    #endif
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Artwork(item: item)
                .aspectRatio(["Audio", "MusicAlbum", "MusicArtist", "Playlist"].contains(item.Type ?? "") ? 1 : 2 / 3, contentMode: .fit)
                .overlay(alignment: .bottom) {
                    if item.progress > 0 {
                        ProgressView(value: item.progress).tint(BluefinStyle.accent).padding(10).accessibilityHidden(true)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                #if os(tvOS)
                .overlay {
                    if focused { RoundedRectangle(cornerRadius: 14).strokeBorder(BluefinStyle.accent, lineWidth: 3) }
                }
                #endif
            Text(item.title).font(.subheadline.weight(.semibold))
                .lineLimit(dynamicType.isAccessibilitySize ? 3 : 2)
            if !item.subtitle.isEmpty {
                Text(item.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(item.title + (item.subtitle.isEmpty ? "" : ", " + item.subtitle))
        .accessibilityValue(item.progress > 0 ? "\(Int(item.progress * 100)) percent watched" : "")
    }
}

struct LandscapeMediaTile: View {
    let item: MediaItem
    #if os(tvOS)
    @Environment(\.isFocused) private var focused
    #endif
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Artwork(item: item, backdrop: true, thumbnailWidth: 640)
                .aspectRatio(16 / 9, contentMode: .fit)
                .overlay(alignment: .bottom) {
                    if item.progress > 0 {
                        ProgressView(value: item.progress).tint(.white).padding(12)
                            .background(LinearGradient(colors: [.clear, .black.opacity(0.65)], startPoint: .top, endPoint: .bottom))
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                #if os(tvOS)
                .overlay {
                    if focused { RoundedRectangle(cornerRadius: 16).strokeBorder(BluefinStyle.accent, lineWidth: 3) }
                }
                #endif
            Text(item.title).font(.headline).lineLimit(1)
            if !item.subtitle.isEmpty { Text(item.subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(1) }
        }.accessibilityElement(children: .combine)
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
    var initialScope = UnifiedSearchScope.all
    @EnvironmentObject private var store: LibraryStore
    @State private var query = ""
    @State private var committed = ""
    var body: some View {
        UnifiedSearchResults(query: committed, initialScope: initialScope)
        .navigationTitle("Search").searchable(text: $query, prompt: Text("Movies, shows, music and live TV"))
        .onSubmit(of: .search) { Task { await store.recordHomeSearch(query: query) } }
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
                    MediaNavigationLink(destination: .collection(title: current.title, parent: current, types: nil)) { Label("Browse \(current.Type == "Series" ? "episodes" : "collection")", systemImage: "square.grid.2x2") }.buttonStyle(.borderedProminent).tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary)
                } else { playActions }
                if let overview {
                    Text(overview).font(.body).lineSpacing(5).fixedSize(horizontal: false, vertical: true)
                    LocalAssistanceSummary(item: current, description: overview, context: store.accountContext)
                }
                #endif
                NativeIntegrationActions(item: current)
                if let offer = CatalogOfferReference(item: current) {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("Availability").font(.headline)
                        CatalogOfferPanel(reference: offer)
                    }
                }
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
            }.buttonStyle(.borderedProminent).tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary).disabled(store.startingPlayback)
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
            if !store.preview { HomeHistorySettings() }
            Section("Appearance") {
                Picker("Appearance", selection: $appearance) {
                    Text("System").tag(0)
                    Text("Light").tag(1)
                    Text("Dark").tag(2)
                }
            }
            #if !os(tvOS)
            Section("On this device") {
                Button { store.deviceMusicVisible = true } label: { Label("Music on this device", systemImage: "music.note.house") }
            }
            #endif
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
                if !store.preview { NavigationLink(destination: MusicAssistantView()) { Label("Music Assistant", systemImage: "music.note.list") } }
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
