import Foundation

enum VideoQualityPreference: String, Codable, CaseIterable, Identifiable {
    case automatic, original, balanced, dataSaver
    var id: String { rawValue }
    var title: String {
        switch self {
        case .automatic: return "Automatic"
        case .original: return "Prefer original"
        case .balanced: return "Balanced"
        case .dataSaver: return "Data saver"
        }
    }
    var maximumBitrate: Int {
        switch self {
        case .automatic: return 80_000_000
        case .original: return 200_000_000
        case .balanced: return 12_000_000
        case .dataSaver: return 3_000_000
        }
    }
}

/// Indices belong to mediaSourceID. Nil audio is automatic; nil subtitle is off.
struct VideoOptionSelection: Codable, Equatable {
    var mediaSourceID: String?
    var audioStreamIndex: Int?
    var subtitleStreamIndex: Int?
    var quality: VideoQualityPreference = .automatic
    var plannerFields: [String: Any] {
        var fields: [String: Any] = ["quality": quality.rawValue, "subtitleStreamIndex": subtitleStreamIndex ?? -1]
        if let mediaSourceID { fields["mediaSourceId"] = mediaSourceID }
        if let audioStreamIndex { fields["audioStreamIndex"] = audioStreamIndex }
        return fields
    }
}

struct MediaItem: Codable, Identifiable, Hashable {
    let Id: String
    var Name: String?
    var `Type`: String?
    var Overview: String?
    var Genres: [String]?
    var ProductionYear: Int?
    var RunTimeTicks: Int64?
    var SeriesName: String?
    var SeriesId: String?
    var SeriesPrimaryImageTag: String?
    var ParentId: String?
    var PlaylistItemId: String?
    var NormalizationGain: Double?
    var AlbumNormalizationGain: Double?
    var Chapters: [MediaChapter]?
    var Number: String?
    var CurrentProgram: LiveProgram?
    var ChannelId: String?
    var StartDate: String?
    var EndDate: String?
    var ParentIndexNumber: Int?
    var IndexNumber: Int?
    var ImageTags: [String: String]?
    var BackdropImageTags: [String]?
    var ParentBackdropItemId: String?
    var ParentBackdropImageTags: [String]?
    var ParentPrimaryImageItemId: String?
    var ParentPrimaryImageTag: String?
    var ParentThumbItemId: String?
    var ParentThumbImageTag: String?
    var UserData: UserData?
    var IsFolder: Bool?
    var CollectionType: String?
    var ProviderIds: [String: String]?
    var ExternalUrls: [ExternalLink]?
    var Album: String?
    var AlbumId: String?
    var AlbumPrimaryImageTag: String?
    var Artists: [String]?
    var ArtistItems: [MediaNamedItem]?
    var AlbumArtist: String?
    var MediaSources: [MediaSource]?
    var MediaStreams: [MediaStream]?
    static func matchesID(_ left: String, _ right: String) -> Bool {
        if left == right { return true }
        func compactGUID(_ value: String) -> String? {
            if let id = UUID(uuidString: value) {
                return id.uuidString.replacingOccurrences(of: "-", with: "").lowercased()
            }
            guard value.utf8.count == 32, value.utf8.allSatisfy({
                (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
            }) else { return nil }
            return value.lowercased()
        }
        guard let left = compactGUID(left), let right = compactGUID(right) else { return false }
        return left == right
    }
    var id: String { Id }
    var title: String { Name ?? "Untitled" }
    var isContainer: Bool { IsFolder == true || ["Series", "Season", "MusicAlbum", "MusicArtist", "CollectionFolder", "Folder", "BoxSet", "Playlist"].contains(Type ?? "") }
    var subtitle: String {
        if let series = SeriesName { return "\(series) · S\(ParentIndexNumber ?? 1) E\(IndexNumber ?? 1)" }
        if Type == "Audio" || Type == "MusicAlbum" { return Artists?.joined(separator: ", ") ?? AlbumArtist ?? Album ?? "" }
        return [ProductionYear.map(String.init), RunTimeTicks.map { "\(max(1, $0 / 600_000_000)) min" }].compactMap { $0 }.joined(separator: " · ")
    }
    var start: Date? { StartDate.flatMap(Self.scheduleDate) }
    var end: Date? { EndDate.flatMap(Self.scheduleDate) }
    private static func scheduleDate(_ value: String) -> Date? {
        (try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(value))
            ?? (try? Date.ISO8601FormatStyle().parse(value))
    }
    var resumeTicks: Int64 {
        guard let value = UserData?.PlaybackPositionTicks, value > 0 else { return 0 }
        if let duration = RunTimeTicks, duration > 0, value >= duration { return 0 }
        return value
    }
    var progress: Double {
        guard let duration = RunTimeTicks, duration > 0 else { return 0 }
        return min(1, max(0, Double(UserData?.PlaybackPositionTicks ?? 0) / Double(duration)))
    }
}

struct MediaNamedItem: Codable, Hashable {
    var Id: String?
    var Name: String?
}

struct ExternalLink: Codable, Hashable {
    var Name: String?
    var Url: String?
}

struct MediaSource: Codable, Hashable {
    var Id: String?
    var Container: String?
    var MediaStreams: [MediaStream]?
    var LiveStreamId: String?
    var DefaultAudioStreamIndex: Int?
}

struct MediaStream: Codable, Hashable {
    var `Type`: String?
    var Codec: String?
    var BitDepth: Int?
    var SampleRate: Int?
    var Channels: Int?
    var Index: Int?
    var Language: String?
    var DisplayTitle: String?
    var Title: String?
    var IsExternal: Bool?
    var Profile: String?
    var VideoRange: String?
    var VideoRangeType: String?
    var Width: Int?
    var Height: Int?
}

struct MediaChapter: Codable, Hashable {
    var Name: String?
    var StartPositionTicks: Int64
    var ImageTag: String?
}

struct LiveProgram: Codable, Hashable, Identifiable {
    var Id: String
    var Name: String?
    var Overview: String?
    var ChannelId: String?
    var StartDate: String?
    var EndDate: String?
    var IsSports: Bool?
    var EpisodeTitle: String?
    var ImageTags: [String: String]?
    var id: String { Id }
    var artwork: MediaItem? {
        guard let primary = ImageTags?["Primary"], !primary.isEmpty else { return nil }
        return MediaItem(Id: Id, Type: "TvProgram", ImageTags: ["Primary": primary])
    }
    var title: String { Name ?? "Live programme" }
    var start: Date? { StartDate.flatMap(Self.date) }
    var end: Date? { EndDate.flatMap(Self.date) }
    private static func date(_ value: String) -> Date? {
        (try? fractionalTimestamp.parse(value)) ?? (try? wholeTimestamp.parse(value))
    }
    private static let fractionalTimestamp = Date.ISO8601FormatStyle(includingFractionalSeconds: true)
    private static let wholeTimestamp = Date.ISO8601FormatStyle()
}

struct UserData: Codable, Hashable {
    var PlaybackPositionTicks: Int64?
    var Played: Bool?
    var IsFavorite: Bool?
    var Likes: Bool?
}

struct ItemPage: Decodable {
    var Items: [MediaItem]
    var TotalRecordCount: Int?
}

struct HomeRecommendation: Decodable, Identifiable {
    var id: String
    var title: String
    var items: [MediaItem]
    var reason: String?
}

struct HomeRecommendationsPage: Decodable {
    var recommendations: [HomeRecommendation]
    var searchHistorySupported: Bool
    var searchHistoryGeneration: Int64?
}

/// A bounded presentation of the owner's recommendations. Normalizes UUID
/// spellings and removes repeats from progress shelves without inventing scores.
struct HomePage: Decodable {
    var resume: [MediaItem]
    var latest: [MediaItem]
    var nextUp: [MediaItem]
    var recommendations: [HomeRecommendation] = []
    var featured: MediaItem? { resume.first ?? recommendations.first?.items.first ?? nextUp.first ?? latest.first }

