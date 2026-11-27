import Foundation

@main enum SportsScoreChecks {
    static func main() throws {
        var count = 0
        func require(_ condition: @autoclosure () -> Bool, _ name: String) throws {
            guard condition() else { throw Failure.check(name) }
            count += 1
        }
        let hash = String(repeating: "a", count: 64)
        try require(SportsLogoResource.hash("/sports/api/v1/logos/" + hash) == hash, "Canonical logo")
        for value in [nil, "", "https://a.espncdn.com/logo.png", "//evil/sports/api/v1/logos/" + hash,
                      "/sports/api/v1/logos/" + hash + "?url=evil", "/sports/api/v1/logos//" + hash,
                      "/sports/api/v1/logos/" + String(repeating: "A", count: 64), "/sports/api/v1/logos/" + hash + "/extra"] {
            try require(SportsLogoResource.hash(value) == nil, "Reject untrusted logo route")
        }
        let team: [String: Any] = ["id": "1", "name": "Synthetic test team", "score": NSNull(), "record": NSNull()]
        let event: [String: Any] = ["id": "401000123", "league": "nhl", "name": "Synthetic test game",
            "date": NSNull(), "status": ["state": NSNull(), "detail": NSNull(), "period": NSNull(), "clock": NSNull()],
            "home": team, "away": team, "stats": NSNull(), "plays": NSNull()]
        let envelope: [String: Any] = ["schema": 1, "provider": "ESPN", "fetchedAt": "2026-11-27T00:00:00Z",
            "stale": true, "league": "nhl", "events": [event]]
        let board = try JSONDecoder().decode(SportsScoreboard.self, from: JSONSerialization.data(withJSONObject: envelope))
        let game = board.events[0]
        try require(board.stale, "Stale update retained")
        try require(game.home.score == nil && game.away.score == nil, "Missing scores are not zero")
        try require(game.stats == nil && game.plays == nil, "Missing statistics stay absent")
        try require(!game.isLive && !game.isScheduled && game.statusLabel == "Game", "Unknown state is not guessed")
        try require(game.dateValue == nil, "Missing date is not inferred")
        let preferences = SportsPreferences(schema: 1, sports: [], leagues: [], conferences: [],
            teams: ["espn:nhl:1"], players: [], all_conferences: true, all_teams: true, all_players: true,
            home: true, away: true, native_home: false, auto_record: false, sports_only: false, completed: true)
        try require(game.home.isFollowed(league: "nhl", preferences: preferences), "Qualified follow")
        try require(!game.home.isFollowed(league: "nfl", preferences: preferences), "Numeric team IDs do not cross leagues")
        var legacy = preferences; legacy.teams = ["1"]
        try require(game.home.isFollowed(league: "nhl", preferences: legacy), "Existing follow compatibility")
        try require(SportsScoreLeague.allCases.map(\.rawValue) == ["nhl","nfl","nba","mlb","wnba","epl","mls"], "Supported league contract")
        let instant = try unwrap(SportsScoreDate.parse("2026-11-27T01:00:00Z"))
        try require(SportsScoreDate.key(instant, timeZone: TimeZone(secondsFromGMT: 0)!) == "2026-11-27", "UTC schedule day")
        try require(SportsScoreDate.key(instant, timeZone: TimeZone(secondsFromGMT: -8 * 3600)!) == "2026-11-26", "Local schedule day")
        let standings: [String: Any] = ["schema":1, "provider":"ESPN", "fetchedAt":"2026-11-27T00:00:00Z", "stale":false,
            "league":"nhl", "groups":[["id":"g", "name":"Synthetic group", "entries":[
                ["team":team, "rank":NSNull(), "stats":["wins":"12","points":"25"]]]]]]
        let table = try JSONDecoder().decode(SportsScoreStandings.self, from: JSONSerialization.data(withJSONObject: standings))
        try require(table.groups[0].entries[0].rank == nil, "Missing rank remains absent")
        try require(table.groups[0].entries[0].stats["overtimeLosses"] == nil, "Missing standings cell remains absent")
        print("PASS \(count) native Sports contract checks")
    }
    private static func unwrap<T>(_ value: T?) throws -> T { guard let value else { throw Failure.check("Missing value") }; return value }
    enum Failure: Error { case check(String) }
}
