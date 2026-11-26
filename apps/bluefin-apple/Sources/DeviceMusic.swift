import SwiftUI
import AVFoundation
import UniformTypeIdentifiers

struct DeviceMusicTrack: Codable, Sendable, Identifiable {
    let id: String
    let title: String
    let format: String
    let byteLength: UInt64
    let importedAtUnixMs: UInt64
    let source: String
    var mediaItem: MediaItem { MediaItem(Id: "owned-" + id, Name: title, Type: "Audio", Album: "On this device") }
}
struct DeviceMusicQueue: Codable, Sendable {
    var items: [String]
    var currentIndex: UInt32?
    var repeatMode: String
    var shuffled: Bool
}
struct DeviceMusicCatalog: Codable, Sendable {
    let version: UInt32
    let libraryId: String
    let revision: UInt64
    let items: [DeviceMusicTrack]
    let queue: DeviceMusicQueue
    var current: DeviceMusicTrack? {
        guard let index = queue.currentIndex, Int(index) < queue.items.count else { return nil }
        return items.first { $0.id == queue.items[Int(index)] }
    }
}
private struct DeviceMusicImport: Decodable, Sendable {
    let catalog: DeviceMusicCatalog
    let itemId: String
    let imported: Bool
}
private struct DeviceMusicPlayback: Decodable, Sendable {
    let url: URL
    let item: DeviceMusicTrack
}
private struct DeviceMusicSearch: Decodable, Sendable {
    let libraryId: String
    let revision: UInt64
    let items: [DeviceMusicTrack]
}

/// Rust owns the durable catalog, hashes, queue, and local file admission.
/// Swift supplies Files access, AVFoundation suitability, and native controls.
@MainActor final class DeviceMusicLibrary: ObservableObject {
    @Published private(set) var catalog: DeviceMusicCatalog?
    @Published private(set) var matches: [DeviceMusicTrack] = []
    @Published private(set) var player: PlaybackController?
    @Published private(set) var busy = false
    @Published var problem: String?
    @Published var notice: String?
    @Published var expanded = false
    private var query = ""
    private var work: Task<Void, Never>?
    private var playbackGeneration = UUID()
    private var retirement: Task<Void, Error>?
    private var afterWork: (() -> Void)?

    private func directory() throws -> URL {
        #if os(tvOS)
        throw BluefinError.message("Apple TV storage is not a durable owner library. Import music on your phone or Mac.")
        #else
        let base = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                               appropriateFor: nil, create: true)
        var directory = base.appendingPathComponent("BluefinMusic", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
            attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        var values = URLResourceValues()
        values.isExcludedFromBackup = false
        try directory.setResourceValues(values)
        return directory.resolvingSymlinksInPath()
        #endif
    }

    private func fields(changing: Bool = false) throws -> [String: Any] {
        var fields: [String: Any] = ["libraryRoot": try directory().path]
        if changing {
            guard let catalog else { throw BluefinError.message("Open your local library first.") }
            fields["libraryId"] = catalog.libraryId
            fields["expectedRevision"] = catalog.revision
        }
        return fields
    }

    private func update(_ value: DeviceMusicCatalog) async throws {
        catalog = value
        try await search(query)
    }

    func refresh() {
        run {
            let value: DeviceMusicCatalog = try await RustBridge.call("ownedLibraryList", fields: self.fields())
            try await self.update(value)
        }
    }

    func setSearch(_ text: String) async throws {
        query = text
        guard !busy else { return }
        try await search(text)
    }

    private func search(_ text: String) async throws {
        query = text
        var request = try fields()
        request["query"] = text
        let result: DeviceMusicSearch = try await RustBridge.call("ownedLibrarySearch", fields: request)
        try Task.checkCancellation()
        guard query == text, result.libraryId == catalog?.libraryId, result.revision == catalog?.revision else { return }
        matches = result.items.sorted {
            let order = $0.title.localizedStandardCompare($1.title)
            return order == .orderedSame ? $0.id < $1.id : order == .orderedAscending
        }
    }

