import SwiftUI

struct BluefinCapabilities: Decodable {
    var protocolVersion: Int
    var musicQueueSync: Bool
    var maxQueueItems: Int
}
struct ServerQueueState: Codable, Equatable {
    var itemIds: [String]
    var cursor: Int?
    var shuffle: Bool
    var repeatMode: String
    enum CodingKeys: String, CodingKey { case itemIds, cursor, shuffle; case repeatMode = "repeat" }
    func fields() throws -> [String: Any] {
        guard let fields = try JSONSerialization.jsonObject(with: JSONEncoder().encode(self)) as? [String: Any] else {
            throw BluefinError.message("The queue could not be encoded.")
        }
        return fields
    }
}
struct ServerMusicQueue: Decodable {
    var protocolVersion: Int
    var revision: UInt64
    var queue: ServerQueueState
}

extension LibraryStore {
    var serverQueueState: ServerQueueState {
        .init(itemIds: musicQueue.map(\.Id), cursor: musicQueue.indices.contains(musicIndex) ? musicIndex : nil,
              shuffle: musicShuffled, repeatMode: musicRepeat == .one ? "one" : musicRepeat == .all ? "all" : "off")
    }
}

/// Synchronization is deliberate. A revision conflict never silently overwrites another device.
struct QueueSyncView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var capabilities: BluefinCapabilities?
    @State private var remote: ServerMusicQueue?
    @State private var busy = false
    @State private var problem: String?
    @State private var notice: String?
    var body: some View {
        List {
            Section {
                Text("Save this device's queue to your Bluefin server, or play the queue saved by another device.")
                Text("Your account must be signed in on each device. Playback permissions still apply.").font(.footnote).foregroundStyle(.secondary)
            }
            if let remote, capabilities?.musicQueueSync == true {
                Section("On your server") {
                    Text("\(remote.queue.itemIds.count) tracks")
                    Button("Play server queue") { Task { await restore(remote) } }
                        .disabled(busy || remote.queue.itemIds.isEmpty || remote.queue.itemIds.count > 500)
                    if remote.queue.itemIds.count > 500 { Text("This queue exceeds this client's 500-track bound.").font(.footnote) }
                }
                Section("On this device") {
                    Text("\(store.musicQueue.count) tracks")
                    Button("Save this queue to server") { Task { await save(remote.revision) } }
                        .disabled(busy || store.musicQueue.count > (capabilities?.maxQueueItems ?? 0))
                }
            }
            if busy { ProgressView("Connecting to your server") }
            if let problem { Text(problem).foregroundStyle(.secondary) }
            if let notice { Text(notice).foregroundStyle(.secondary).accessibilityAddTraits(.updatesFrequently) }
            Button("Reload server queue") { Task { await load() } }.disabled(busy)
        }.navigationTitle("Queue sync")
        .task { await load() }
    }
    @MainActor private func load() async {
        guard !busy else { return }
        busy = true; problem = nil; notice = nil
        defer { busy = false }
        do {
            let capability: BluefinCapabilities = try await store.authenticated("bluefinCapabilities")
            guard capability.protocolVersion == 1, capability.musicQueueSync else { throw BluefinError.message("This server does not offer queue sync.") }
            let queue: ServerMusicQueue = try await store.authenticated("bluefinMusicQueue")
            capabilities = capability; remote = queue
        } catch { remote = nil; capabilities = nil; problem = error.localizedDescription.contains("404") ? "Queue sync is not available on this server yet." : error.localizedDescription }
    }
    @MainActor private func save(_ revision: UInt64) async {
        guard !busy else { return }
        busy = true; problem = nil; notice = nil
        defer { busy = false }
        do {
            let queue = store.serverQueueState
            let saved: ServerMusicQueue = try await store.authenticated("saveBluefinMusicQueue", fields: ["expectedRevision": revision, "queue": try queue.fields()])
            remote = saved; notice = "Saved \(saved.queue.itemIds.count) tracks to your server."
        } catch { problem = error.localizedDescription.contains("409") ? "Another device changed the server queue. Reload it before choosing which queue to keep." : error.localizedDescription }
    }
    @MainActor private func restore(_ saved: ServerMusicQueue) async {
        guard !busy else { return }
        busy = true; problem = nil; notice = nil
        defer { busy = false }
        let current = store.serverQueueState
        do {
            let page: ItemPage = try await store.authenticated("bluefinMusicQueueItems", fields: ["queue": try saved.queue.fields()])
            guard store.serverQueueState == current else { throw BluefinError.message("This device's queue changed. Choose Play server queue again to replace it.") }
            try await store.playServerQueue(page.Items, index: saved.queue.cursor ?? 0, shuffled: saved.queue.shuffle, repeatMode: saved.queue.repeatMode)
        } catch { problem = error.localizedDescription }
    }
}
