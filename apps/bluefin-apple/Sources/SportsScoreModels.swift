import Foundation

enum SportsScoreLeague: String, CaseIterable, Identifiable, Codable {
    case nhl, nfl, nba, mlb, wnba, epl, mls
    var id: String { rawValue }
    var title: String { rawValue.uppercased() }
    var standingsColumns: [(key: String, label: String)] {
        switch self {
        case .nhl: return [("gamesPlayed", "GP"), ("wins", "W"), ("losses", "L"), ("overtimeLosses", "OTL"), ("points", "PTS")]
        case .nfl: return [("wins", "W"), ("losses", "L"), ("ties", "T"), ("winPercent", "PCT")]
        case .epl, .mls: return [("gamesPlayed", "GP"), ("wins", "W"), ("ties", "D"), ("losses", "L"), ("points", "PTS")]
        case .nba, .wnba, .mlb: return [("wins", "W"), ("losses", "L"), ("winPercent", "PCT")]
        }
    }
}

struct SportsScoreTeam: Decodable, Identifiable {
    let id: String
    let name: String
    let shortName: String?
    let abbreviation: String?
    let logo: String?
    let color: String?
    let score: String?
    let record: String?
    var displayName: String { shortName?.isEmpty == false ? shortName! : name }
    func preferenceKey(league: String) -> String { "espn:\(league):\(id)" }
    func isFollowed(league: String, preferences: SportsPreferences?) -> Bool {
        preferences?.teams.contains(preferenceKey(league: league)) == true || preferences?.teams.contains(id) == true
    }
}

struct SportsScoreEvent: Decodable, Identifiable {
    struct Status: Decodable {
        let state: String?
        let detail: String?
        let period: Int?
        let clock: String?
    }
    struct Statistic: Decodable {
        let name: String
        let label: String
        let home: String?
        let away: String?
    }
    struct Play: Decodable, Identifiable {
        let id: String
        let text: String
        let period: Int?
        let clock: String?
        let homeScore: String?
        let awayScore: String?
        let scoring: Bool
        let teamId: String?
    }
    let id: String
    let league: String
    let name: String
    let shortName: String?
    let date: String?
    let status: Status
    let home: SportsScoreTeam
    let away: SportsScoreTeam
    let venue: String?
    let broadcasts: [String]?
    let stats: [Statistic]?
    let plays: [Play]?
    var isLive: Bool { status.state == "in" }
    var isScheduled: Bool { status.state == "pre" }
    var dateValue: Date? { date.flatMap(SportsScoreDate.parse) }
    var statusLabel: String {
        if let detail = status.detail, !detail.isEmpty { return detail }
        switch status.state {
        case "in": return "Live"
        case "post": return "Final"
        case "pre": return "Scheduled"
        default: return "Game"
        }
    }
}

struct SportsScoreboard: Decodable {
    let schema: Int
    let provider: String
    let fetchedAt: String
    let stale: Bool
    let league: String
    let events: [SportsScoreEvent]
}

struct SportsScoreDetail: Decodable {
    let schema: Int
    let provider: String
    let fetchedAt: String
    let stale: Bool
    let league: String
    let event: SportsScoreEvent
}

struct SportsScoreStandings: Decodable {
    struct Entry: Decodable, Identifiable {
        let team: SportsScoreTeam
        let rank: Int?
        let stats: [String: String]
        var id: String { team.id }
    }
    struct Group: Decodable, Identifiable {
        let id: String
        let name: String
        let entries: [Entry]
    }
    let schema: Int
    let provider: String
    let fetchedAt: String
    let stale: Bool
    let league: String
    let groups: [Group]
}

enum SportsScoreDate {
    static func parse(_ value: String) -> Date? {
        (try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(value))
            ?? (try? Date.ISO8601FormatStyle().parse(value))
    }
    static func key(_ value: Date, timeZone: TimeZone = .autoupdatingCurrent) -> String {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = timeZone
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: value)
    }
}

/// Only the server's content-addressed logo route is accepted. No caller/provider URL is fetched.
enum SportsLogoResource {
    static func hash(_ raw: String?) -> String? {
        guard let raw, raw.hasPrefix("/sports/api/v1/logos/") else { return nil }
        let hash = String(raw.dropFirst("/sports/api/v1/logos/".count))
        guard hash.utf8.count == 64,
              hash.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { return nil }
        return hash
    }
}
