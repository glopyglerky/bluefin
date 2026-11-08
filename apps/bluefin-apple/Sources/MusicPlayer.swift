import SwiftUI
import AVKit

struct MusicLibraryView: View {
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        List {
            Section {
                NavigationLink(destination: MusicCollection(title: "Songs", type: "Audio")) { Label("Songs", systemImage: "music.note") }
                NavigationLink(destination: MusicCollection(title: "Albums", type: "MusicAlbum")) { Label("Albums", systemImage: "square.stack") }
                NavigationLink(destination: MusicCollection(title: "Artists", type: "MusicArtist")) { Label("Artists", systemImage: "music.mic") }
                NavigationLink(destination: MusicCollection(title: "Playlists", type: "Playlist")) { Label("Playlists", systemImage: "music.note.list") }
            }
            Section {
                Text("Music streams from your connected library. Source format and the current output route are shown in Now Playing.").font(.footnote).foregroundStyle(.secondary)
            }
        }.navigationTitle("Music")
    }
}

struct MusicCollection: View {
    let title: String
    var type: String = "Audio"
    var parent: MediaItem?
    @EnvironmentObject private var store: LibraryStore
    @State private var items: [MediaItem] = []
    @State private var total = 0
    @State private var loading = false
    @State private var problem: String?
    var body: some View {
        List {
            if !items.isEmpty && type == "Audio" {
                HStack(spacing: 25) {
                    Button { Task { await store.playMusic(items) } } label: { Label("Play", systemImage: "play.fill") }
                    Button { Task { await store.playMusic(items, shuffled: true) } } label: { Label("Shuffle", systemImage: "shuffle") }
                }.disabled(store.startingPlayback).padding(.vertical, 10)
            }
            ForEach(Array(items.enumerated()), id: \.element.Id) { index, item in
                if item.Type == "Audio" {
                    Button { Task { await store.playMusic(items, at: index) } } label: { MusicRow(item: item) }.disabled(store.startingPlayback)
                } else {
                    NavigationLink(destination: MusicCollection(title: item.title, type: item.Type == "MusicArtist" ? "MusicAlbum" : "Audio", parent: item)) { MusicRow(item: item) }
                }
            }
            if loading { ProgressView("Loading music") }
            if let problem { Text(problem).foregroundColor(.red); Button("Try again") { Task { await load(reset: items.isEmpty) } } }
            if !loading && problem == nil && items.isEmpty { Text(store.preview ? "Connect a music library to listen to your collection." : "No music is available in this collection.").foregroundStyle(.secondary) }
            if !loading && items.count < total { Button("Load more") { Task { await load(reset: false) } } }
        }.navigationTitle(title).task { await load(reset: true) }
    }
    private func load(reset: Bool) async {
        guard !loading else { return }
        loading = true; problem = nil
        defer { loading = false }
        if store.preview { return }
        guard let session = store.session else { return }
        do {
            var fields: [String: Any] = ["includeItemTypes": type, "startIndex": reset ? 0 : items.count, "limit": 60]
            if let parent { fields["parentId"] = parent.Id }
            let page: ItemPage = try await RustBridge.call("items", session: session, fields: fields)
            try Task.checkCancellation()
            if reset { items = page.Items } else {
                let existing = Set(items.map(\.Id)); items.append(contentsOf: page.Items.filter { !existing.contains($0.Id) })
            }
            total = page.TotalRecordCount ?? items.count
        } catch is CancellationError {} catch { problem = error.localizedDescription }
    }
}

struct MusicRow: View {
    let item: MediaItem
    var body: some View {
        HStack(spacing: 14) {
            Artwork(item: item).frame(width: 56, height: 56).clipShape(RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 5) {
                Text(item.title).font(.body.weight(.medium)).lineLimit(2)
                if !item.subtitle.isEmpty { Text(item.subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(1) }
            }
            Spacer()
            if item.UserData?.IsFavorite == true { Image(systemName: "star.fill").font(.caption).foregroundColor(.cyan).accessibilityLabel("Favorite") }
        }.padding(.vertical, 4)
    }
}

struct MusicMiniPlayer: View {
    @ObservedObject var playback: PlaybackController
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
                .accessibilityLabel("Next track").disabled(store.musicIndex + 1 >= store.musicQueue.count)
        }.padding(.horizontal, 20).padding(.vertical, 10).background(.regularMaterial)
    }
}

