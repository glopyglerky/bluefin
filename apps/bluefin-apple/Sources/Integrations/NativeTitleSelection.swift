import Foundation

/// A page can authorize title-based playback only when the entire result set is known.
enum NativeTitleSelection {
    static func index(titles: [String], containers: [Bool], total: Int?, query: String) -> Int? {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty, query.count <= 200, titles.count == containers.count,
              total == titles.count else { return nil }
        let matches = titles.indices.filter {
            !containers[$0] && titles[$0].compare(query, options: [.caseInsensitive, .diacriticInsensitive]) == .orderedSame
        }
        return matches.count == 1 ? matches.first : nil
    }
}