    init(resume: [MediaItem], latest: [MediaItem], nextUp: [MediaItem], recommendations: [HomeRecommendation] = []) {
        self.resume = resume; self.latest = latest; self.nextUp = nextUp
        self.recommendations = recommendations
    }
    private enum CodingKeys: String, CodingKey { case resume, latest, nextUp, recommendations }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        resume = try c.decode([MediaItem].self, forKey: .resume)
        latest = try c.decode([MediaItem].self, forKey: .latest)
        nextUp = try c.decode([MediaItem].self, forKey: .nextUp)
        recommendations = try c.decodeIfPresent([HomeRecommendation].self, forKey: .recommendations) ?? []
        boundRecommendations()
    }
    mutating func boundRecommendations() {
        var seen = resume + nextUp
        var shelfIDs = Set<String>()
        recommendations = recommendations.prefix(4).compactMap { shelf in
            guard shelfIDs.insert(shelf.id).inserted else { return nil }
            var shelf = shelf
            var items: [MediaItem] = []
            for item in shelf.items {
                guard !item.Id.isEmpty, !seen.contains(where: { MediaItem.matchesID($0.Id, item.Id) }) else { continue }
                seen.append(item); items.append(item)
                if items.count == 12 { break }
            }
            shelf.items = items
            return shelf.items.isEmpty ? nil : shelf
        }
    }
}

