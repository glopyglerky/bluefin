import Foundation
#if !SPORTS_STANDALONE
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif
#endif

/// Original fictional clubs and scores. No personal preferences or real provider assets.
enum SportsFixtures {
    static var preferences: SportsPreferences {
        SportsPreferences(schema: 1, sports: ["basketball"], leagues: ["synthetic-league"],
            conferences: [], teams: ["harbor"], players: [], all_conferences: true,
            all_teams: false, all_players: true, home: true, away: true, native_home: true,
            auto_record: false, sports_only: true, completed: false)
    }
    static func saved(_ version: UInt64 = 7) -> SportsSaved {
        SportsSaved(version: version, preferences: preferences, etag: "\"\(version)\"")
    }
    static func game(kind: String = "LIVE", state: String = "IN", date: String = "2026-11-08T00:30:00Z") -> SportsOption {
        SportsOption(kind: kind, item_id: "synthetic-\(kind)", channel_id: "authorized-channel", channel_name: "Example sports",
            event: .init(name: "Harbor at Summit", league_name: "Example League", date: date, detail: "Third quarter",
                state: state, competitors: [
                    .init(id: "harbor", name: "Harbor Herons", short_name: "Harbor", abbreviation: "HBR", side: "away", score: "72", logo: nil, record: "4–2", winner: nil),
                    .init(id: "summit", name: "Summit Owls", short_name: "Summit", abbreviation: "SMT", side: "home", score: "68", logo: nil, record: "3–3", winner: nil)
                ], broadcasters: ["Example sports"], event_url: nil), playback_verified: false)
    }
    static var feed: SportsFeed {
        SportsFeed(schema: 1, sports_only: true, games: [game(), game(kind: "UPCOMING", state: "PRE"), game(kind: "FINAL", state: "POST")],
            local_broadcasts: [game(kind: "ON_NOW", state: "IN")], presentation_only: true, providers: [])
    }
    static func session(_ user: String) -> AccountSession {
        AccountSession(server: "https://media.example.invalid", token: "synthetic-token-\(user)", userId: user,
            username: user, deviceId: "synthetic-device", allowHttp: false)
    }
}
