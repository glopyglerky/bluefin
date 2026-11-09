import Foundation
import SwiftUI
import CryptoKit

struct OfflineRecord: Decodable, Identifiable {
    var itemId: String
    var state: String
    var bytes: UInt64
    var totalBytes: UInt64?
    var localUrl: String?
    var item: MediaItem?
    var error: String?
    var id: String { itemId }
    var progress: Double? {
        guard let totalBytes, totalBytes > 0 else { return nil }
        return min(1, Double(bytes) / Double(totalBytes))
    }
}

private struct OfflinePage: Decodable { var Items: [OfflineRecord] }

@MainActor final class OfflineLibrary: ObservableObject {
    @Published private(set) var records: [OfflineRecord] = []
    @Published private(set) var loading = false
    @Published var failure: String?
    private var account: String?
    private var monitor: Task<Void, Never>?
    private var generation = UUID()

    static func identity(_ session: AccountSession) -> String {
        SHA256.hash(data: Data((session.server + "\u{0}" + session.userId).utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private func root() throws -> URL {
        #if os(tvOS)
        let directory: FileManager.SearchPathDirectory = .cachesDirectory
        #else
        let directory: FileManager.SearchPathDirectory = .applicationSupportDirectory
        #endif
        guard let base = FileManager.default.urls(for: directory, in: .userDomainMask).first else {
            throw BluefinError.message("Local storage is unavailable.")
        }
        let folder = base.appendingPathComponent("Bluefin/Downloads", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        #if os(iOS)
        try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: folder.path)
        #endif
        var protected = folder
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try protected.setResourceValues(values)
        return folder.resolvingSymlinksInPath()
    }

    func disconnect() {
        generation = UUID()
        monitor?.cancel(); monitor = nil
        account = nil; records = []; failure = nil; loading = false
    }

    func connect(_ session: AccountSession?) async {
        let identity = session.map(Self.identity)
        if account != identity {
            generation = UUID()
            monitor?.cancel(); monitor = nil
            account = identity
            records = []; failure = nil; loading = false
        }
        guard let session else { return }
        await refresh(session)
    }

    func refresh(_ session: AccountSession) async {
        let identity = Self.identity(session)
        guard account == identity, !loading else { return }
        let current = generation
        loading = true
        defer { if generation == current { loading = false } }
        do {
            let page: OfflinePage = try await RustBridge.call("downloads", session: session, fields: ["cacheRoot": try root().path])
            try Task.checkCancellation()
            guard generation == current, account == identity else { return }
            records = page.Items
            failure = nil
        } catch is CancellationError {} catch { if generation == current { failure = error.localizedDescription } }
    }

    func download(_ item: MediaItem, session: AccountSession) async {
        guard item.Type == "Audio" else { failure = "Only music can be downloaded here."; return }
        let current = generation
        do {
            let _: OfflineRecord = try await RustBridge.call("downloadStart", session: session, fields: ["cacheRoot": try root().path, "itemId": item.Id])
            guard generation == current, account == Self.identity(session) else { return }
            await refresh(session)
            monitor?.cancel()
            monitor = Task { [weak self] in
                while !Task.isCancelled {
                    do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
                    guard let self, self.generation == current else { return }
                    await self.refresh(session)
                    if let record = self.records.first(where: { $0.itemId == item.Id }), record.state != "downloading" { return }
                }
            }
        } catch { if generation == current { failure = error.localizedDescription } }
    }

    func remove(_ record: OfflineRecord, session: AccountSession) async {
        let current = generation
        do {
            let _: EmptyValue = try await RustBridge.call("downloadRemove", session: session, fields: ["cacheRoot": try root().path, "itemId": record.itemId])
            if generation == current { await refresh(session) }
        } catch { if generation == current { failure = error.localizedDescription } }
    }

    func cancel(_ record: OfflineRecord, session: AccountSession) async {
        let current = generation
        do {
            let _: EmptyValue = try await RustBridge.call("downloadCancel", session: session, fields: ["cacheRoot": try root().path, "itemId": record.itemId])
            if generation == current { await refresh(session) }
        } catch { if generation == current { failure = error.localizedDescription } }
    }

    func localURL(for item: MediaItem, session: AccountSession) -> URL? {
        guard account == Self.identity(session),
              let record = records.first(where: { $0.itemId == item.Id && $0.state == "completed" }),
              let value = record.localUrl, let url = URL(string: value), url.isFileURL,
              let base = try? root().resolvingSymlinksInPath(),
              url.resolvingSymlinksInPath().path.hasPrefix(base.path + "/") else { return nil }
        return url
    }
}

struct DownloadsView: View {
    @EnvironmentObject private var store: LibraryStore
    @ObservedObject var offline: OfflineLibrary
    var body: some View {
        Group {
            if offline.records.isEmpty && !offline.loading {
                EmptyLibrary(title: "Your library, wherever you go", message: "Download songs from an album or track menu to listen without a connection.", symbol: "arrow.down.circle")
            } else {
                List {
                    Section {
                        ForEach(offline.records) { record in
                            HStack(spacing: 14) {
                                if let item = record.item {
                                    Button { Task { await store.playMusic([item]) } } label: { MusicRow(item: item) }
                                        .buttonStyle(.plain).disabled(record.state != "completed")
                                } else { Label("Music download", systemImage: "music.note") }
                                Spacer(minLength: 8)
                                VStack(alignment: .trailing, spacing: 5) {
                                    if record.state == "downloading" {
                                        if let progress = record.progress { ProgressView(value: progress).frame(width: 64) }
                                        else { ProgressView() }
                                        Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: record.bytes), countStyle: .file)).font(.caption).foregroundStyle(.secondary)
                                    } else {
                                        Image(systemName: record.state == "completed" ? "checkmark.circle.fill" : "exclamationmark.circle").foregroundColor(record.state == "completed" ? .accentColor : .secondary)
                                    }
                                }
                                #if os(tvOS)
                                downloadAction(record)
                                #else
                                Menu {
                                    downloadAction(record)
                                } label: { Image(systemName: "ellipsis").frame(minWidth: 44, minHeight: 44) }
                                    .accessibilityLabel("Download options")
                                #endif
                            }
                            if let error = record.error { Text(error).font(.footnote).foregroundStyle(.secondary) }
                        }
                    } footer: {
                        #if os(tvOS)
                        Text("Apple TV stores music in its cache. tvOS can remove cached files when it needs space.")
                        #else
                        Text("Downloads stay on this device and are excluded from backups. You can remove them at any time.")
                        #endif
                    }
                }
            }
        }.navigationTitle("Downloads")
        .task(id: store.session.map(OfflineLibrary.identity)) { await offline.connect(store.session) }
        .bluefinRefresh { if let session = store.session { await offline.refresh(session) } }
        .alert("Downloads", isPresented: Binding(get: { offline.failure != nil }, set: { if !$0 { offline.failure = nil } })) {
            Button("OK") { offline.failure = nil }
        } message: { Text(offline.failure ?? "") }
    }
    private func downloadAction(_ record: OfflineRecord) -> some View {
        Button(record.state == "downloading" ? "Cancel download" : "Remove download", role: .destructive) {
            guard let session = store.session else { return }
            Task {
                if record.state == "downloading" { await offline.cancel(record, session: session) }
                else { await offline.remove(record, session: session) }
            }
        }
    }
}
