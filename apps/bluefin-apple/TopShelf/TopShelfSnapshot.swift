#if os(tvOS)
import Foundation
import TVServices

struct TopShelfSnapshot: Codable {
    struct Card: Codable {
        let id: String
        let title: String
        let image: UUID
    }

    let generation: UUID
    let expires: Date
    let cards: [Card]

    static let group = "group.org.glopyglerky.bluefin.topshelf"
    static let preference = "bluefin.showLibraryInTopShelf"
    static let maximumCards = 6

    static var directory: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?
            .appendingPathComponent("Library/Caches/TopShelf", isDirectory: true)
    }

    var folder: URL? { Self.directory?.appendingPathComponent(generation.uuidString, isDirectory: true) }

    static func read(now: Date = .now) -> Self? {
        guard let file = directory?.appendingPathComponent("shelf.json"),
              let length = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize, length <= 16 * 1024,
              let data = try? Data(contentsOf: file),
              let result = try? JSONDecoder().decode(Self.self, from: data),
              result.expires > now, result.cards.count <= maximumCards else { return nil }
        return result
    }

    func launchURL(for card: Card) -> URL? {
        URL(string: "bluefin-shelf://item/\(generation.uuidString)/\(card.image.uuidString)")
    }

    static func itemID(for url: URL) -> String? {
        guard url.scheme == "bluefin-shelf", url.host == "item", url.user == nil,
              url.password == nil, url.port == nil, url.query == nil, url.fragment == nil,
              let snapshot = read() else { return nil }
        let path = url.pathComponents
        guard path.count == 3, UUID(uuidString: path[1]) == snapshot.generation,
              let image = UUID(uuidString: path[2]) else { return nil }
        return snapshot.cards.first(where: { $0.image == image })?.id
    }

    static func clear() {
        guard let directory, FileManager.default.fileExists(atPath: directory.path) else { return }
        try? FileManager.default.removeItem(at: directory)
        TVTopShelfContentProvider.topShelfContentDidChange()
    }
}
#endif
