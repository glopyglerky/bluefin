import SwiftUI
import AVKit
import MediaPlayer

struct MusicLibraryView: View {
    @EnvironmentObject private var store: LibraryStore
    @StateObject private var recent = MusicLibraryModel()
    @StateObject private var favorites = MusicLibraryModel()
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 32) {
                #if os(tvOS)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 360), spacing: 20)], spacing: 20) {
                    navigationItems
                }.buttonStyle(.bordered)
                #else
                VStack(spacing: 0) { navigationItems }.buttonStyle(.plain)
                #endif
                shelf("Recently added", model: recent, sort: .added)
                shelf("Favorite albums", model: favorites, sort: .name, onlyFavorites: true)
                if !recent.loading && !favorites.loading && recent.items.isEmpty && favorites.items.isEmpty && recent.problem == nil {
                    MusicEmptyState(title: "Your music, in one place", message: store.preview ? "Connect your music library to explore albums, artists and playlists." : "Albums from your connected library will appear here. Browse Songs to find individual tracks.")
                }
            }.padding().frame(maxWidth: BluefinStyle.contentWidth).frame(maxWidth: .infinity)
        }.background(BluefinBackground()).navigationTitle("Library").modifier(MusicNavigationTitle(large: true))
        .task(id: "\(store.session?.server ?? "")|\(store.session?.userId ?? "")") {
            await recent.load(store: store, type: "MusicAlbum", parent: nil, sort: .added)
            await favorites.load(store: store, type: "MusicAlbum", parent: nil, favorites: true)
        }
    }
    @ViewBuilder private var navigationItems: some View {
        browse("Playlists", type: "Playlist", symbol: "music.note.list")
        browse("Artists", type: "MusicArtist", symbol: "music.mic")
        browse("Albums", type: "MusicAlbum", symbol: "square.stack")
        browse("Songs", type: "Audio", symbol: "music.note")
        NavigationLink(destination: MusicCollection(title: "Favorite songs", initialFavorites: true)) {
            navigationLabel("Made for You", symbol: "person.crop.square")
        }
        NavigationLink(destination: ItemGrid(title: "Music Videos", parent: nil, includeItemTypes: "MusicVideo")) {
            navigationLabel("Music Videos", symbol: "play.rectangle")
        }
        NavigationLink(destination: MusicGenreBrowser()) {
            navigationLabel("Genres", symbol: "guitars")
        }
        NavigationLink(destination: DownloadsView(offline: store.offline)) {
            navigationLabel("Downloads", symbol: "arrow.down.circle")
        }
        #if !os(tvOS)
        Button { store.deviceMusicVisible = true } label: {
            navigationLabel("On this device", symbol: "music.note.house")
        }
        #endif
    }
    private func browse(_ title: String, type: String, symbol: String) -> some View {
        NavigationLink(destination: MusicCollection(title: title, type: type)) {
            navigationLabel(title, symbol: symbol)
        }
    }
    private func navigationLabel(_ title: String, symbol: String) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 20) {
                Image(systemName: symbol).font(.system(size: 28, weight: .regular)).frame(width: 38).foregroundStyle(BluefinStyle.text)
                Text(title).font(.title2)
                Spacer()
                Image(systemName: "chevron.right").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity, minHeight: 68).padding(.vertical, 8).contentShape(Rectangle())
            #if !os(tvOS)
            Divider().padding(.leading, 58)
            #endif
        }
    }
    private func shelf(_ title: String, model: MusicLibraryModel, sort: MusicSort, onlyFavorites: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            if !model.items.isEmpty {
                NavigationLink(destination: MusicCollection(title: title, type: "MusicAlbum", initialSort: sort, initialFavorites: onlyFavorites)) {
                    HStack { Text(title).font(.title2.bold()); Spacer(); Image(systemName: "chevron.right").font(.subheadline) }
                }.buttonStyle(.plain)
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHStack(alignment: .top, spacing: 20) {
                        ForEach(Array(model.items.prefix(12))) { item in
                            NavigationLink(destination: MusicCollection(title: item.title, type: "Audio", parent: item)) { MusicShelfCard(item: item).frame(width: shelfWidth) }
                                .modifier(MusicContentLinkStyle())
                        }
                    }.padding(.vertical, 6)
                }
            }
            if model.loading { ProgressView("Loading \(title.lowercased())") }
            if let problem = model.problem {
                Text(problem).font(.callout).foregroundStyle(.secondary)
                Button("Try again") { Task { await model.load(store: store, type: "MusicAlbum", parent: nil, sort: sort, favorites: onlyFavorites) } }
            }
        }
    }
    private var shelfWidth: CGFloat {
        #if os(tvOS)
        return 250
        #else
        return 170
        #endif
    }
}