    func importFiles(_ urls: [URL]) {
        guard !urls.isEmpty, urls.count <= 20 else {
            problem = "Choose up to 20 audio files at a time."; return
        }
        run {
            var added = 0
            for url in urls {
                try Task.checkCancellation()
                guard url.isFileURL else { throw BluefinError.message("Choose files from the Files picker.") }
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                let sizes = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
                guard sizes.isRegularFile == true, let size = sizes.fileSize, size > 0, size <= 2 * 1024 * 1024 * 1024 else {
                    throw BluefinError.message("Choose regular audio files smaller than 2 GiB.")
                }
                let root = try self.directory()
                #if os(tvOS)
                // tvOS exposes ordinary free capacity, without the important-usage API.
                let capacity = try root.resourceValues(forKeys: [.volumeAvailableCapacityKey])
                if let free = capacity.volumeAvailableCapacity.map(Int64.init),
                   free < Int64(size) + 512 * 1024 * 1024 {
                    throw BluefinError.message("Free more storage before importing. This import needs room for the file plus 512 MiB of spare space.")
                }
                #else
                let capacity = try root.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
                if let free = capacity.volumeAvailableCapacityForImportantUsage,
                   free < Int64(size) + 512 * 1024 * 1024 {
                    throw BluefinError.message("Free more storage before importing. This import needs room for the file plus 512 MiB of spare space.")
                }
                #endif
                var request = try self.fields(changing: true)
                request["title"] = Self.title(url.deletingPathExtension().lastPathComponent)
                let result = try await DeviceMusicFileInput.importFile(url, fields: request)
                self.catalog = result.catalog
                // Check the copied file, never a changing provider URL. A failed
                // native decoder check does not remove a preexisting duplicate.
                do {
                    let playable = try await self.localPlayback(result.itemId)
                    _ = try await Self.audioDuration(playable.url)
                } catch {
                    if result.imported {
                        var remove = try self.fields(changing: true)
                        remove["itemId"] = result.itemId
                        let restored: DeviceMusicCatalog = try await RustBridge.call("ownedLibraryRemove", fields: remove)
                        self.catalog = restored
                    }
                    throw error
                }
                if result.imported { added += 1 }
            }
            if let catalog = self.catalog { try await self.update(catalog) }
            self.notice = added == 1 ? "One audio file imported on this device." : "\(added) audio files imported on this device."
        }
    }

    private static func title(_ value: String) -> String {
        var title = ""
        for scalar in value.unicodeScalars {
            let next = CharacterSet.controlCharacters.contains(scalar) ? " " : String(scalar)
            if title.utf8.count + next.utf8.count > 512 { break }
            title += next
        }
        title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return title.isEmpty ? "Imported audio" : title
    }

    private func localPlayback(_ id: String) async throws -> DeviceMusicPlayback {
        var request = try fields()
        request["itemId"] = id
        return try await RustBridge.call("ownedLibraryPlay", fields: request)
    }

    private static func audioDuration(_ url: URL) async throws -> Double {
        guard url.isFileURL else { throw BluefinError.message("Local music must stay on this device.") }
        let asset = AVURLAsset(url: url)
        let playable = try await asset.load(.isPlayable)
        let protected = try await asset.load(.hasProtectedContent)
        let audio = try await asset.loadTracks(withMediaType: .audio)
        let video = try await asset.loadTracks(withMediaType: .video)
        let duration = try await asset.load(.duration).seconds
        guard playable, !protected, !audio.isEmpty, audio.count <= 32, video.isEmpty,
              duration.isFinite, duration > 0, duration <= Double(Int64.max / 10_000_000) else {
            throw BluefinError.message("Choose unprotected audio that this Apple device can play.")
        }
        return duration
    }

