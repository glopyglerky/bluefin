import SwiftUI
#if os(iOS)
import CoreSpotlight
#endif

@main struct BluefinApp: App {
    @StateObject private var store = LibraryStore()
    @AppStorage("bluefin.appearance") private var appearance = 0
    var body: some Scene {
        WindowGroup {
            RootView().environmentObject(store).tint(BluefinStyle.accent).preferredColorScheme(appearance == 1 ? .light : appearance == 2 ? .dark : nil)
        }
    }
}

struct RootView: View {
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        Group {
            if store.signedIn { LibraryTabs() }
            else { WelcomeView() }
        }
        .fullScreenCover(item: Binding(get: { store.playback?.item.Type == "Audio" ? nil : store.playback }, set: { if $0 == nil && store.playback?.item.Type != "Audio" { store.dismissPlayback() } }), onDismiss: { Task { await store.refresh() } }) { PlaybackScreen(playback: $0) }
        .sheet(isPresented: $store.musicExpanded) {
            if let playback = store.playback, playback.item.Type == "Audio" { MusicPlayer(playback: playback).id(playback.id) }
        }
        .task {
            NativeIntegrations.attach(to: store)
            #if os(iOS)
            if #available(iOS 16.0, *) { BluefinAppShortcuts.updateAppShortcutParameters() }
            #endif
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

struct WelcomeView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var server = ""
    @State private var username = ""
    @State private var password = ""
    @State private var allowHttp = false
    @State private var busy = false
    @State private var problem: String?
    @State private var serverName: String?
    @State private var quickConnect = false
    @FocusState private var focus: Field?
    private enum Field: Hashable { case server, username, password }
    var body: some View {
        NavigationView {
            ScrollView {
                VStack(alignment: .leading, spacing: 30) {
                    VStack(alignment: .leading, spacing: 16) {
                        Image(systemName: "water.waves").font(.system(size: 50, weight: .light)).foregroundColor(.cyan).accessibilityHidden(true)
                        Text("Bluefin").font(.system(.largeTitle, design: .rounded).bold())
                        Text("Your library.\nRight at home.").font(.largeTitle.bold()).fixedSize(horizontal: false, vertical: true)
                        Text("Movies, shows, music, and live TV from your own server. Made for your Apple devices.")
                            .font(.body).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                    VStack(alignment: .leading, spacing: 18) {
                        Text("Connect your server").font(.headline)
                        TextField("https://media.example.com", text: $server).bluefinAddressInput().textContentType(.URL).focused($focus, equals: .server).submitLabel(.next).onSubmit { focus = .username }
                            .accessibilityIdentifier("serverAddress")
                        TextField("Username", text: $username).bluefinAddressInput().textContentType(.username).focused($focus, equals: .username).submitLabel(.next).onSubmit { focus = .password }
                            .accessibilityIdentifier("username")
                        SecureField("Password", text: $password).textContentType(.password).focused($focus, equals: .password).submitLabel(.go).onSubmit { Task { await signIn() } }
                            .accessibilityIdentifier("password")
                        if server.lowercased().hasPrefix("http:") {
                            Toggle("Allow HTTP on my local network", isOn: $allowHttp)
                            Text("HTTP sends your sign-in and media without transport encryption. Use HTTPS when available.").font(.footnote).foregroundStyle(.secondary)
                        }
                        if let problem { Label(problem, systemImage: "exclamationmark.circle").font(.callout).foregroundColor(.red).accessibilityIdentifier("signInError") }
                        if let serverName { Label(serverName, systemImage: "checkmark.circle").foregroundColor(.green) }
                        Button { Task { await signIn() } } label: {
                            HStack { Spacer(); if busy { ProgressView() }; Text(busy ? "Connecting…" : "Connect").font(.body.weight(.semibold)); Spacer() }.padding(.vertical, 8)
                        }.buttonStyle(.borderedProminent).disabled(busy || server.isEmpty || username.isEmpty).accessibilityIdentifier("connectServer")
                        Button("Check server connection") { Task { await checkServer() } }.disabled(busy || server.isEmpty)
                        Button("Sign in with Quick Connect") { focus = nil; quickConnect = true }.disabled(busy || server.isEmpty)
                    }
                    .bluefinTextFields()
                    Divider()
                    Button { store.explore() } label: { Label("Explore the preview", systemImage: "play.rectangle.on.rectangle") }
                        .accessibilityIdentifier("explorePreview")
                    Text("Preview uses sample titles. Your server address and sign-in stay on this device, protected by Keychain.").font(.footnote).foregroundStyle(.secondary)
                }
                .padding(28).frame(maxWidth: 520).frame(maxWidth: .infinity)
            }
            .background {
                LinearGradient(colors: [.cyan.opacity(0.09), .clear, .clear], startPoint: .topLeading, endPoint: .bottomTrailing).ignoresSafeArea()
            }
            .navigationTitle("")
        }.navigationViewStyle(.stack)
        #if os(iOS)
        .toolbar { ToolbarItemGroup(placement: .keyboard) { Spacer(); Button("Done") { focus = nil } } }
        #endif
        .sheet(isPresented: $quickConnect) { QuickConnectView(server: server.trimmingCharacters(in: .whitespacesAndNewlines), allowHttp: allowHttp) }
    }
    private func signIn() async {
        guard !busy else { return }
        busy = true; problem = nil; focus = nil
        defer { busy = false; password = "" }
        do { try await store.signIn(server: server.trimmingCharacters(in: .whitespacesAndNewlines), username: username, password: password, allowHttp: allowHttp) }
        catch { problem = error.localizedDescription }
    }
    private func checkServer() async {
        busy = true; problem = nil; serverName = nil
        defer { busy = false }
        do {
            let info: PublicInfo = try await RustBridge.call("info", fields: ["server": server.trimmingCharacters(in: .whitespacesAndNewlines), "allowHttp": allowHttp, "deviceId": store.deviceId])
            serverName = info.ServerName ?? "Server is reachable"
        } catch { problem = error.localizedDescription }
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
                Text("Sign in on your own terms").font(.title2.bold()).multilineTextAlignment(.center)
                Text("Open Quick Connect in another signed-in client for your server and enter this code.").multilineTextAlignment(.center).foregroundStyle(.secondary)
                if let code { Text(code).font(.system(size: 42, weight: .semibold, design: .monospaced)).tracking(6).accessibilityLabel("Quick Connect code, \(code.map(String.init).joined(separator: ", "))").accessibilityIdentifier("quickConnectCode") }
                if let problem { Text(problem).foregroundColor(.red).multilineTextAlignment(.center); Button("Try again") { retry = UUID() } }
                else { ProgressView(code == nil ? "Getting a code" : "Waiting for your approval") }
                Button("Cancel") { dismiss() }
            }.padding(32).frame(maxWidth: 550).frame(maxWidth: .infinity, maxHeight: .infinity)
                .navigationTitle("Quick Connect")
        }.navigationViewStyle(.stack)
        .task(id: retry) { await connect() }
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
    var body: some View {
        TabView(selection: $selectedTab) {
            NavigationView { HomeView(sports: store.sports) }.navigationViewStyle(.stack).tabItem { Label("Home", systemImage: "house") }.tag(0)
            NavigationView { LibrariesView() }.tabItem { Label("Library", systemImage: "square.stack") }.tag(1)
            NavigationView { MusicLibraryView() }.navigationViewStyle(.stack).tabItem { Label("Music", systemImage: "music.note") }.tag(2)
            NavigationView { SearchView() }.navigationViewStyle(.stack).tabItem { Label("Search", systemImage: "magnifyingglass") }.tag(3)
            NavigationView { DownloadsView(offline: store.offline) }.navigationViewStyle(.stack).tabItem { Label("Downloads", systemImage: "arrow.down.circle") }.tag(4)
        }.safeAreaInset(edge: .bottom, spacing: 0) {
            if let playback = store.playback, playback.item.Type == "Audio" { MusicMiniPlayer(playback: playback) }
        }.task {
            NativeIntegrations.attach(to: store)
            NativeIntegrations.shared.openLibrary = { selectedTab = 1 }
            await store.refresh()
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
                    if !store.home.latest.isEmpty { MediaShelf(title: "Recently added", items: store.home.latest) }
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
                if store.loading { ProgressView("Loading your library").frame(maxWidth: .infinity).padding(40) }
                if !store.loading && store.home.latest.isEmpty && store.home.resume.isEmpty && sports.saved?.preferences.sports_only != true {
                    EmptyLibrary(title: "Your library is ready", message: "Open Library to browse your server's collections.", symbol: "square.stack")
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
                Artwork(item: item, backdrop: true)
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
                Button { Task { await store.play(item) } } label: { Label(item.progress > 0 ? "Resume" : "Play", systemImage: "play.fill").font(.body.weight(.semibold)).padding(.horizontal, 20).padding(.vertical, 6) }
                    .buttonStyle(.borderedProminent).disabled(store.startingPlayback)
                NavigationLink(destination: DetailView(item: item)) { Label("Details", systemImage: "info.circle") }.buttonStyle(.bordered)
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
                    ForEach(items) { item in NavigationLink(destination: DetailView(item: item)) { Poster(item: item).frame(width: posterWidth) }.bluefinPosterButton() }
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
    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Artwork(item: item).aspectRatio(item.Type == "MusicAlbum" ? 1 : 2 / 3, contentMode: .fit).clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay(alignment: .bottom) { if item.progress > 0 { ProgressView(value: item.progress).tint(.cyan).padding(8).accessibilityHidden(true) } }
            Text(item.title).font(.subheadline.weight(.semibold)).lineLimit(2).foregroundStyle(.primary)
            if !item.subtitle.isEmpty { Text(item.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
        }.accessibilityElement(children: .ignore).accessibilityLabel(item.title + (item.subtitle.isEmpty ? "" : ", " + item.subtitle))
    }
}

struct LibrariesView: View {
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        List {
            if !store.preview {
                NavigationLink(destination: LiveTVView()) { Label("Live TV", systemImage: "dot.radiowaves.left.and.right").padding(.vertical, 12) }
            }
            if store.preview {
                NavigationLink(destination: ItemGrid(title: "Sample movies", parent: nil)) { Label("Sample movies", systemImage: "film") }
            }
            ForEach(store.libraries) { library in
                NavigationLink(destination: ItemGrid(title: library.title, parent: library)) {
                    Label(library.title, systemImage: symbol(library.CollectionType)).padding(.vertical, 12)
                }
            }
            if !store.preview && store.libraries.isEmpty { Text("No libraries are available to this account.").foregroundStyle(.secondary) }
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
    @EnvironmentObject private var store: LibraryStore
    @State private var items: [MediaItem] = []
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
            LazyVGrid(columns: columns, alignment: .leading, spacing: 26) {
                ForEach(items) { item in NavigationLink(destination: DetailView(item: item)) { Poster(item: item) }.bluefinPosterButton() }
            }.padding(24)
            if loading { ProgressView().padding(30) }
            if let problem { EmptyLibrary(title: "Couldn't load this library", message: problem, symbol: "wifi.exclamationmark"); Button("Try again") { Task { await load(reset: items.isEmpty) } } }
            if !loading && problem == nil && items.isEmpty { EmptyLibrary(title: "Nothing here yet", message: query.isEmpty ? "This collection has no items available to your account." : "Try another title, artist, or show.", symbol: "magnifyingglass") }
            if items.count < total && !loading { Button("Load more") { Task { await load(reset: false) } }.padding(30) }
        }.navigationTitle(title).task(id: query) { await load(reset: true) }
    }
    private func load(reset: Bool) async {
        guard !loading else { return }
        loading = true; problem = nil
        defer { loading = false }
        do {
            let page = try await store.items(parent: parent, query: query, start: reset ? 0 : items.count)
            try Task.checkCancellation()
            if reset { items = page.Items } else {
                let existing = Set(items.map(\.Id)); items.append(contentsOf: page.Items.filter { !existing.contains($0.Id) })
            }
            total = page.TotalRecordCount ?? items.count
        } catch is CancellationError {} catch { problem = error.localizedDescription }
    }
}

struct SearchView: View {
    @State private var query = ""
    @State private var committed = ""
    var body: some View {
        Group {
            if committed.isEmpty { EmptyLibrary(title: "Find your next favorite", message: "Search the movies, shows, and music available to your account.", symbol: "magnifyingglass") }
            else { ItemGrid(title: "Search", parent: nil, query: committed).id(committed) }
        }
        .navigationTitle("Search").searchable(text: $query, prompt: Text("Your library"))
        .task(id: query) {
            do { try await Task.sleep(nanoseconds: 350_000_000); try Task.checkCancellation(); committed = query.trimmingCharacters(in: .whitespacesAndNewlines) } catch {}
        }
    }
}

struct DetailView: View {
    let item: MediaItem
    @EnvironmentObject private var store: LibraryStore
    @State private var detail: MediaItem?
    @State private var problem: String?
    private var current: MediaItem { detail ?? item }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                Artwork(item: current, backdrop: true).aspectRatio(16 / 9, contentMode: .fit).clipShape(RoundedRectangle(cornerRadius: 20))
                Text(current.title).font(.largeTitle.bold()).accessibilityAddTraits(.isHeader)
                if !current.subtitle.isEmpty { Text(current.subtitle).foregroundStyle(.secondary) }
                if current.isContainer {
                    NavigationLink(destination: ItemGrid(title: current.title, parent: current)) { Label("Browse \(current.Type == "Series" ? "episodes" : "collection")", systemImage: "square.grid.2x2") }.buttonStyle(.borderedProminent)
                } else {
                    HStack(spacing: 20) {
                        Button { Task { await store.play(current) } } label: { Label(current.progress > 0 ? "Resume" : "Play", systemImage: "play.fill").padding(6) }.buttonStyle(.borderedProminent).disabled(store.startingPlayback)
                        if current.progress > 0 { Button("Start over") { Task { await store.play(current, fromBeginning: true) } }.disabled(store.startingPlayback) }
                        if store.startingPlayback { ProgressView() }
                    }
                }
                if let overview = current.Overview, !overview.isEmpty { Text(overview).font(.body).lineSpacing(5).fixedSize(horizontal: false, vertical: true) }
                NativeIntegrationActions(item: current)
                if let problem { Label(problem, systemImage: "exclamationmark.circle").font(.callout).foregroundStyle(.secondary) }
            }.padding(24).frame(maxWidth: 1100).frame(maxWidth: .infinity)
        }
        .task { do { detail = try await store.detail(item) } catch { problem = error.localizedDescription } }
        .navigationTitle("")
    }
}

struct SettingsView: View {
    @AppStorage("bluefin.appearance") private var appearance = 0
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        Form {
            Section("Your library") {
                LabeledContentCompat("Account", value: store.preview ? "Preview" : store.session?.username ?? "")
                if let session = store.session { Text(session.server).font(.footnote).foregroundStyle(.secondary) }
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
                if store.sports.available { NavigationLink(destination: SportsSettingsView(sports: store.sports)) { Label("Sports preferences", systemImage: "sportscourt") } }
                NavigationLink(destination: HomeAssistantView()) { Label("Home Assistant", systemImage: "house") }
            }
            Section("Bluefin") {
                LabeledContentCompat("Version", value: "0.1.0 · Development")
                Text("Development build. No analytics or advertising. Bluefin is an independent client for Bluefin, Ferrofin and Jellyfin servers.").font(.footnote).foregroundStyle(.secondary)
                Link("Source and licenses", destination: URL(string: "https://github.com/glopyglerky/bluefin")!)
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
    @ViewBuilder func bluefinPosterButton() -> some View {
        #if os(tvOS)
        self.buttonStyle(.card)
        #else
        self.buttonStyle(.plain)
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
