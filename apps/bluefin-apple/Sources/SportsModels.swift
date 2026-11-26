import Foundation

struct SportsPreferences: Codable, Equatable {
    var schema: Int
    var sports: Set<String>
    var leagues: Set<String>
    var conferences: Set<String>
    var teams: Set<String>
    var players: Set<String>
    var all_conferences: Bool
    var all_teams: Bool
    var all_players: Bool
    var home: Bool
    var away: Bool
    var native_home: Bool
    var auto_record: Bool
    var sports_only: Bool
    var completed: Bool
}

struct SportsSaved: Codable { var version: UInt64; var preferences: SportsPreferences; var etag: String }

struct SportsOption: Codable, Identifiable {
    struct Competitor: Codable, Identifiable {
        var id: String; var name: String; var short_name: String?; var abbreviation: String?
        var side: String?; var score: String?; var logo: String?; var record: String?; var winner: Bool?
    }
    struct Event: Codable {
        var name: String; var league_name: String?; var date: String?; var detail: String?; var state: String?
        var competitors: [Competitor]?; var broadcasters: [String]?; var event_url: String?
    }
    var kind: String
    var item_id: String
    var channel_id: String?
    var channel_name: String?
    var event: Event?
    var title: String?
    var native_title: String?
    var native_start: String?
    var native_end: String?
    var start: String?
    var end: String?
    var provenance: String?
    var recording_status: String?
    var playback_verified: Bool?
    var id: String { item_id + ":" + kind }
    var displayTitle: String { event?.name ?? title ?? native_title ?? "Sports broadcast" }
    var stateLabel: String { status.label }
    var canOpenChannel: Bool {
        ["LIVE", "ON_NOW", "UPCOMING"].contains(kind) && channel_id?.isEmpty == false
    }

    func isAiring(on date: Date) -> Bool {
        guard canOpenChannel,
              let starts = SportsSchedule.date(native_start ?? start),
              let ends = SportsSchedule.date(native_end ?? end) else { return false }
        return starts <= date && date < ends
    }
}

struct SportsFeed: Codable {
    struct Provider: Codable, Identifiable {
        var name: String; var url: String; var kind: String; var access: String; var action: String
        var id: String { url }
    }
    var schema: Int
    var sports_only: Bool
    var games: [SportsOption]
    var local_broadcasts: [SportsOption]
    var presentation_only: Bool
    var providers: [Provider]?
}
