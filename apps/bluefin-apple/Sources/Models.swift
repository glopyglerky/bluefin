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
    var ProductionYear: Int?
    var RunTimeTicks: Int64?
    var SeriesName: String?
    var SeriesId: String?
    var ParentId: String?
    var PlaylistItemId: String?
    var NormalizationGain: Double?
    var AlbumNormalizationGain: Double?
    var Chapters: [MediaChapter]?
    var Number: String?
    var CurrentProgram: LiveProgram?
    var ParentIndexNumber: Int?
    var IndexNumber: Int?
    var ImageTags: [String: String]?
    var BackdropImageTags: [String]?
    var UserData: UserData?
    var IsFolder: Bool?
    var CollectionType: String?
    var ProviderIds: [String: String]?
    var ExternalUrls: [ExternalLink]?
    var Album: String?
    var Artists: [String]?
    var AlbumArtist: String?
    var MediaSources: [MediaSource]?
    var MediaStreams: [MediaStream]?
    var id: String { Id }
    var title: String { Name ?? "Untitled" }
    var isContainer: Bool { IsFolder == true || ["Series", "Season", "MusicAlbum", "MusicArtist", "CollectionFolder", "Folder", "BoxSet", "Playlist"].contains(Type ?? "") }
    var subtitle: String {
        if let series = SeriesName { return "\(series) · S\(ParentIndexNumber ?? 1) E\(IndexNumber ?? 1)" }
        if Type == "Audio" || Type == "MusicAlbum" { return Artists?.joined(separator: ", ") ?? AlbumArtist ?? Album ?? "" }
        return [ProductionYear.map(String.init), RunTimeTicks.map { "\(max(1, $0 / 600_000_000)) min" }].compactMap { $0 }.joined(separator: " · ")
    }
    var progress: Double {
        guard let duration = RunTimeTicks, duration > 0 else { return 0 }
        return min(1, max(0, Double(UserData?.PlaybackPositionTicks ?? 0) / Double(duration)))
    }
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
    var id: String { Id }
    var title: String { Name ?? "Live programme" }
    var start: Date? { StartDate.flatMap(Self.date) }
    var end: Date? { EndDate.flatMap(Self.date) }
    private static func date(_ value: String) -> Date? {
        let parser = ISO8601DateFormatter()
        parser.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return parser.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }
}

struct UserData: Codable, Hashable {
    var PlaybackPositionTicks: Int64?
    var Played: Bool?
    var IsFavorite: Bool?
}

struct ItemPage: Decodable {
    var Items: [MediaItem]
    var TotalRecordCount: Int?
}

struct HomePage: Decodable {
    var resume: [MediaItem]
    var latest: [MediaItem]
    var nextUp: [MediaItem]
}

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

struct BluefinLyrics: Decodable {
    struct Metadata: Decodable { var IsSynced: Bool?; var Offset: Int64? }
    struct Line: Decodable { var Text: String; var Start: Int64? }
    var Metadata: Metadata
    var Lyrics: [Line]
}