    func play(_ id: String, store: LibraryStore) {
        run {
            try await store.withDeviceMusicPlayback {
                let generation = self.playbackGeneration
                var request = try self.fields(changing: true)
                request["itemId"] = id
                let value: DeviceMusicCatalog = try await RustBridge.call("ownedLibrarySelect", fields: request)
                try await self.update(value)
                try await self.startCurrent(store: store, generation: generation)
            }
        }
    }

    func playAll(store: LibraryStore) {
        run {
            guard let catalog = self.catalog else { return }
            let items = self.matches.prefix(512).map(\.id)
            guard !items.isEmpty else { return }
            try await store.withDeviceMusicPlayback {
                let generation = self.playbackGeneration
                var request = try self.fields(changing: true)
                request["queue"] = ["items": items, "currentIndex": 0,
                    "repeatMode": catalog.queue.repeatMode, "shuffled": false]
                let value: DeviceMusicCatalog = try await RustBridge.call("ownedLibraryQueue", fields: request)
                try await self.update(value)
                try await self.startCurrent(store: store, generation: generation)
            }
        }
    }

    func advance(forward: Bool, automatic: Bool = false, store: LibraryStore) {
        if busy && automatic {
            afterWork = { [weak self, weak store] in
                guard let self, let store else { return }
                self.advance(forward: forward, automatic: true, store: store)
            }
            return
        }
        run {
            try await store.withDeviceMusicPlayback {
                let generation = self.playbackGeneration
                var request = try self.fields(changing: true)
                request["forward"] = forward
                request["automatic"] = automatic
                let value: DeviceMusicCatalog = try await RustBridge.call("ownedLibraryAdvance", fields: request)
                try await self.update(value)
                if value.current == nil { _ = self.stopPlayback(); return }
                try await self.startCurrent(store: store, generation: generation)
            }
        }
    }

    func cycleRepeat() {
        run {
            guard let catalog = self.catalog else { return }
            let next = catalog.queue.repeatMode == "off" ? "all" : catalog.queue.repeatMode == "all" ? "one" : "off"
            var queue = catalog.queue
            queue.repeatMode = next
            var request = try self.fields(changing: true)
            request["queue"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(queue))
            let value: DeviceMusicCatalog = try await RustBridge.call("ownedLibraryQueue", fields: request)
            try await self.update(value)
        }
    }

    private func startCurrent(store: LibraryStore, generation: UUID) async throws {
        guard let track = catalog?.current else { return }
        let source = try await localPlayback(track.id)
        let duration = try await Self.audioDuration(source.url)
        try Task.checkCancellation()
        guard playbackGeneration == generation else { throw CancellationError() }
        if let player { retirement = player.stop(); self.player = nil }
        try await retirement?.value
        guard playbackGeneration == generation else { throw CancellationError() }
        var item = source.item.mediaItem
        item.RunTimeTicks = Int64(duration * 10_000_000)
        let controller = try PlaybackController(item: item, url: source.url, plan: nil, session: nil)
        controller.configureAudio(store.audio.settings)
        controller.onNext = { [weak self, weak store] in
            guard let self, let store else { return }; self.advance(forward: true, store: store)
        }
        controller.onPrevious = { [weak self, weak store] in
            guard let self, let store else { return }; self.advance(forward: false, store: store)
        }
        controller.onFinish = { [weak self, weak store] in
            guard let self, let store else { return }; self.advance(forward: true, automatic: true, store: store)
        }
        controller.onReplay = { [weak self, weak store] in
            guard let self, let store else { return }; self.play(track.id, store: store)
        }
        player = controller
    }

    /// Stops playback only. Imports and owned files do not depend on a server
    /// account, and signing out never deletes the user's local music.
    @discardableResult func stopPlayback() -> Task<Void, Error>? {
        playbackGeneration = UUID()
        afterWork = nil
        if let player { retirement = player.stop(); self.player = nil }
        return retirement
    }