struct HomeHistoryState: Decodable { var generation: Int64 }


struct PublicInfo: Decodable {
    var ServerName: String?
    var Version: String?
    var Id: String?
}

struct LoginResult: Decodable {
    struct User: Decodable { var Id: String; var Name: String? }
    var User: User
    var AccessToken: String
    var ServerId: String?
}

struct QuickConnectChallenge: Decodable {
    var Secret: String
    var Code: String
    var Authenticated: Bool?
}

struct AccountSession: Codable {
    var server: String
    var token: String
    var userId: String
    var username: String
    var deviceId: String
    var allowHttp: Bool
    var irohPeerId: String?
    var irohPeerAddress: String?
    /// A reference only. The mesh device key is stored in a separate device-only vault.
    var meshSessionId: String? = nil
    var companionGeneration: UUID? = UUID()
}

struct MediaSkipSegment: Decodable, Equatable {
    enum Kind: String {
        case commercial = "Commercial", preview = "Preview", recap = "Recap", outro = "Outro", intro = "Intro"

        var title: String {
            switch self {
            case .commercial: "Skip commercial"
            case .preview: "Skip preview"
            case .recap: "Skip recap"
            case .outro: "Skip credits"
            case .intro: "Skip intro"
            }
        }
    }

    let kind: Kind?
    let itemID: String
    let start: Int64
    let end: Int64

    private enum CodingKeys: String, CodingKey {
        case type = "Type"
        case ItemId, StartTicks, EndTicks
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        itemID = try values.decode(String.self, forKey: .ItemId)
        start = try values.decode(Int64.self, forKey: .StartTicks)
        end = try values.decode(Int64.self, forKey: .EndTicks)
        if let name = try? values.decode(String.self, forKey: .type) {
            kind = Kind(rawValue: name)
        } else {
            kind = switch try? values.decode(Int.self, forKey: .type) {
            case 1: .commercial
            case 2: .preview
            case 3: .recap
            case 4: .outro
            case 5: .intro
            default: nil
            }
        }
    }

    static func admitted(_ input: [Self], for item: MediaItem) -> [Self] {
        input.lazy.filter { segment in
            segment.kind != nil && MediaItem.matchesID(segment.itemID, item.Id)
                && segment.start >= 0 && segment.end > segment.start
                && (item.RunTimeTicks.map { $0 <= 0 || segment.end <= $0 } ?? true)
        }.prefix(256).sorted { ($0.end, $0.start) < ($1.end, $1.start) }
    }

    func contains(_ ticks: Int64) -> Bool { start <= ticks && ticks < end }
}

struct PlaybackPlan: Decodable {
    var url: String
    var headers: [String: String]?
    var playSessionId: String
    var mediaSourceId: String
    var startTicks: Int64?
    var playMethod: String?
    var source: MediaSource?
    var proxyId: String?
    var startOffsetTicks: Int64?
    var audioStreamIndex: Int?
    var subtitleStreamIndex: Int?
    var nativeProbeRequired: Bool?
}

enum BluefinError: LocalizedError {
    case message(String)
    var errorDescription: String? {
        switch self { case .message(let message): return message }
    }
}

