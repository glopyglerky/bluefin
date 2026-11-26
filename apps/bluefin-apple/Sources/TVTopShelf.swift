#if os(tvOS)
import SwiftUI
import TVServices

struct TVTopShelfSync: View {
    @EnvironmentObject private var store: LibraryStore
    @ObservedObject var sports: SportsStore
    @AppStorage(TopShelfSnapshot.preference) private var enabled = true

    private struct Request: Equatable {
        let account: UUID
        let enabled: Bool
        let titles: [String]
    }

    var body: some View {
        let requested = enabled && !store.preview && !store.homeFromCatalog
            && sports.saved?.preferences.sports_only != true
        let candidates = store.home.latest.filter {
            ["Movie", "Series", "Episode"].contains($0.Type ?? "") && !($0.BackdropImageTags ?? []).isEmpty
        }
        let request = Request(account: store.accountContext, enabled: requested,
                              titles: candidates.map { [$0.Id, $0.title, $0.BackdropImageTags?.first ?? ""].joined(separator: "|") })
        Color.clear.task(id: request) {
            if !requested {
                TopShelfSnapshot.clear()
            } else if let session = store.session, !candidates.isEmpty {
                await publish(candidates, session: session, account: request.account)
            }
        }
    }

    @MainActor private func publish(_ titles: [MediaItem], session: AccountSession, account: UUID) async {
        guard let directory = TopShelfSnapshot.directory else { return }
        let generation = UUID()
        let folder = directory.appendingPathComponent(generation.uuidString, isDirectory: true)
        var cards: [TopShelfSnapshot.Card] = []
        var seen = Set<String>()
        var published = false
        defer { if !published { try? FileManager.default.removeItem(at: folder) } }

        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            for title in titles.prefix(18) where seen.insert(title.Id).inserted {
                guard let image = await Artwork.nativeImage(for: title, session: session, backdrop: true) else { continue }
                try Task.checkCancellation()
                guard account == store.accountContext else { return }
                let jpeg = await Task.detached(priority: .utility) { image.jpegData(compressionQuality: 0.8) }.value
                try Task.checkCancellation()
                guard account == store.accountContext else { return }
                guard let jpeg, jpeg.count <= 2 * 1024 * 1024 else { continue }
                let id = UUID()
                try jpeg.write(to: folder.appendingPathComponent(id.uuidString + ".jpg"), options: .atomic)
                cards.append(.init(id: title.Id, title: String(title.title.prefix(200)), image: id))
                if cards.count == TopShelfSnapshot.maximumCards { break }
            }
            try Task.checkCancellation()
            guard account == store.accountContext, !cards.isEmpty else { return }
            let previous = TopShelfSnapshot.read(now: .distantPast)
            let snapshot = TopShelfSnapshot(generation: generation, expires: .now.addingTimeInterval(7 * 86400), cards: cards)
            try JSONEncoder().encode(snapshot).write(to: directory.appendingPathComponent("shelf.json"), options: .atomic)
            published = true
            TVTopShelfContentProvider.topShelfContentDidChange()
            if let old = previous?.folder { try? FileManager.default.removeItem(at: old) }
        } catch {
            return
        }
    }
}
#endif