    func remove(_ track: DeviceMusicTrack) {
        run {
            if self.player?.item.Id == track.mediaItem.Id { try await self.stopPlayback()?.value }
            var request = try self.fields(changing: true)
            request["itemId"] = track.id
            let value: DeviceMusicCatalog = try await RustBridge.call("ownedLibraryRemove", fields: request)
            try await self.update(value)
        }
    }

    private func run(_ operation: @escaping @MainActor () async throws -> Void) {
        guard !busy else { return }
        busy = true; problem = nil; notice = nil
        work = Task {
            defer {
                busy = false; work = nil
                let next = afterWork; afterWork = nil; next?()
            }
            do { try await operation() }
            catch is CancellationError {}
            catch {
                problem = error.localizedDescription
                // A completed import can outlive a lost UI response. Re-read the
                // durable owner instead of inferring failure from an await.
                if let recovered: DeviceMusicCatalog = try? await RustBridge.call("ownedLibraryList", fields: fields()) {
                    catalog = recovered
                    try? await search(query)
                }
            }
        }
    }
}

/// The synchronous FFI copy runs within NSFileCoordinator's read accessor. The
/// picker security scope stays open until the physical call has returned.
private enum DeviceMusicFileInput {
    static func importFile(_ url: URL, fields: [String: Any]) async throws -> DeviceMusicImport {
        let permit = try await CoreAdmission.shared.acquire()
        defer { Task { await CoreAdmission.shared.release(permit) } }
        try Task.checkCancellation()
        let base = try JSONSerialization.data(withJSONObject: fields)
        let result = try await Task.detached(priority: .userInitiated) {
            var result: Result<DeviceMusicImport, Error>?
            var coordinationError: NSError?
            NSFileCoordinator().coordinate(readingItemAt: url, options: [], error: &coordinationError) { coordinated in
                result = Result {
                    var request = try JSONSerialization.jsonObject(with: base) as? [String: Any] ?? [:]
                    let source = coordinated.deletingLastPathComponent().resolvingSymlinksInPath()
                        .appendingPathComponent(coordinated.lastPathComponent)
                    request["sourcePath"] = source.path
                    request["operation"] = "ownedLibraryImport"
                    let bytes = try JSONSerialization.data(withJSONObject: request)
                    guard bytes.count <= 65536, let json = String(data: bytes, encoding: .utf8) else {
                        throw BluefinError.message("The file import request is too large.")
                    }
                    return try json.withCString { input in
                        guard let output = bluefin_call(input) else { throw BluefinError.message("The music engine did not respond.") }
                        defer { bluefin_free(output) }
                        let bytes = Data(String(cString: output).utf8)
                        guard let envelope = try JSONSerialization.jsonObject(with: bytes) as? [String: Any],
                              envelope["ok"] as? Bool == true, let value = envelope["value"] else {
                            let envelope = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]
                            throw BluefinError.message(envelope?["error"] as? String ?? "The audio file could not be imported.")
                        }
                        return try JSONDecoder().decode(DeviceMusicImport.self,
                            from: JSONSerialization.data(withJSONObject: value))
                    }
                }
            }
            if let coordinationError { throw coordinationError }
            guard let result else { throw BluefinError.message("The selected file is unavailable. Download it in Files and try again.") }
            return try result.get()
        }.value
        // Return committed receipts before caller cancellation can discard them.
        return result
    }
}

