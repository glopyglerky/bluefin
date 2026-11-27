import SwiftUI

private enum MusicDestination: String, CaseIterable, Identifiable {
    case home = "Home", library = "Library", search = "Search"
    var id: String { rawValue }
}

/// Music keeps its own browse hierarchy while the six Bluefin media destinations remain available.
struct MusicHubView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var destination = MusicDestination.home
    var body: some View {
        Group {
            switch destination {
            case .home: MusicHomeView()
            case .library: MusicLibraryView()
            case .search: MusicSearchView()
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            Picker("Music destination", selection: $destination) {
                ForEach(MusicDestination.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented).frame(maxWidth: 520)
            .padding(.horizontal, 20).padding(.vertical, 10)
            .frame(maxWidth: .infinity).background(BluefinStyle.background)
        }
        .onChange(of: store.accountContext) { _ in destination = .home }
    }
}

struct MusicHomeView: View {
    @EnvironmentObject private var store: LibraryStore
    @StateObject private var recent = MusicLibraryModel()
    @StateObject private var favorites = MusicLibraryModel()
    @StateObject private var artists = MusicLibraryModel()

    private var recommendations: [MediaItem] {
        var ids = Set<String>()
        return store.home.recommendations.flatMap(\.items).filter {
            ["Audio", "MusicAlbum", "MusicArtist", "Playlist"].contains($0.Type ?? "") && ids.insert($0.Id).inserted
        }
    }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 32) {
                if let featured = recommendations.first ?? recent.items.first {
                    MusicFeaturedCard(item: featured, personalized: !recommendations.isEmpty)
                        .padding(.horizontal, 20)
                }
                if !recommendations.isEmpty { MusicBrowseShelf(title: "Made for You", items: recommendations) }
                if !recent.items.isEmpty { MusicBrowseShelf(title: "Recently added", items: recent.items) }
                if !favorites.items.isEmpty { MusicBrowseShelf(title: "Your favorite albums", items: favorites.items) }
                if !artists.items.isEmpty { MusicBrowseShelf(title: "Artists", items: artists.items, circular: true) }
                if recent.loading { ProgressView("Loading music").frame(maxWidth: .infinity) }
                if let failure = recent.problem {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(failure).foregroundStyle(.secondary)
                        Button("Try again") { Task { await load() } }
                    }.padding(.horizontal, 20)
                } else if recent.items.isEmpty && artists.items.isEmpty && !recent.loading {
                    MusicEmptyState(title: "Your music starts here", message: "Albums, artists and recommendations from your library appear here.")
                }
            }.padding(.vertical, 12).padding(.bottom, 20)
                .frame(maxWidth: BluefinStyle.contentWidth).frame(maxWidth: .infinity)
        }
        .background(BluefinBackground()).navigationTitle("Music")
        .modifier(MusicNavigationTitle(large: true))
        .task(id: store.accountContext) { await load() }
        .bluefinRefresh { await load() }
    }
    private func load() async {
        async let a: Void = recent.load(store: store, type: "MusicAlbum", parent: nil, sort: .added)
        async let b: Void = favorites.load(store: store, type: "MusicAlbum", parent: nil, favorites: true)
        async let c: Void = artists.load(store: store, type: "MusicArtist", parent: nil)
        _ = await (a, b, c)
    }
}

