import Foundation

struct TrailersTitle: Decodable, Identifiable {
    let Id: String
    let Name: String?
    let Overview: String?
    let ProductionYear: Int?
    let ImageTags: [String: String]?
    let BackdropImageTags: [String]?
    let RemoteTrailers: [TrailersRemote]?
    var id: String { Id }

    var media: MediaItem {
        MediaItem(Id: Id, Name: Name, Overview: Overview, ProductionYear: ProductionYear,
                  ImageTags: ImageTags, BackdropImageTags: BackdropImageTags)
    }
}
struct TrailersRemote: Decodable { let Url: String?; let Name: String? }
struct TrailersPage: Decodable { let Items: [TrailersTitle]; let TotalRecordCount: Int? }
struct TrailersDetail: Decodable {
    let item: TrailersTitle
    let localTrailers: [MediaItem]
    func local(id: String, ownerID: String) -> MediaItem? {
        guard MediaItem.matchesID(item.Id, ownerID),
              !MediaItem.matchesID(id, ownerID) else { return nil }
        let matches = localTrailers.filter { MediaItem.matchesID($0.Id, id) }
        return matches.count == 1 ? matches[0] : nil
    }
    var providers: [TrailersProvider] {
        var seen = Set<URL>()
        return (item.RemoteTrailers ?? []).compactMap { remote in
            guard let raw = remote.Url, let value = TrailersProvider.validated(raw),
                  seen.insert(value.url).inserted else { return nil }
            return value
        }
    }
}

/// Only exact public video/detail URLs. Never interpreted as playable media URLs.
struct TrailersProvider: Identifiable {
    let name: String
    let url: URL
    var id: String { url.absoluteString }
    static func validated(_ raw: String) -> Self? {
        guard raw.count <= 2048, !raw.contains("\\"), !raw.contains("%"),
              !raw.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains($0) }),
              let p = URLComponents(string: raw), p.scheme == "https", p.user == nil,
              p.password == nil, p.port == nil, p.fragment == nil, let host = p.host,
              let url = p.url else { return nil }
        let name: String
        switch host {
        case "www.youtube.com", "youtube.com":
            guard p.path == "/watch", let query = p.queryItems, query.count == 1,
                  query[0].name == "v", let value = query[0].value, videoID(value) else { return nil }
            name = "YouTube"
        case "youtu.be":
            guard p.query == nil, videoID(String(p.path.dropFirst())) else { return nil }
            name = "YouTube"
        case "vimeo.com":
            let id = String(p.path.dropFirst())
            guard p.query == nil, !id.isEmpty, id.count <= 20, id.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
            name = "Vimeo"
        case "tv.apple.com":
            let segments = p.path.split(separator: "/")
            guard p.query == nil, segments.count >= 3, segments.count <= 5,
                  segments.contains("movie") || segments.contains("show"),
                  segments.last?.hasPrefix("umc.cmc.") == true else { return nil }
            name = "Apple TV"
        default: return nil
        }
        return Self(name: name, url: url)
    }
    private static func videoID(_ value: String) -> Bool {
        value.count == 11 && value.utf8.allSatisfy {
            (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 45 || $0 == 95
        }
    }
}

enum TrailersAccount {
    // Memory-only identity includes credential rotation; never render or persist it.
    static func identity(_ session: AccountSession?) -> String {
        guard let session else { return "signed-out" }
        return [session.server, session.userId, session.token].joined(separator: "\u{0}")
    }
}
