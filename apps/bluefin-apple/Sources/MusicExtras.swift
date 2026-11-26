import SwiftUI

struct MusicLyricsView: View {
    @ObservedObject var playback: PlaybackController
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @State private var lyrics: BluefinLyrics?
    @State private var lyricRuns: [[LyricDisplayRun]] = []
    @State private var failure: String?
    @State private var loading = false
    @State private var following = true
    @State private var singingControls = false
    @State private var lyricRequest = UUID()
    @State private var preparing: Task<Void, Never>?
    @State private var preparationStatus: String?
    @State private var karaokeFailure: String?
    @State private var preparationID = UUID()
    private var identity: String { "\(store.session?.server ?? "")|\(store.session?.userId ?? "")|\(playback.item.Id)" }
    private var hasWordTiming: Bool { lyricRuns.contains { $0.contains { $0.start != nil && $0.end != nil } } }

    var body: some View {
        NavigationView {
            Group {
                if let karaoke = playback.karaoke {
                    KaraokeLyricsView(playback: playback, audio: karaoke)
                } else {
                    VStack(spacing: 0) { trackHeader; lyricContent; singingBar }.background(BluefinBackground())
                }
            }.navigationTitle(playback.karaoke == nil ? "Lyrics" : "Sing")
                .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { dismiss() } } }
                .task(id: identity + "|karaoke=\(playback.karaoke != nil)") {
                    guard playback.karaoke == nil else { return }
                    let request = UUID(); lyricRequest = request
                    loading = true; failure = nil; lyrics = nil; lyricRuns = []; following = true
                    defer { if lyricRequest == request { loading = false } }
                    do {
                        let value = try await store.lyrics(item: playback.item)
                        try Task.checkCancellation()
                        guard lyricRequest == request else { return }
                        lyrics = value; lyricRuns = value.Lyrics.map(\.displayRuns)
                    } catch is CancellationError {} catch {
                        if lyricRequest == request && !Task.isCancelled { failure = error.localizedDescription }
                    }
                }
        }.navigationViewStyle(.stack)
            .onChange(of: identity) { _ in cancelPreparation(); playback.stopKaraokeMicrophones() }
            .onDisappear { cancelPreparation(); playback.stopKaraokeMicrophones() }
    }
    private var trackHeader: some View {
        HStack(spacing: 14) {
            Artwork(item: playback.item).frame(width: 52, height: 52).clipShape(RoundedRectangle(cornerRadius: 9))
            VStack(alignment: .leading, spacing: 4) {
                Text(playback.item.title).font(.headline).lineLimit(2)
                Text(playback.item.subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
        }.padding(.horizontal, 28).padding(.vertical, 12).frame(maxWidth: 820)
    }
    @ViewBuilder private var lyricContent: some View {
        if let lyrics, !lyrics.Lyrics.isEmpty {
            // A media-clock sample stays in sync through pause, seek and buffering.
            TimelineView(.animation(minimumInterval: hasWordTiming && !reduceMotion ? 1.0 / 30 : 1.0 / 8,
                                    paused: !playback.isPlaying)) { _ in
                let seconds = playback.lyricPositionSeconds
                let active = playback.isStopped ? nil : lyrics.activeIndex(at: seconds)
                ScrollViewReader { reader in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 28) {
                            ForEach(Array(lyrics.Lyrics.enumerated()), id: \.offset) { index, line in
                                Group {
                                    if let seek = lyrics.seekSeconds(for: index) {
                                        Button { playback.seek(seconds: seek); following = true } label: {
                                            lyricLine(line, index: index, active: active, seconds: seconds - lyrics.offsetSeconds)
                                        }.buttonStyle(.plain).accessibilityLabel(line.Text.isEmpty ? "Instrumental" : line.Text)
                                            .accessibilityHint("Play from this lyric")
                                    } else { lyricLine(line, index: index, active: active, seconds: seconds - lyrics.offsetSeconds) }
                                }.id(index)
                            }
                        }.padding(.horizontal, 28).padding(.vertical, 32).frame(maxWidth: 820).frame(maxWidth: .infinity)
                    }
                    #if os(iOS)
                    .simultaneousGesture(DragGesture(minimumDistance: 12).onChanged { _ in following = false })
                    #endif
                    .onAppear { if let active { reader.scrollTo(active, anchor: .center) } }
                    .onChange(of: active) { index in
                        guard following, let index else { return }
                        withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.25)) { reader.scrollTo(index, anchor: .center) }
                    }
                    .onChange(of: following) { enabled in
                        guard enabled, let active else { return }
                        withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.25)) { reader.scrollTo(active, anchor: .center) }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        if !following && lyrics.isSynced {
                            Button("Follow lyrics") {
                                following = true
                            }.padding(.horizontal, 16).padding(.vertical, 12)
                                .modifier(BluefinGlassSurface(cornerRadius: 24)).padding()
                        }
                    }
                }
            }
        } else {
            VStack(spacing: 14) {
                Spacer()
                if loading { ProgressView("Loading lyrics") }
                else {
                    Image(systemName: "quote.bubble").font(.system(size: 42)).foregroundStyle(.secondary)
                    Text(failure == nil ? "Lyrics aren’t available for this song." : "Lyrics couldn’t load.").font(.title3.bold())
                    if let failure { Text(failure).font(.callout).foregroundStyle(.secondary) }
                    Text("You can still sing along with the original track.").font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
            }.multilineTextAlignment(.center).padding(28).frame(maxWidth: .infinity)
        }
    }
    @ViewBuilder private func lyricLine(_ line: BluefinLyrics.Line, index: Int, active: Int?, seconds: Double) -> some View {
        let runs = lyricRuns.indices.contains(index) ? lyricRuns[index] : []
        if active == index && runs.contains(where: { $0.start != nil && $0.end != nil }) {
            TimedLyricsWordFill(runs: runs, seconds: seconds, color: BluefinStyle.text).accessibilityLabel(line.Text)
        } else {
            Text(line.Text.isEmpty ? "♪" : line.Text).font(.system(.title, design: .rounded).bold())
                .foregroundStyle(BluefinStyle.text.opacity(active == nil || active == index ? 1 : 0.42))
                .multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
        }
    }
    private var singingBar: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 20) {
                Button { playback.toggle() } label: {
                    Label(playback.isPlaying ? "Pause" : "Play", systemImage: playback.isPlaying ? "pause.fill" : "play.fill").frame(minHeight: 44)
                }.accessibilityLabel(playback.isPlaying ? "Pause song" : "Play song")
                Spacer()
                Button { singingControls.toggle() } label: {
                    Label("Sing", systemImage: "mic.fill").frame(minHeight: 44)
                }.accessibilityValue(singingControls ? "Controls shown" : "Controls hidden")
                #if os(tvOS)
                Button(following ? "Browse lyrics" : "Follow lyrics") { following.toggle() }
                #endif
            }
            if singingControls {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        #if os(tvOS)
                        if #available(tvOS 17.0, *) { KaraokeContinuityControls(microphone: playback.continuityMicrophone) }
                        else { Text("An iPhone microphone requires tvOS 17 or later and a compatible Apple TV.").font(.callout).foregroundStyle(.secondary) }
                        #else
                        KaraokeMicrophoneControls(microphone: playback.karaokeMicrophone)
                        #endif
                        Divider()
                        Button(preparing == nil ? "Adjust lead vocals" : "Cancel preparation") {
                            if preparing != nil { cancelPreparation() } else { startSinging() }
                        }.disabled(store.session == nil && preparing == nil)
                        if let preparationStatus { ProgressView(preparationStatus) }
                        else if let karaokeFailure { Text(karaokeFailure).font(.callout).foregroundStyle(.secondary) }
                        else { Text("Vocal controls need prepared audio and timing from your server.").font(.caption).foregroundStyle(.secondary) }
                    }.padding(.bottom, 4)
                }.frame(maxHeight: dynamicTypeSize.isAccessibilitySize ? 320 : 230)
            }
        }.padding(20).frame(maxWidth: 820).modifier(BluefinGlassSurface(cornerRadius: 28))
            .padding(.horizontal, 16).padding(.bottom, 12)
    }
    @MainActor private func cancelPreparation() {
        preparationID = UUID(); preparing?.cancel(); preparing = nil; preparationStatus = nil; karaokeFailure = nil
    }
    @MainActor private func startSinging() {
        guard let session = store.session else { return }
        let request = UUID(); preparationID = request; karaokeFailure = nil
        preparing = Task { @MainActor in
            defer { if preparationID == request { preparing = nil; preparationStatus = nil } }
            do {
                try await KaraokePreparation.enable(playback: playback, session: session, currentSession: { store.session }, status: {
                    if preparationID == request { preparationStatus = $0 }
                })
            } catch is CancellationError {} catch {
                if preparationID == request && !Task.isCancelled {
                    karaokeFailure = error.localizedDescription.contains("404") ? "Vocal controls aren’t available for this song. You can still use the microphone and original audio." : error.localizedDescription
                }
            }
        }
    }
}

