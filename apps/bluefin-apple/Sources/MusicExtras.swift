import SwiftUI

struct MusicLyricsView: View {
    @ObservedObject var playback: PlaybackController
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var lyrics: BluefinLyrics?
    @State private var failure: String?
    @State private var loading = false
    @State private var preparing: Task<Void, Never>?
    @State private var preparationStatus: String?
    @State private var karaokeFailure: String?
    @State private var preparationID = UUID()
    private var identity: String { "\(store.session?.server ?? "")|\(store.session?.userId ?? "")|\(playback.item.Id)" }
    private var active: Int? {
        guard let lyrics, lyrics.Metadata.IsSynced == true else { return nil }
        let ticks = playback.positionSeconds * 10_000_000 - Double(lyrics.Metadata.Offset ?? 0)
        return lyrics.Lyrics.lastIndex { $0.Start.map { Double($0) <= ticks } == true }
    }
    var body: some View {
        NavigationView {
            Group {
                if let karaoke = playback.karaoke {
                    KaraokeLyricsView(playback: playback, audio: karaoke)
                } else {
                    ScrollViewReader { reader in
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 24) {
                                Button(preparing == nil ? "Sing" : "Cancel preparation") {
                                    if let preparing { preparing.cancel() }
                                    else { startSinging() }
                                }.buttonStyle(.borderedProminent)
                                    .disabled(store.session == nil && preparing == nil)
                                if store.preview { Text("Connect a music library to use Sing.").font(.callout).foregroundStyle(.secondary) }
                                if let preparationStatus { ProgressView(preparationStatus) }
                                if let karaokeFailure { Text(karaokeFailure).foregroundStyle(.secondary) }
                                Text("Adjust the vocals and follow the lyrics as you sing.").font(.caption).foregroundStyle(.secondary)
                                if let lyrics {
                                    ForEach(Array(lyrics.Lyrics.enumerated()), id: \.offset) { index, line in
                                        Button {
                                            if let ticks = line.Start { playback.seek(seconds: max(0, (Double(ticks) + Double(lyrics.Metadata.Offset ?? 0)) / 10_000_000)) }
                                        } label: {
                                            Text(line.Text.isEmpty ? "♪" : line.Text).font(.title2.bold())
                                                .foregroundStyle(active == nil || active == index ? .primary : .secondary)
                                                .multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
                                        }.buttonStyle(.plain).disabled(line.Start == nil).id(index)
                                    }
                                }
                                if loading { ProgressView("Loading lyrics") }
                                if let failure { Text(failure).foregroundStyle(.secondary) }
                                if !loading && failure == nil && lyrics?.Lyrics.isEmpty != false { Text("Lyrics aren't available for this track.").foregroundStyle(.secondary) }
                            }.padding(28).frame(maxWidth: 700).frame(maxWidth: .infinity)
                        }.onChange(of: active) { index in
                            guard let index else { return }
                            withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.3)) { reader.scrollTo(index, anchor: .center) }
                        }
                    }
                }
            }.navigationTitle(playback.karaoke == nil ? "Lyrics" : "Sing")
                .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { dismiss() } } }
                .task(id: identity + "|karaoke=\(playback.karaoke != nil)") {
                    guard playback.karaoke == nil else { return }
                    loading = true; failure = nil; lyrics = nil
                    defer { loading = false }
                    do { lyrics = try await store.lyrics(item: playback.item) }
                    catch is CancellationError {} catch { failure = error.localizedDescription }
                }
        }.navigationViewStyle(.stack)
            .onChange(of: identity) { _ in preparing?.cancel() }
            .onDisappear { preparing?.cancel() }
    }
    @MainActor private func startSinging() {
        guard let session = store.session else { return }
        let request = UUID(); preparationID = request; karaokeFailure = nil
        preparing = Task { @MainActor in
            defer { if preparationID == request { preparing = nil; preparationStatus = nil } }
            do { try await KaraokePreparation.enable(playback: playback, session: session, currentSession: { store.session }, status: { preparationStatus = $0 }) }
            catch is CancellationError {}
            catch { karaokeFailure = error.localizedDescription.contains("404") ? "Sing isn't available for this song." : error.localizedDescription }
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
