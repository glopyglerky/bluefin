import Foundation

struct MediaItem: Codable, Identifiable, Hashable {
    let Id: String
    var Name: String?
    var `Type`: String?
    var Overview: String?
    var ProductionYear: Int?
    var RunTimeTicks: Int64?
    var SeriesName: String?
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
}

enum BluefinError: LocalizedError {
    case message(String)
    var errorDescription: String? {
        switch self { case .message(let message): return message }
    }
}

extension MediaItem {
    static let preview: [MediaItem] = [
        .init(Id: "preview-1", Name: "Big Buck Bunny", Type: "Movie", Overview: "A gentle giant finds his quiet morning interrupted. An open movie from the Blender Foundation.", ProductionYear: 2008, RunTimeTicks: 5_960_000_000),
        .init(Id: "preview-2", Name: "Sintel", Type: "Movie", Overview: "A young traveler searches for a dragon she once befriended. An open movie from the Blender Foundation.", ProductionYear: 2010, RunTimeTicks: 8_880_000_000),
        .init(Id: "preview-3", Name: "Tears of Steel", Type: "Movie", Overview: "A group of friends tries to save the world. An open movie from the Blender Foundation.", ProductionYear: 2012, RunTimeTicks: 7_340_000_000)
    ]
}