struct MusicPlaylistPicker: View {
    let items: [MediaItem]
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var playlists: [MediaItem] = []
    @State private var total = 0
    @State private var name = ""
    @State private var busy = false
    @State private var failure: String?
    var body: some View {
        NavigationView {
            List {
                Section("New playlist") {
                    TextField("Playlist name", text: $name)
                    Button(items.isEmpty ? "Create" : "Create and add") {
                        Task { await perform { _ = try await store.createPlaylist(name: name.trimmingCharacters(in: .whitespacesAndNewlines), items: items) } }
                    }.disabled(busy || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                if !items.isEmpty { Section("Add to playlist") {
                    ForEach(playlists) { playlist in
                        Button(playlist.title) { Task { await perform { try await store.addPlaylistItems(playlist: playlist, items: items) } } }.disabled(busy)
                    }
                    if playlists.count < total { Button("Load more") { Task { await load() } }.disabled(busy) }
                }
                }
                if busy { ProgressView() }
                if let failure { Text(failure).foregroundStyle(.secondary) }
            }.navigationTitle("Add to playlist")
                .toolbar { ToolbarItem(placement: .primaryAction) { Button("Cancel") { dismiss() } } }
                .task { if !items.isEmpty { await load() } }
        }.navigationViewStyle(.stack)
    }
    private func load() async {
        busy = true; defer { busy = false }
        do {
            let page = try await store.musicItems(type: "Playlist", start: playlists.count)
            let seen = Set(playlists.map(\.Id)); playlists += page.Items.filter { !seen.contains($0.Id) }
            total = page.TotalRecordCount ?? playlists.count
        } catch is CancellationError {} catch { failure = error.localizedDescription }
    }
    private func perform(_ request: () async throws -> Void) async {
        busy = true; failure = nil; defer { busy = false }
        do { try await request(); dismiss() }
        catch is CancellationError {} catch { failure = error.localizedDescription }
    }
}

struct MusicPlaylistSettings: View {
    let playlist: MediaItem
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var busy = false
    @State private var failure: String?
    @State private var deleting = false
    var body: some View {
        NavigationView {
            Form {
                Section("Playlist name") {
                    TextField("Name", text: $name)
                    Button("Save name") { Task { await perform { try await store.renamePlaylist(playlist, name: name.trimmingCharacters(in: .whitespacesAndNewlines)) } } }
                        .disabled(busy || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                Section { Button("Delete playlist", role: .destructive) { deleting = true }.disabled(busy) }
                if busy { ProgressView() }
                if let failure { Text(failure).foregroundStyle(.secondary) }
            }.navigationTitle("Playlist settings")
                .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { dismiss() } } }
                .onAppear { name = playlist.title }
                .confirmationDialog("Delete this playlist?", isPresented: $deleting, titleVisibility: .visible) {
                    Button("Delete playlist", role: .destructive) { Task { await perform { try await store.deletePlaylist(playlist) } } }
                    Button("Cancel", role: .cancel) {}
                }
        }.navigationViewStyle(.stack)
    }
    private func perform(_ request: () async throws -> Void) async {
        busy = true; failure = nil; defer { busy = false }
        do { try await request(); dismiss() }
        catch is CancellationError {} catch { failure = error.localizedDescription }
    }
}