private struct MusicFeaturedCard: View {
    @EnvironmentObject private var store: LibraryStore
    let item: MediaItem
    let personalized: Bool
    var body: some View {
        Group {
            if item.Type == "Audio" {
                Button { Task { await store.playMusic([item]) } } label: { label }
                    .disabled(store.startingPlayback)
            } else {
                NavigationLink(destination: MusicCollection(title: item.title,
                    type: item.Type == "MusicArtist" ? "MusicAlbum" : "Audio", parent: item)) { label }
            }
        }.modifier(MusicContentLinkStyle())
    }
    private var label: some View {
        HStack(alignment: .center, spacing: 24) {
            MusicCover(item: item, radius: 16).frame(maxWidth: 250)
            VStack(alignment: .leading, spacing: 10) {
                Text(personalized ? "PICKED FOR YOU" : "RECENTLY ADDED")
                    .font(.caption.weight(.semibold)).tracking(1)
                Text(item.title).font(.title2.bold()).lineLimit(3)
                if !item.subtitle.isEmpty {
                    Text(item.subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                }
                Label(item.Type == "Audio" ? "Play" : "Listen", systemImage: "play.fill")
                    .font(.headline).padding(.top, 10)
            }.frame(maxWidth: .infinity, alignment: .leading)
        }.padding(20).background(BluefinStyle.secondaryBackground,
            in: RoundedRectangle(cornerRadius: 26, style: .continuous))
    }
}

struct MusicBrowseShelf: View {
    let title: String
    let items: [MediaItem]
    var circular = false
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(title).font(.title2.bold()).padding(.horizontal, 20).accessibilityAddTraits(.isHeader)
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: 18) {
                    ForEach(Array(items.prefix(16))) { item in
                        if item.Type == "Audio" {
                            MusicShelfTrack(item: item, items: items).frame(width: cardWidth)
                        } else {
                            NavigationLink(destination: MusicCollection(title: item.title,
                                type: item.Type == "MusicArtist" ? "MusicAlbum" : "Audio", parent: item)) {
                                VStack(alignment: .leading, spacing: 10) {
                                    MusicCover(item: item, radius: circular ? cardWidth / 2 : 12)
                                    Text(item.title).font(.body.weight(.medium)).lineLimit(2)
                                    if !item.subtitle.isEmpty { Text(item.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                                }.frame(width: cardWidth).accessibilityElement(children: .combine)
                            }.modifier(MusicContentLinkStyle())
                        }
                    }
                }.padding(.horizontal, 20).padding(.vertical, 8)
            }
        }
    }
    private var cardWidth: CGFloat {
        #if os(tvOS)
        260
        #else
        168
        #endif
    }
}

private struct MusicShelfTrack: View {
    @EnvironmentObject private var store: LibraryStore
    let item: MediaItem
    let items: [MediaItem]
    var body: some View {
        Button {
            let tracks = items.filter { $0.Type == "Audio" }
            Task { await store.playMusic(tracks, at: tracks.firstIndex(where: { $0.Id == item.Id }) ?? 0) }
        } label: { MusicShelfCard(item: item) }
        .modifier(MusicContentLinkStyle()).disabled(store.startingPlayback)
    }
}

struct MusicSearchView: View {
    @State private var query = ""
    var body: some View {
        Group {
            if query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { MusicGenreBrowser(showKinds: true) }
            else { UnifiedSearchResults(query: query, initialScope: .music) }
        }
        .navigationTitle("Search").modifier(MusicNavigationTitle(large: true))
        #if os(iOS)
        .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always),
                    prompt: Text("Artists, songs, albums and more"))
        #else
        .searchable(text: $query, prompt: Text("Artists, songs, albums and more"))
        #endif
    }
}