struct MusicCollection: View {
    let title: String
    var type: String = "Audio"
    var parent: MediaItem?
    var initialSort: MusicSort = .name
    var initialFavorites = false
    var genre = ""
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @StateObject private var model = MusicLibraryModel()
    @State private var query = ""
    @State private var sort: MusicSort = .name
    @State private var favorites = false
    @State private var playlistPicker = false
    @State private var playlistSettings = false
    @State private var selectedTracks: [MediaItem] = []
    @State private var refresh = UUID()
    @State private var fromLetter = ""
    init(title: String, type: String = "Audio", parent: MediaItem? = nil,
         initialSort: MusicSort = .name, initialFavorites: Bool = false, genre: String = "") {
        self.title = title; self.type = type; self.parent = parent
        self.initialSort = initialSort; self.initialFavorites = initialFavorites; self.genre = genre
        _sort = State(initialValue: initialSort); _favorites = State(initialValue: initialFavorites)
    }
    private var isAlbum: Bool { parent?.Type == "MusicAlbum" }
    private var isPlaylist: Bool { parent?.Type == "Playlist" }
    private var isTracks: Bool { type == "Audio" }
    private var requestKey: String { "\(query)|\(sort.rawValue)|\(favorites)|\(fromLetter)|\(refresh)|\(store.session?.server ?? "")|\(store.session?.userId ?? "")" }
    private var letters: [String] { "ABCDEFGHIJKLMNOPQRSTUVWXYZ".map(String.init) }
    var body: some View {
        ScrollViewReader { reader in
            List {
                if let parent { Section { collectionHeader(parent).listRowBackground(Color.clear) } }
                if let parent, let identity = PublicCatalogIdentity.library(parent) {
                    PublicCatalogAccessSection(identity: identity)
                }
                if !isPlaylist && !isAlbum {
                    Section {
                        HStack {
                            if !isAlbum { Picker("Sort", selection: $sort) { ForEach(MusicSort.allCases) { Text($0.rawValue).tag($0) } } }
                            Toggle("Favorites", isOn: $favorites)
                        }
                        if sort == .name && !isAlbum && !model.items.isEmpty {
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack {
                                    Button("All") { fromLetter = "" }.frame(minWidth: 44, minHeight: 44)
                                    ForEach(letters, id: \.self) { letter in
                                        Button(letter) {
                                            fromLetter = letter
                                        }.frame(minWidth: 44, minHeight: 44).accessibilityLabel("Browse from \(letter)")
                                    }
                                }
                            }
                        }
                    }
                }
                if isTracks && !model.items.isEmpty {
                    Section {
                        MusicPlayActions(pageOnly: model.hasNext || model.hasPrevious, disabled: store.startingPlayback) { shuffled in
                            Task { await store.playMusic(model.items, shuffled: shuffled) }
                        }
                    }
                }
                if isTracks {
                    Section {
                        ForEach(Array(model.items.enumerated()), id: \.offset) { index, item in
                            Button { Task { await store.playMusic(model.items, at: index) } } label: {
                                MusicRow(item: item, trackNumber: isAlbum ? item.IndexNumber : nil)
                            }.modifier(MusicContentLinkStyle()).disabled(store.startingPlayback).id(isPlaylist ? "\(index):\(item.Id)" : item.Id)
                                .contextMenu { trackMenu(item) }
                        }
                    }
                } else {
                    Section {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: cardWidth), spacing: 20)], alignment: .leading, spacing: 26) {
                            ForEach(model.items) { item in
                                NavigationLink(destination: MusicCollection(title: item.title, type: item.Type == "MusicArtist" ? "MusicAlbum" : "Audio", parent: item)) { MusicShelfCard(item: item) }.id(item.Id)
                            }
                        }.padding(.vertical, 12).listRowBackground(Color.clear)
                    }
                }
                Section {
                    if model.loading { ProgressView("Loading music") }
                    if let problem = model.problem {
                        Text(problem).foregroundStyle(.secondary)
                        Button("Try again") { Task { await load(start: model.start) } }
                    } else if model.items.isEmpty && !model.loading {
                        MusicEmptyState(title: "No music found", message: query.isEmpty ? "Try another filter or add music to your server library." : "Try a different search.")
                    }
                    if !model.loading && model.problem == nil && (model.hasPrevious || model.hasNext) {
                        Text("\(model.start + 1)–\(model.start + model.items.count) of \(model.total)").font(.footnote.monospacedDigit()).foregroundStyle(.secondary)
                        HStack {
                            Button("Previous page") { Task { await load(start: max(0, model.start - 60)) } }.disabled(!model.hasPrevious)
                            Spacer()
                            Button("Next page") { Task { await load(start: model.start + model.pageCount) } }.disabled(!model.hasNext)
                        }
                    }
                }
            }.navigationTitle(title).modifier(MusicNavigationTitle(large: parent == nil)).modifier(MusicSearch(text: $query, enabled: !isPlaylist, prompt: "Search \(title.lowercased())"))
                .background(BluefinBackground())
                .toolbar {
                    ToolbarItem(placement: .primaryAction) {
                        HStack {
                            if parent != nil && isPlaylist {
                                Button { playlistSettings = true } label: { Image(systemName: "ellipsis.circle") }.accessibilityLabel("Playlist settings")
                            }
                            if type == "Playlist" && parent == nil {
                                Button { selectedTracks = []; playlistPicker = true } label: { Image(systemName: "plus") }.accessibilityLabel("Create playlist")
                            }
                        }
                    }
                }
                .sheet(isPresented: $playlistPicker, onDismiss: { refresh = UUID() }) { MusicPlaylistPicker(items: selectedTracks).environmentObject(store) }
                .sheet(isPresented: $playlistSettings, onDismiss: { refresh = UUID() }) { if let parent { MusicPlaylistSettings(playlist: parent).environmentObject(store) } }
                .task(id: requestKey) {
                    do {
                        if !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            try await Task.sleep(nanoseconds: 250_000_000)
                        }
                        try Task.checkCancellation(); await load()
                    } catch {}
                }
        }
    }
    private var cardWidth: CGFloat {
        #if os(tvOS)
        return 240
        #else
        return 140
        #endif
    }
    private func collectionHeader(_ item: MediaItem) -> some View {
        VStack(spacing: 16) {
            MusicCover(item: item).frame(maxWidth: 256).shadow(color: .black.opacity(0.14), radius: 20, y: 10)
            Text(item.title).font(.title2.bold()).multilineTextAlignment(.center)
            if !item.subtitle.isEmpty { Text(item.subtitle).font(.title3).foregroundStyle(.secondary).multilineTextAlignment(.center) }
            if let year = item.ProductionYear { Text(String(year)).font(.subheadline).foregroundStyle(.secondary) }
        }.frame(maxWidth: .infinity).padding(.vertical, 12).accessibilityElement(children: .combine)
    }
    @ViewBuilder private func trackMenu(_ item: MediaItem) -> some View {
        Button { store.enqueueMusic([item], next: true) } label: { Label("Play next", systemImage: "text.line.first.and.arrowtriangle.forward") }
        Button { store.enqueueMusic([item], next: false) } label: { Label("Add to queue", systemImage: "text.badge.plus") }
        Button { selectedTracks = [item]; playlistPicker = true } label: { Label("Add to playlist", systemImage: "music.note.list") }
        if isPlaylist, let parent, item.PlaylistItemId != nil {
            Button(role: .destructive) { Task { do { try await store.removePlaylistItems(playlist: parent, entries: [item]); await load(start: model.start) } catch { store.error = error.localizedDescription } } } label: { Label("Remove from playlist", systemImage: "minus.circle") }
        }
        if let session = store.session {
            Button { Task { await store.offline.download(item, session: session) } } label: { Label("Download", systemImage: "arrow.down.circle") }
        }
    }
    private func load(start: Int = 0) async {
        await model.load(store: store, type: type, parent: parent, start: start, query: isPlaylist ? "" : query, sort: sort, favorites: isPlaylist ? false : favorites, fromLetter: isPlaylist || isAlbum || sort != .name ? "" : fromLetter, genre: genre)
    }
}