#if !os(tvOS)
struct DeviceMusicView: View {
    @EnvironmentObject private var store: LibraryStore
    var body: some View { DeviceMusicContent(model: store.deviceMusic).environmentObject(store) }
}
private struct DeviceMusicContent: View {
    @ObservedObject var model: DeviceMusicLibrary
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var importing = false
    @State private var query = ""
    @State private var removing: DeviceMusicTrack?
    var body: some View {
        BluefinNavigation {
            List {
                Section {
                    Text("Import audio files you are allowed to use. Your music stays in this device's library and works without a server account.")
                        .font(.callout).foregroundStyle(.secondary)
                    Button { importing = true } label: { Label("Import from Files", systemImage: "square.and.arrow.down") }
                        .disabled(model.busy).accessibilityIdentifier("ownedMusicImport")
                    if let catalog = model.catalog {
                        Text("\(catalog.items.count) songs · \(ByteCountFormatter.string(fromByteCount: Int64(catalog.items.reduce(UInt64(0)) { $0 + $1.byteLength }), countStyle: .file)) on this device")
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                    if model.matches.count > 1 {
                        Button(model.matches.count > 512 ? "Play first 512 songs" : "Play songs") { model.playAll(store: store) }
                            .disabled(model.busy)
                    }
                }
                if model.busy { ProgressView("Updating your local library") }
                if let problem = model.problem { Text(problem).foregroundStyle(.secondary) }
                if let notice = model.notice { Text(notice).foregroundStyle(.secondary) }
                Section("Songs on this device") {
                    ForEach(model.matches) { track in
                        HStack {
                            Button { model.play(track.id, store: store) } label: { MusicRow(item: track.mediaItem) }
                                .buttonStyle(.plain).disabled(model.busy || store.startingPlayback)
                            Button(role: .destructive) { removing = track } label: {
                                Image(systemName: "trash").frame(minWidth: 44, minHeight: 44)
                            }.buttonStyle(.plain).disabled(model.busy).accessibilityLabel("Remove \(track.title) from this device")
                        }
                    }
                    if model.matches.isEmpty && !model.busy {
                        MusicEmptyState(title: query.isEmpty ? "Your music travels with you" : "No matching songs",
                            message: query.isEmpty ? "Import MP3, AAC, M4A, FLAC, WAV, or AIFF files that this device can play." : "Search only looks in music imported on this device.")
                    }
                }
            }
            .navigationTitle("Music on this device")
            .searchable(text: $query, prompt: "Search local songs")
            .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { dismiss() } } }
            .safeAreaInset(edge: .bottom) {
                if let player = model.player { DeviceMusicMiniPlayer(model: model, player: player).environmentObject(store) }
            }
        }
        .task { model.refresh() }
        .task(id: query) {
            do { try await Task.sleep(nanoseconds: 200_000_000); try await model.setSearch(query) }
            catch is CancellationError {}
            catch { model.problem = error.localizedDescription }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.audio], allowsMultipleSelection: true) { result in
            do { model.importFiles(try result.get()) } catch { model.problem = error.localizedDescription }
        }
        .sheet(isPresented: $model.expanded) {
            if let player = model.player { DeviceMusicPlayer(model: model, player: player).environmentObject(store) }
        }
        .alert("Remove this local copy?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button("Cancel", role: .cancel) { removing = nil }
            Button("Remove", role: .destructive) { if let track = removing { model.remove(track) }; removing = nil }
        } message: { Text("This removes the imported copy from Bluefin. The original file you selected remains where it was.") }
    }
}