struct MusicGenreBrowser: View {
    var showKinds = false
    @EnvironmentObject private var store: LibraryStore
    @StateObject private var albums = MusicLibraryModel()
    private var genres: [String] {
        Array(Set(albums.items.flatMap { $0.Genres ?? [] }.filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })).sorted {
            $0.localizedStandardCompare($1) == .orderedAscending
        }
    }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 20) {
                Text(showKinds ? "Browse your music" : "Genres").font(.title2.bold())
                    .accessibilityAddTraits(.isHeader)
                LazyVGrid(columns: columns, spacing: 16) {
                    ForEach(Array(genres.enumerated()), id: \.element) { index, genre in
                        NavigationLink(destination: MusicCollection(title: genre, type: "MusicAlbum", genre: genre)) {
                            MusicBrowseTile(title: genre, symbol: "music.note",
                                item: albums.items.first { $0.Genres?.contains(genre) == true }, tint: index)
                        }.modifier(MusicContentLinkStyle())
                    }
                    if showKinds {
                        kind("Songs", type: "Audio", symbol: "music.note", tint: 0)
                        kind("Albums", type: "MusicAlbum", symbol: "square.stack", tint: 1)
                        kind("Artists", type: "MusicArtist", symbol: "music.mic", tint: 2)
                        kind("Playlists", type: "Playlist", symbol: "music.note.list", tint: 3)
                        NavigationLink(destination: MusicCollection(title: "Favorite songs", initialFavorites: true)) {
                            MusicBrowseTile(title: "Favorites", symbol: "star.fill", tint: 4)
                        }.modifier(MusicContentLinkStyle())
                        NavigationLink(destination: MusicCollection(title: "Recently added", type: "MusicAlbum", initialSort: .added)) {
                            MusicBrowseTile(title: "Recently added", symbol: "clock", tint: 5)
                        }.modifier(MusicContentLinkStyle())
                    }

                }
                if albums.loading { ProgressView("Loading genres") }
                if let failure = albums.problem {
                    Text(failure).foregroundStyle(.secondary)
                    Button("Try again") { Task { await load() } }
                } else if genres.isEmpty && !albums.loading && !showKinds {
                    MusicEmptyState(title: "No genres yet", message: "Genre names from your album metadata appear here.")
                }
            }.padding(20).frame(maxWidth: BluefinStyle.contentWidth).frame(maxWidth: .infinity)
        }
        .background(BluefinBackground())
        .task(id: store.accountContext) { await load() }
    }
    private var columns: [GridItem] {
        #if os(tvOS)
        [GridItem(.adaptive(minimum: 320), spacing: 20)]
        #else
        [GridItem(.adaptive(minimum: 145), spacing: 16)]
        #endif
    }
    private func load() async { await albums.load(store: store, type: "MusicAlbum", parent: nil, sort: .added) }
    private func kind(_ title: String, type: String, symbol: String, tint: Int) -> some View {
        NavigationLink(destination: MusicCollection(title: title, type: type)) {
            MusicBrowseTile(title: title, symbol: symbol, tint: tint)
        }.modifier(MusicContentLinkStyle())
    }
}

private struct MusicBrowseTile: View {
    let title: String
    let symbol: String
    var item: MediaItem?
    let tint: Int
    private let tones: [Color] = [
        Color(red: 0.20, green: 0.31, blue: 0.42), Color(red: 0.33, green: 0.30, blue: 0.48),
        Color(red: 0.20, green: 0.39, blue: 0.39), Color(red: 0.40, green: 0.30, blue: 0.39),
        Color(red: 0.30, green: 0.37, blue: 0.51), Color(red: 0.36, green: 0.38, blue: 0.34)]
    var body: some View {
        ZStack(alignment: .bottomLeading) {
            tones[tint % tones.count]
            HStack {
                Spacer(minLength: 34)
                if let item {
                    MusicCover(item: item, radius: 8).frame(width: 104, height: 104)
                        .rotationEffect(.degrees(12)).offset(x: 16, y: 12).opacity(0.72)
                } else {
                    Image(systemName: symbol).font(.system(size: 70, weight: .light))
                        .foregroundStyle(.white.opacity(0.24)).padding(.trailing, 12)
                }
            }
            LinearGradient(colors: [.clear, .black.opacity(0.38)], startPoint: .top, endPoint: .bottom)
            Text(title).font(.title3.bold()).foregroundStyle(.white).lineLimit(2).padding(16)
        }
        .frame(height: 140).clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityElement(children: .ignore).accessibilityLabel(title)
    }
}

/// Album color comes from the actual cached artwork. No stripe texture or moving backdrop.
struct MusicPlayerBackground: View {
    let item: MediaItem
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                BluefinStyle.background
                if !reduceTransparency {
                    Artwork(item: item, thumbnailWidth: 320)
                        .frame(width: geometry.size.width, height: geometry.size.height)
                        .blur(radius: 70, opaque: true).scaleEffect(1.3).opacity(0.25)
                    LinearGradient(colors: [BluefinStyle.background.opacity(0.25), BluefinStyle.background.opacity(0.85)],
                        startPoint: .top, endPoint: .bottom)
                }
            }.clipped()
        }.ignoresSafeArea().accessibilityHidden(true).allowsHitTesting(false)
    }
}
