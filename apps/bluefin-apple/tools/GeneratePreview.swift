import Foundation
import AVFoundation

/// Compile with Tests/NativePlaybackFixtures.swift, then supply the output folder.
/// Generates original media without reading a library, account, or network resource.
@main struct GeneratePreview {
    static func main() async throws {
        guard CommandLine.arguments.count == 2 else { throw NativePlaybackFixtures.Failure.invalid("Supply an output folder") }
        let root = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let video = root.appendingPathComponent("BluefinOceanStudy.mp4")
        guard !FileManager.default.fileExists(atPath: video.path) else { throw NativePlaybackFixtures.Failure.invalid("Output already exists") }
        try await NativePlaybackFixtures.makeVideo(video, width: 640, height: 360, seconds: 30)
        let asset = AVURLAsset(url: video)
        let duration = try await asset.load(.duration)
        guard abs(duration.seconds - 30) < 0.1 else { throw NativePlaybackFixtures.Failure.invalid("Encoded duration") }
        let image = AVAssetImageGenerator(asset: asset)
        _ = try image.copyCGImage(at: CMTime(seconds: 1, preferredTimescale: 30), actualTime: nil)
        print("Original Bluefin video generated and decoded, duration \(duration.seconds)s")
    }
}
