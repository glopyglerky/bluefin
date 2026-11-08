#if os(iOS)
import CarPlay
import AVFoundation

/// Called only from a provisioned CarPlay scene. System policy controls whether
/// video is available; app code must never infer or override the vehicle state.
@available(iOS 26.4, *)
@MainActor enum BluefinCarPlayVideo {
    @discardableResult static func configure(_ row: CPListItem, item: MediaItem, session: CPSessionConfiguration) -> Bool {
        guard session.supportsVideoPlayback, ["Movie", "Episode", "Video", "TvChannel"].contains(item.Type ?? "") else { return false }
        row.playbackConfiguration = CPPlaybackConfiguration(
            preferredPresentation: .video,
            playbackAction: .play,
            elapsedTime: CMTime(value: item.UserData?.PlaybackPositionTicks ?? 0, timescale: 10_000_000),
            duration: CMTime(value: item.RunTimeTicks ?? 0, timescale: 10_000_000))
        return true
    }
}
#endif
