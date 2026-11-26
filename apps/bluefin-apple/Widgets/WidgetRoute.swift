import Foundation

/// Navigation only. Never carries an account, server, media identifier or playback command.
enum WidgetRoute: String, CaseIterable, Codable, Sendable {
    case library, music, liveTV, search

    var url: URL { URL(string: "bluefin-widget://open/" + rawValue)! }
    var tab: Int {
        switch self { case .library: 1; case .music: 2; case .liveTV: 4; case .search: 3 }
    }
    var title: String {
        switch self { case .library: "Library"; case .music: "Music"; case .liveTV: "Live TV"; case .search: "Search" }
    }
    var symbol: String {
        switch self {
        case .library: "square.stack"
        case .music: "music.note"
        case .liveTV: "dot.radiowaves.left.and.right"
        case .search: "magnifyingglass"
        }
    }

    init?(url: URL) {
        // Exact canonical strings also reject userinfo, ports, encoding tricks, queries and fragments.
        guard let route = Self.allCases.first(where: { $0.url.absoluteString == url.absoluteString }) else { return nil }
        self = route
    }

    /// Call synchronously on the app's main actor, using live session state, immediately before navigation.
    static func admitted(_ url: URL, authenticated: Bool, preview: Bool) -> Self? {
        guard authenticated, !preview else { return nil }
        return Self(url: url)
    }
}