struct MusicPlayer: View {
    @ObservedObject var playback: PlaybackController
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var queueVisible = false
    @State private var favorite = false
    @State private var favoriteBusy = false
    @State private var problem: String?
    private var duration: Double { max(1, Double(playback.item.RunTimeTicks ?? 0) / 10_000_000) }
    var body: some View {
        ScrollView {
            Group {
                #if os(tvOS)
                HStack(alignment: .center, spacing: 70) { cover.frame(maxWidth: 600); controls.frame(maxWidth: 650) }
                #else
                if sizeClass == .regular { HStack(spacing: 45) { cover; controls }.frame(maxWidth: 1050) }
                else { VStack(spacing: 30) { cover; controls }.frame(maxWidth: 520) }
                #endif
            }.padding(30).frame(maxWidth: .infinity)
        }
        .background(LinearGradient(colors: [.cyan.opacity(0.13), .clear], startPoint: .topLeading, endPoint: .bottomTrailing).ignoresSafeArea())
        .onAppear { favorite = playback.item.UserData?.IsFavorite == true }
        .sheet(isPresented: $queueVisible) { queue }
        .alert("Music", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) { Button("OK") { problem = nil } } message: { Text(problem ?? "") }
    }
    private var cover: some View {
        Artwork(item: playback.item).aspectRatio(1, contentMode: .fit).clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous)).shadow(color: .black.opacity(0.12), radius: 20, y: 12)
    }
    private var controls: some View {
        VStack(alignment: .leading, spacing: 24) {
            HStack {
                Text("NOW PLAYING").font(.caption.weight(.semibold)).tracking(2).foregroundStyle(.secondary)
                Spacer()
                Button { dismiss() } label: { Image(systemName: "chevron.down").frame(width: 44, height: 44) }.accessibilityLabel("Close Now Playing")
            }
            VStack(alignment: .leading, spacing: 8) {
                Text(playback.item.title).font(.title.bold()).fixedSize(horizontal: false, vertical: true)
                Text(playback.item.subtitle).font(.title3).foregroundStyle(.secondary)
                if let album = playback.item.Album { Text(album).font(.subheadline).foregroundStyle(.secondary) }
            }
            VStack(spacing: 8) {
                #if os(iOS)
                Slider(value: Binding(get: { min(duration, playback.positionSeconds) }, set: { playback.seek(seconds: $0) }), in: 0...duration).accessibilityLabel("Playback position")
                #else
                ProgressView(value: min(duration, playback.positionSeconds), total: duration).tint(.cyan)
                HStack {
                    Button("Back 15 seconds") { playback.seek(seconds: max(0, playback.positionSeconds - 15)) }
                    Button("Forward 15 seconds") { playback.seek(seconds: min(duration, playback.positionSeconds + 15)) }
                }
                #endif
                HStack { Text(time(playback.positionSeconds)); Spacer(); Text("−" + time(max(0, duration - playback.positionSeconds))) }.font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            HStack(spacing: 28) {
                Spacer()
                Button { Task { await store.skipMusic(-1) } } label: { Image(systemName: "backward.fill").font(.title).frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Previous track").disabled(store.musicIndex == 0)
                Button { playback.toggle() } label: { Image(systemName: playback.isPlaying ? "pause.fill" : "play.fill").font(.system(size: 42)).frame(width: 70, height: 70) }.accessibilityLabel(playback.isPlaying ? "Pause" : "Play")
                Button { Task { await store.skipMusic(1) } } label: { Image(systemName: "forward.fill").font(.title).frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Next track").disabled(store.musicIndex + 1 >= store.musicQueue.count)
                Spacer()
            }
            HStack(spacing: 28) {
                Button { Task { await saveFavorite() } } label: { Image(systemName: favorite ? "star.fill" : "star").frame(width: 44, height: 44) }.accessibilityLabel(favorite ? "Remove favorite" : "Add favorite").disabled(favoriteBusy)
                Spacer()
                #if os(iOS)
                MusicRoutePicker().frame(width: 44, height: 44).accessibilityLabel("Choose audio output")
                #endif
                Spacer()
                Button { queueVisible = true } label: { Image(systemName: "list.bullet").frame(width: 44, height: 44) }.accessibilityLabel("Up next queue")
            }
            audioInformation
            NativeIntegrationActions(item: playback.item)
            if let failure = playback.failure { Text(failure).font(.callout).foregroundColor(.red) }
            if let failure = playback.reportingFailure { Text(failure).font(.footnote).foregroundColor(.orange) }
        }
    }
    private var audioInformation: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let stream = playback.source?.MediaStreams?.first(where: { $0.Type == "Audio" }), let codec = stream.Codec {
                let lossless = ["flac", "alac", "pcm_s16le", "pcm_s24le", "pcm_s32le"].contains(codec.lowercased())
                Label(lossless ? "Lossless source" : "Source format", systemImage: "waveform").font(.caption.weight(.semibold))
                Text([codec.uppercased(), stream.BitDepth.map { "\($0)-bit" }, stream.SampleRate.map { String(format: "%.1f kHz", Double($0) / 1000) }].compactMap { $0 }.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
                Text(playback.playMethod == "DirectPlay" ? "Playing the native source" : "Server conversion is active").font(.caption).foregroundStyle(.secondary)
            }
            Text("Output: \(AVAudioSession.sharedInstance().currentRoute.outputs.map(\.portName).joined(separator: ", "))").font(.caption).foregroundStyle(.secondary)
            Text("Source quality and output quality may differ. Your selected route determines the final audio format.").font(.caption2).foregroundStyle(.secondary)
        }
    }
    private var queue: some View {
        NavigationView {
            List {
                ForEach(Array(store.musicQueue.enumerated()), id: \.element.Id) { index, item in
                    Button { Task { await store.playMusic(store.musicQueue, at: index); queueVisible = false } } label: {
                        HStack { MusicRow(item: item); if index == store.musicIndex { Image(systemName: "waveform").foregroundColor(.cyan) } }
                    }
                }
            }.navigationTitle("Up next").toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { queueVisible = false } } }
        }.navigationViewStyle(.stack)
    }
    private func saveFavorite() async {
        favoriteBusy = true
        defer { favoriteBusy = false }
        do { try await store.setFavorite(playback.item, value: !favorite); favorite.toggle() }
        catch { problem = error.localizedDescription }
    }
    private func time(_ seconds: Double) -> String {
        let value = seconds.isFinite ? Int(max(0, min(seconds, Double(Int.max / 2)))) : 0
        return "\(value / 60):\(String(format: "%02d", value % 60))"
    }
}

#if os(iOS)
struct MusicRoutePicker: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let view = AVRoutePickerView()
        view.prioritizesVideoDevices = false
        return view
    }
    func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}
#endif