struct DeviceMusicAccessory: View {
    @ObservedObject var model: DeviceMusicLibrary
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        if let player = model.player, !store.deviceMusicVisible {
            DeviceMusicMiniPlayer(model: model, player: player).environmentObject(store)
        }
    }
}
private struct DeviceMusicMiniPlayer: View {
    @ObservedObject var model: DeviceMusicLibrary
    @ObservedObject var player: PlaybackController
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        HStack(spacing: 16) {
            Button {
                if store.deviceMusicVisible { model.expanded = true }
                else { store.deviceMusicVisible = true }
            } label: {
                HStack(spacing: 14) {
                    MusicCover(item: player.item, radius: 7).frame(width: 44, height: 44)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(player.item.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                        Text("On this device").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                }
            }.buttonStyle(.plain).accessibilityLabel("Now playing, \(player.item.title)")
            Button { player.toggle() } label: {
                Image(systemName: player.isPlaying ? "pause.fill" : "play.fill").frame(width: 44, height: 44)
            }.accessibilityLabel(player.isPlaying ? "Pause" : "Play")
            Button { model.advance(forward: true, store: store) } label: {
                Image(systemName: "forward.fill").frame(width: 44, height: 44)
            }.accessibilityLabel("Next track").disabled(model.busy)
        }.padding(.horizontal, 20).padding(.vertical, 10)
            .background(.regularMaterial).overlay(alignment: .top) { Divider() }
    }
}
private struct DeviceMusicPlayer: View {
    @ObservedObject var model: DeviceMusicLibrary
    @ObservedObject var player: PlaybackController
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var typeSize
    private var duration: Double { max(1, Double(player.item.RunTimeTicks ?? 0) / 10_000_000) }
    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(spacing: 24) {
                    HStack {
                        Button { dismiss() } label: { Image(systemName: "chevron.down").frame(width: 44, height: 44) }
                            .accessibilityLabel("Close Now Playing")
                        Spacer()
                        Text("NOW PLAYING").font(.caption.weight(.semibold)).tracking(2).foregroundStyle(.secondary)
                        Spacer()
                        Button { model.stopPlayback(); dismiss() } label: { Image(systemName: "stop.fill").frame(width: 44, height: 44) }
                            .accessibilityLabel("Stop playback")
                    }
                    if geometry.size.width >= 760 && !typeSize.isAccessibilitySize {
                        HStack(spacing: 36) { cover.frame(maxWidth: 420); controls.frame(maxWidth: 520) }
                    } else {
                        cover.frame(maxWidth: 320)
                        controls.frame(maxWidth: 520)
                    }
                }.padding(.horizontal, 20).padding(.vertical, 12).frame(maxWidth: 1100).frame(maxWidth: .infinity)
            }
        }.background(BluefinBackground())
    }
    private var cover: some View { MusicCover(item: player.item, radius: 18).aspectRatio(1, contentMode: .fit) }
    private var controls: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(player.item.title).font(.title.bold())
            Text("On this device").font(.title3).foregroundStyle(.secondary)
            Slider(value: Binding(get: { min(duration, player.positionSeconds) }, set: { player.seek(seconds: $0) }), in: 0...duration)
                .accessibilityLabel("Playback position")
            HStack { Text(musicTime(player.positionSeconds)); Spacer(); Text(musicTime(duration)) }
                .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            HStack(spacing: 24) {
                Button { model.advance(forward: false, store: store) } label: { Image(systemName: "backward.fill").frame(width: 44, height: 44) }
                    .accessibilityLabel("Previous track").disabled(model.busy)
                Button { player.toggle() } label: { Image(systemName: player.isPlaying ? "pause.fill" : "play.fill").font(.largeTitle).frame(width: 60, height: 60) }
                    .accessibilityLabel(player.isPlaying ? "Pause" : "Play")
                Button { model.advance(forward: true, store: store) } label: { Image(systemName: "forward.fill").frame(width: 44, height: 44) }
                    .accessibilityLabel("Next track").disabled(model.busy)
                Spacer()
            }
            HStack {
                Button { model.cycleRepeat() } label: {
                    Label("Repeat \(model.catalog?.queue.repeatMode ?? "off")",
                          systemImage: model.catalog?.queue.repeatMode == "one" ? "repeat.1" : "repeat")
                }.disabled(model.busy)
                Spacer()
                #if os(iOS)
                MusicRoutePicker().frame(width: 44, height: 44)
                #endif
            }
            #if os(iOS)
            MusicVolumeControl().frame(height: 40)
            #endif
            if let failure = player.failure { Text(failure).font(.callout).foregroundStyle(.secondary) }
            if let problem = model.problem { Text(problem).font(.callout).foregroundStyle(.secondary) }
        }
    }
}
#endif
