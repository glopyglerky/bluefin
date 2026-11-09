import AVFoundation

enum NativeAssetAdmission {
    /// Expanded format observations are not enough: ask the native decoder about
    /// this authorized asset before replacing the current player. No remote token
    /// or provider URL is accepted here.
    static func validate(_ url: URL) async throws {
        try Task.checkCancellation()
        guard url.isFileURL || (url.scheme == "http" && url.host == "127.0.0.1" && url.user == nil && url.password == nil) else {
            throw BluefinError.message("The native asset probe requires an authorized local source.")
        }
        let asset = AVURLAsset(url: url)
        defer { asset.cancelLoading() }
        try await withTaskCancellationHandler {
            try await withThrowingTaskGroup(of: Void.self) { group in
                group.addTask {
                    guard try await asset.load(.isPlayable) else { throw BluefinError.message("This source is not playable by the native player.") }
                    let tracks = try await asset.loadTracks(withMediaType: .video)
                    guard !tracks.isEmpty else { throw BluefinError.message("The source has no native video track.") }
                    for track in tracks {
                        guard try await track.load(.isDecodable) else { throw BluefinError.message("The native decoder does not support this source.") }
                    }
                }
                group.addTask {
                    try await Task.sleep(nanoseconds: 8_000_000_000)
                    asset.cancelLoading()
                    throw BluefinError.message("Native format verification timed out.")
                }
                defer { group.cancelAll() }
                _ = try await group.next()
            }
        } onCancel: { asset.cancelLoading() }
        try Task.checkCancellation()
    }
}