extension MediaItem {
    static let preview: [MediaItem] = [
        .init(Id: "preview-1", Name: "Ocean study", Type: "Movie", Overview: "An original thirty-second Bluefin motion study. Generated locally without personal or third-party media.", RunTimeTicks: 300_000_000)
    ]
    static let previewAlbum = MediaItem(Id: "preview-album", Name: "Bluefin audio check", Type: "MusicAlbum", AlbumArtist: "Bluefin", MediaSources: nil)
    static let previewArtist = MediaItem(Id: "preview-artist", Name: "Bluefin", Type: "MusicArtist")
    static let previewAudio = MediaItem(Id: "preview-audio", Name: "Original audio check", Type: "Audio", Overview: "A quiet four-second original test tone for trying the native music player and equalizer.", RunTimeTicks: 40_000_000, ParentIndexNumber: 1, IndexNumber: 1, Album: "Bluefin audio check", Artists: ["Bluefin"], AlbumArtist: "Bluefin", MediaStreams: [.init(Type: "Audio", Codec: "alac", BitDepth: 16, SampleRate: 44_100, Channels: 1)])
    var previewURL: URL? {
        switch Id {
        case "preview-audio": return Bundle.main.url(forResource: "BluefinAudioCheck", withExtension: "m4a")
        case "preview-1": return Bundle.main.url(forResource: "BluefinOceanStudy", withExtension: "mp4")
        default: return nil
        }
    }
}

/// The compatibility API uses 100 ns ticks and UTF-16 cue ranges.
struct BluefinLyrics: Decodable {
    struct Metadata: Decodable { var IsSynced: Bool?; var Offset: Int64?; var Length: Int64? }
    struct Cue: Decodable {
        var Start: Int64?
        var End: Int64?
        var Position: Int?
        var EndPosition: Int?
    }
    struct Line: Decodable {
        var Text: String
        var Start: Int64?
        var Cues: [Cue]?

        var displayRuns: [LyricDisplayRun] {
            var runs: [LyricDisplayRun] = []
            var cursor = Text.startIndex
            var previousPosition = 0
            var previousEnd: Int64 = 0
            let length = Text.utf16.count
            let boundaries = Set(Text.indices.map { $0.utf16Offset(in: Text) } + [length])
            for cue in Cues ?? [] {
                guard let position = cue.Position, let endPosition = cue.EndPosition,
                      position >= previousPosition, endPosition > position, endPosition <= length,
                      boundaries.contains(position), boundaries.contains(endPosition),
                      let range = Range(NSRange(location: position, length: endPosition - position), in: Text),
                      let start = cue.Start, let end = cue.End, start >= previousEnd, end > start else { continue }
                if cursor < range.lowerBound { runs.append(.init(text: String(Text[cursor..<range.lowerBound]))) }
                runs.append(.init(text: String(Text[range]), start: Double(start) / 10_000_000,
                                  end: Double(end) / 10_000_000))
                cursor = range.upperBound; previousPosition = endPosition; previousEnd = end
            }
            if cursor < Text.endIndex { runs.append(.init(text: String(Text[cursor...]))) }
            return runs
        }
    }
    var Metadata: Metadata
    var Lyrics: [Line]

    /// Older servers omit IsSynced while supplying real line timestamps.
    var isSynced: Bool {
        self.Metadata.IsSynced ?? Lyrics.contains { $0.Start.map { $0 >= 0 } == true }
    }
    var offsetSeconds: Double { Double(self.Metadata.Offset ?? 0) / 10_000_000 }
    func activeIndex(at seconds: Double) -> Int? {
        guard isSynced, seconds.isFinite, seconds >= 0 else { return nil }
        return Lyrics.lastIndex { line in
            line.Start.map { $0 >= 0 && seconds >= Double($0) / 10_000_000 + offsetSeconds } == true
        }
    }
    func seekSeconds(for index: Int) -> Double? {
        guard Lyrics.indices.contains(index), isSynced,
              let start = Lyrics[index].Start, start >= 0 else { return nil }
        return max(0, Double(start) / 10_000_000 + offsetSeconds)
    }
}

/// Untimed text remains readable; partial word fills require a valid supplied interval.
struct LyricDisplayRun {
    var text: String
    var start: Double? = nil
    var end: Double? = nil
    func progress(at seconds: Double) -> Double {
        guard let start, let end, seconds.isFinite, start.isFinite, end.isFinite, end > start else { return 0 }
        return min(1, max(0, (seconds - start) / (end - start)))
    }
}
