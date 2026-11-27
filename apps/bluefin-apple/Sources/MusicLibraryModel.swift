import SwiftUI

enum MusicRepeatMode { case off, all, one }

enum MusicSort: String, CaseIterable, Identifiable {
    case name = "Name", added = "Recently added", year = "Year"
    var id: String { rawValue }
    var key: String {
        switch self { case .name: return "SortName"; case .added: return "DateCreated"; case .year: return "ProductionYear" }
    }
}

/// A single bounded server page. Request identity prevents late search results replacing newer ones.
@MainActor final class MusicLibraryModel: ObservableObject {
    @Published private(set) var items: [MediaItem] = []
    @Published private(set) var total = 0
    @Published private(set) var start = 0
    @Published private(set) var loading = false
    @Published private(set) var problem: String?
    private var request = UUID()
    private(set) var pageCount = 0
    var hasNext: Bool { pageCount > 0 && start + pageCount < total }
    var hasPrevious: Bool { start > 0 }

    func load(store: LibraryStore, type: String, parent: MediaItem?, start: Int = 0, query: String = "", sort: MusicSort = .name, favorites: Bool = false, fromLetter: String = "", genre: String = "") async {
        let identity = UUID()
        request = identity
        loading = true; problem = nil
        // Do not allow a prior page to play while a different filter is being loaded.
        items = []; pageCount = 0; total = 0; self.start = start
        defer { if request == identity { loading = false } }
        do {
            let page = try await store.musicItems(type: type, parent: parent, start: start, query: query, sort: parent?.Type == "MusicAlbum" ? "IndexNumber" : sort.key, descending: parent?.Type == "MusicAlbum" ? false : sort != .name, favorites: favorites, fromLetter: fromLetter, genre: genre)
            try Task.checkCancellation()
            guard identity == request else { return }
            items = page.Items
            pageCount = page.Items.count
            self.start = start
            total = max(start + page.Items.count, page.TotalRecordCount ?? start + page.Items.count)
        } catch is CancellationError {} catch {
            if identity == request { problem = error.localizedDescription }
        }
    }

    static func letter(_ item: MediaItem) -> String {
        let first = String(item.title.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: .current).prefix(1)).uppercased()
        return first.rangeOfCharacter(from: .letters) == nil ? "#" : first
    }
}

func musicTime(_ seconds: Double) -> String {
    guard seconds.isFinite else { return "0:00" }
    let value = Int(max(0, min(seconds, Double(Int.max / 2))))
    return "\(value / 60):\(String(format: "%02d", value % 60))"
}