struct MusicMiniPlayer: View {
    @ObservedObject var playback: PlaybackController
    var nativeAccessory = false
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        HStack(spacing: 16) {
            Button { store.musicExpanded = true } label: {
                HStack(spacing: 14) {
                    Artwork(item: playback.item).frame(width: 44, height: 44).clipShape(RoundedRectangle(cornerRadius: 7))
                    VStack(alignment: .leading, spacing: 3) {
                        Text(playback.item.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                        Text(playback.item.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer()
                }
            }.buttonStyle(.plain).accessibilityLabel("Now playing, \(playback.item.title)")
            Button { playback.toggle() } label: { Image(systemName: playback.isPlaying ? "pause.fill" : "play.fill").font(.title3).frame(width: 44, height: 44) }
                .accessibilityLabel(playback.isPlaying ? "Pause" : "Play")
            Button { Task { await store.skipMusic(1) } } label: { Image(systemName: "forward.fill").frame(width: 44, height: 44) }
                .accessibilityLabel("Next track").disabled(store.startingPlayback || (store.musicRepeat != .all && store.musicIndex + 1 >= store.musicQueue.count))
        }.padding(.horizontal, nativeAccessory ? 12 : 20).padding(.vertical, nativeAccessory ? 4 : 10)
            .modifier(BluefinGlassSurface(enabled: !nativeAccessory, cornerRadius: 26))
            .padding(.horizontal, nativeAccessory ? 0 : 12)
            .padding(.vertical, nativeAccessory ? 0 : 5)
    }
}

#if os(iOS)
@available(iOS 26, *)
struct MusicNativeAccessory: View {
    @ObservedObject var playback: PlaybackController
    @Environment(\.tabViewBottomAccessoryPlacement) private var placement
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        if placement == .inline {
            HStack(spacing: 8) {
                Button { store.musicExpanded = true } label: {
                    Text(playback.item.title).font(.subheadline).lineLimit(1)
                }.buttonStyle(.plain).accessibilityLabel("Now playing, \(playback.item.title)")
                Button { playback.toggle() } label: {
                    Image(systemName: playback.isPlaying ? "pause.fill" : "play.fill").frame(minWidth: 44, minHeight: 44)
                }.buttonStyle(.plain).accessibilityLabel(playback.isPlaying ? "Pause" : "Play")
            }
        } else { MusicMiniPlayer(playback: playback, nativeAccessory: true) }
    }
}
#endif

struct MusicPlayer: View {
    @ObservedObject var playback: PlaybackController
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @State private var queueVisible = false
    @State private var lyricsVisible = false
    @State private var addToPlaylist = false
    @State private var qualityVisible = false
    @State private var integrationsVisible = false
    @State private var favorite = false
    @State private var favoriteBusy = false
    @State private var problem: String?
    private var duration: Double { max(1, Double(playback.item.RunTimeTicks ?? 0) / 10_000_000) }
    var body: some View {
        GeometryReader { geometry in
            VStack(spacing: 0) {
                playerHeader.padding(.horizontal, 20).padding(.vertical, 12)
                if lyricsVisible {
                    lyricLayout(size: geometry.size)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        playerLayout(size: geometry.size)
                            .padding(.horizontal, 28).padding(.top, 18).padding(.bottom, 24)
                            .frame(maxWidth: .infinity)
                    }
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(MusicPlayerBackground(item: playback.item))
        .onAppear { favorite = playback.item.UserData?.IsFavorite == true }
        .onChange(of: playback.item.Id) { _ in favorite = playback.isFavorite }
        .onChange(of: playback.isFavorite) { favorite = $0 }
        .sheet(isPresented: $queueVisible) { queue.environmentObject(store) }
        .sheet(isPresented: $addToPlaylist) { MusicPlaylistPicker(items: [playback.item]).environmentObject(store) }
        .sheet(isPresented: $qualityVisible) { AudioQualityView(playback: playback) }
        .sheet(isPresented: $integrationsVisible) {
            BluefinNavigation {
                Form { NativeIntegrationActions(item: playback.item) }.navigationTitle("Sharing and apps")
                    .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { integrationsVisible = false } } }
            }.environmentObject(store)
        }
        .alert("Music", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) { Button("OK") { problem = nil } } message: { Text(problem ?? "") }
    }
    @ViewBuilder private func lyricLayout(size: CGSize) -> some View {
        #if os(tvOS)
        HStack(spacing: 34) {
            ScrollView {
                VStack(spacing: 24) { cover.frame(maxWidth: 330); controls }
                    .frame(maxWidth: 480).padding(24)
            }.frame(maxWidth: 520)
            MusicLyricsView(playback: playback, embedded: true)
        }.padding(.horizontal, 24)
        #else
        MusicLyricsView(playback: playback, embedded: true)
        #endif
    }
    private var playerHeader: some View {
        HStack {
            Button { if lyricsVisible { lyricsVisible = false } else { dismiss() } } label: {
                Image(systemName: lyricsVisible ? "chevron.left" : "chevron.down").frame(width: 44, height: 44)
            }.buttonStyle(.plain).modifier(BluefinGlassSurface(cornerRadius: 22))
                .accessibilityLabel(lyricsVisible ? "Back to album artwork" : "Close Now Playing")
            Spacer()
            Text(lyricsVisible ? (playback.karaoke == nil ? "LYRICS" : "SING") : "NOW PLAYING")
                .font(.caption.weight(.semibold)).tracking(2).foregroundStyle(.secondary)
            Spacer()
            headerActions
        }.frame(maxWidth: 1100)
    }
    // Keep the availability check out of the surrounding stack's result builder.
    // Menu is available on iOS 14, but requires tvOS 17.
    private var headerActions: some View {
        if #available(iOS 14, tvOS 17, *) {
            return AnyView(headerMenu)
        } else {
            return AnyView(Group {
                Button("Add to playlist") { addToPlaylist = true }
                Button("Sharing and apps") { integrationsVisible = true }
            })
        }
    }
    @available(iOS 14, tvOS 17, *)
    private var headerMenu: some View {
        Menu {
            Button { addToPlaylist = true } label: { Label("Add to playlist", systemImage: "music.note.list") }
            Button { integrationsVisible = true } label: { Label("Sharing and apps", systemImage: "square.and.arrow.up") }
        } label: { Image(systemName: "ellipsis").frame(width: 44, height: 44) }
            .modifier(BluefinGlassSurface(cornerRadius: 22)).accessibilityLabel("More music actions")
    }
    @ViewBuilder private func playerLayout(size: CGSize) -> some View {
        #if os(tvOS)
        HStack(alignment: .center, spacing: 70) { cover.frame(maxWidth: 600); controls.frame(maxWidth: 650) }
        #else
        let wide = (size.width >= 760 || (size.width >= 600 && size.width > size.height)) && !dynamicTypeSize.isAccessibilitySize
        let artworkSize = max(120, min(wide ? (size.width - 76) * 0.45 : size.width - 40,
                                        wide ? size.height - 140 : max(220, size.height - 460), 480))
        if #available(iOS 16, *) {
            let layout = wide ? AnyLayout(HStackLayout(alignment: .top, spacing: 36)) : AnyLayout(VStackLayout(spacing: 30))
            layout { cover.frame(width: artworkSize, height: artworkSize); controls.frame(maxWidth: 520) }
                .frame(maxWidth: wide ? 1100 : 520)
        } else {
            Group {
                if wide {
                    HStack(alignment: .top, spacing: 28) { cover.frame(width: artworkSize, height: artworkSize); controls.frame(maxWidth: 520) }
                } else {
                    VStack(spacing: 24) { cover.frame(width: artworkSize, height: artworkSize); controls }
                }
            }.frame(maxWidth: wide ? 1100 : 520)
        }
        #endif
    }
    private var cover: some View {
        Artwork(item: playback.item).aspectRatio(1, contentMode: .fit).clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous)).shadow(color: .black.opacity(0.12), radius: 20, y: 12)
    }
    private var controls: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 8) {
                Text(playback.item.title).font(.title.bold()).fixedSize(horizontal: false, vertical: true)
                Text(playback.item.subtitle).font(.title3).foregroundStyle(.secondary)
            }
            qualityBadge
            if (playback.item.RunTimeTicks ?? 0) > 0 {
            VStack(spacing: 8) {
                #if os(iOS)
                Slider(value: Binding(get: { min(duration, playback.positionSeconds) }, set: { playback.seek(seconds: $0) }), in: 0...duration).accessibilityLabel("Playback position")
                #else
                ProgressView(value: min(duration, playback.positionSeconds), total: duration).tint(BluefinStyle.accent)
                HStack {
                    Button { playback.seek(seconds: max(0, playback.positionSeconds - 15)) } label: {
                        Image(systemName: "gobackward.15").frame(minWidth: 44, minHeight: 44)
                    }.accessibilityLabel("Back 15 seconds")
                    Button { playback.seek(seconds: min(duration, playback.positionSeconds + 15)) } label: {
                        Image(systemName: "goforward.15").frame(minWidth: 44, minHeight: 44)
                    }.accessibilityLabel("Forward 15 seconds")
                }
                #endif
                HStack { Text(time(playback.positionSeconds)); Spacer(); Text("−" + time(max(0, duration - playback.positionSeconds))) }.font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            }
            HStack(spacing: 0) {
                Button { store.toggleMusicShuffle() } label: {
                    Image(systemName: "shuffle").frame(minWidth: 44, minHeight: 44)
                        .foregroundStyle(store.musicShuffled ? BluefinStyle.accent : .secondary)
                }.accessibilityLabel("Shuffle").accessibilityValue(store.musicShuffled ? "On" : "Off").frame(maxWidth: .infinity)
                Button { Task { await store.skipMusic(-1) } } label: { Image(systemName: "backward.fill").font(.title).frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Previous track").disabled(store.startingPlayback || (store.musicRepeat != .all && store.musicIndex <= 0)).frame(maxWidth: .infinity)
                Button { playback.toggle() } label: { Image(systemName: playback.isPlaying ? "pause.fill" : "play.fill").font(.system(size: 42)).frame(width: 70, height: 70) }.accessibilityLabel(playback.isPlaying ? "Pause" : "Play")
                Button { Task { await store.skipMusic(1) } } label: { Image(systemName: "forward.fill").font(.title).frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Next track").disabled(store.startingPlayback || (store.musicRepeat != .all && store.musicIndex + 1 >= store.musicQueue.count)).frame(maxWidth: .infinity)
                Button { store.cycleMusicRepeat() } label: {
                    Image(systemName: store.musicRepeat == .one ? "repeat.1" : "repeat").frame(minWidth: 44, minHeight: 44)
                        .foregroundStyle(store.musicRepeat == .off ? .secondary : BluefinStyle.accent)
                }.accessibilityLabel("Repeat").accessibilityValue(store.musicRepeat == .one ? "One track" : store.musicRepeat == .all ? "All tracks" : "Off").frame(maxWidth: .infinity)
            }
            HStack(spacing: 16) {
                Button { Task { await saveFavorite() } } label: { Image(systemName: favorite ? "star.fill" : "star").frame(width: 44, height: 44) }.accessibilityLabel(favorite ? "Remove favorite" : "Add favorite").disabled(favoriteBusy)
                Spacer()
                Button { lyricsVisible = true } label: { Image(systemName: "quote.bubble").frame(width: 44, height: 44) }.accessibilityLabel("Lyrics and Sing")
                Spacer()
                #if os(iOS)
                MusicRoutePicker().frame(width: 44, height: 44).accessibilityLabel("Choose audio output")
                #endif
                Spacer()
                Button { queueVisible = true } label: { Image(systemName: "list.bullet").frame(width: 44, height: 44) }.accessibilityLabel("Up next queue")
            }.buttonStyle(.plain).padding(.horizontal, 12).padding(.vertical, 5)
                .modifier(BluefinGlassSurface(cornerRadius: 27))
            #if os(iOS)
            HStack(spacing: 12) {
                Image(systemName: "speaker.fill").foregroundStyle(.secondary).accessibilityHidden(true)
                MusicVolumeControl().frame(minHeight: 44)
                Image(systemName: "speaker.wave.3.fill").foregroundStyle(.secondary).accessibilityHidden(true)
            }
            #endif
            if let failure = playback.failure { Text(failure).font(.callout).foregroundColor(.red) }
            if let failure = playback.reportingFailure { Text(failure).font(.footnote).foregroundColor(.orange) }
            LocalAssistancePlayback(status: playback.failure != nil ? .failed : playback.isBuffering ? .buffering : playback.isStopped ? .stopped : playback.isPlaying ? .playing : .paused,
                context: store.accountContext)
        }
    }
    private var qualityBadge: some View {
        Button { qualityVisible = true } label: {
            Label(AudioQualitySummary.badge(stream: playback.audioSourceStream, method: playback.playMethod,
                                            singing: playback.karaoke != nil), systemImage: "waveform")
                .font(.caption.weight(.medium)).foregroundStyle(.secondary).frame(minHeight: 28)
        }.buttonStyle(.plain).accessibilityHint("View audio quality and output details")
    }
    private var queue: some View {
        NavigationView {
            List {
                Section {
                    Text(playback.item.title).font(.headline)
                    Text("Now playing").font(.caption).foregroundStyle(.secondary)
                }
                Section {
                    HStack {
                        Button { store.toggleMusicShuffle() } label: { Label(store.musicShuffled ? "Shuffle on" : "Shuffle off", systemImage: "shuffle") }
                        Spacer()
                        Button { store.cycleMusicRepeat() } label: { Label(store.musicRepeat == .one ? "Repeat one" : store.musicRepeat == .all ? "Repeat all" : "Repeat off", systemImage: store.musicRepeat == .one ? "repeat.1" : "repeat") }
                    }
                }
                Section {
                    ForEach(Array(store.musicQueue.enumerated()), id: \.offset) { index, item in
                        Button { Task { await store.playMusic(store.musicQueue, at: index); queueVisible = false } } label: {
                            HStack {
                                MusicRow(item: item)
                                if index == store.musicIndex { Image(systemName: "waveform").foregroundStyle(BluefinStyle.accent).accessibilityLabel("Now playing") }
                            }
                        }.disabled(store.startingPlayback)
                            .contextMenu {
                                if index > 0 { Button("Move earlier") { store.moveMusic(from: IndexSet(integer: index), to: index - 1) } }
                                if index + 1 < store.musicQueue.count { Button("Move later") { store.moveMusic(from: IndexSet(integer: index), to: index + 2) } }
                                Button("Remove from queue", role: .destructive) { store.removeMusic(at: IndexSet(integer: index)) }
                            }
                    }
                    .onMove { store.moveMusic(from: $0, to: $1) }
                    .onDelete { store.removeMusic(at: $0) }
                } footer: {
                    Text("Queue edits change what plays next. Clear keeps the current track.")
                }
                Section {
                    if !store.preview { NavigationLink("Queue sync", destination: QueueSyncView()) }
                    Button("Clear queue", role: .destructive) { store.clearMusicQueue() }.disabled(store.musicQueue.isEmpty)
                }
            }.navigationTitle("Up next").toolbar {
                ToolbarItem(placement: .primaryAction) { Button("Done") { queueVisible = false } }
                #if os(iOS)
                ToolbarItem(placement: .navigationBarLeading) { EditButton() }
                #endif
            }
        }.navigationViewStyle(.stack)
    }
    private func saveFavorite() async {
        favoriteBusy = true
        defer { favoriteBusy = false }
        let item = playback.item
        let value = !favorite
        do { try await store.setFavorite(item, value: value); if playback.item.Id == item.Id { favorite = value } }
        catch { problem = error.localizedDescription }
    }
    private func time(_ seconds: Double) -> String {
        let value = seconds.isFinite ? Int(max(0, min(seconds, Double(Int.max / 2)))) : 0
        return "\(value / 60):\(String(format: "%02d", value % 60))"
    }
}

#if os(iOS)
struct MusicVolumeControl: UIViewRepresentable {
    func makeUIView(context: Context) -> MPVolumeView {
        let view = MPVolumeView()
        view.showsRouteButton = false
        view.accessibilityLabel = "Output volume"
        return view
    }
    func updateUIView(_ uiView: MPVolumeView, context: Context) {}
}

struct MusicRoutePicker: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let view = AVRoutePickerView()
        view.prioritizesVideoDevices = false
        return view
    }
    func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}
#endif
