import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

final class NativeFormatTests: XCTestCase {
    func testServerIrohAdvertisementRequiresSupportedProtocolAndIdentity() {
        let valid = ServerIrohTransport(version: 1, endpointId: String(repeating: "01", count: 32), directAddresses: ["127.0.0.1:32100"], alpn: "bluefin/http/1", directOnly: true)
        XCTAssertTrue(valid.supported)
        var changed = valid; changed.version = 2; XCTAssertFalse(changed.supported)
        changed = valid; changed.endpointId = String(repeating: "x", count: 64); XCTAssertFalse(changed.supported)
        changed = valid; changed.directAddresses = []; XCTAssertFalse(changed.supported)
        changed = valid; changed.alpn = "other"; XCTAssertFalse(changed.supported)
        changed = valid; changed.directOnly = false; XCTAssertFalse(changed.supported)
        for hint in ["example.invalid:32100", "127.0.0.1:0", "user@127.0.0.1:32100", "127.0.0.1:32100/private", "127.0.0.1:32100?token=value"] {
            changed = valid; changed.directAddresses = [hint]; XCTAssertFalse(changed.supported, hint)
        }
        changed = valid; changed.directAddresses = ["[::1]:32100"]; XCTAssertTrue(changed.supported)
    }
    func testLosslessBadgeDoesNotHideServerConversion() {
        let lossless = MediaStream(Type: "Audio", Codec: "alac", BitDepth: 24, SampleRate: 96_000)
        XCTAssertEqual(AudioQualitySummary.label(stream: lossless, method: "DirectPlay"), "Hi-Res Lossless")
        XCTAssertEqual(AudioQualitySummary.label(stream: lossless, method: "Transcode"), "Converted audio")
        XCTAssertEqual(AudioQualitySummary.format(lossless), "24-bit / 96 kHz")
        XCTAssertEqual(AudioQualitySummary.label(stream: nil, method: "DirectPlay"), "Audio format unavailable")
        XCTAssertEqual(AudioQualitySummary.badge(stream: lossless, method: "DirectPlay", singing: true), "Sing · Processed audio")
        XCTAssertEqual(AudioQualitySummary.badge(stream: lossless, method: "Transcode", singing: false), "Converted audio")
    }
    func testQualityUsesNegotiatedAudioIndexAndDoesNotGuessUnknownTracks() {
        let lossless = MediaStream(Type: "Audio", Codec: "flac", BitDepth: 24, SampleRate: 96_000, Index: 1)
        let lossy = MediaStream(Type: "Audio", Codec: "aac", SampleRate: 48_000, Index: 2)
        let source = MediaSource(MediaStreams: [lossless, lossy], DefaultAudioStreamIndex: 1)
        XCTAssertEqual(AudioQualitySummary.selectedStream(source: source, index: 2), lossy)
        XCTAssertEqual(AudioQualitySummary.selectedStream(source: source, index: nil), lossless)
        XCTAssertNil(AudioQualitySummary.selectedStream(source: source, index: 3))
        XCTAssertNil(AudioQualitySummary.selectedStream(source: MediaSource(MediaStreams: [lossless, lossy]), index: nil))
        XCTAssertEqual(AudioQualitySummary.selectedStream(source: MediaSource(MediaStreams: [lossless]), index: nil), lossless)
        XCTAssertNil(AudioQualitySummary.selectedStream(source: MediaSource(MediaStreams: [lossy, lossy]), index: 2))
    }
    func testFormatDoesNotInventResolutionForInvalidOrLossyMetadata() {
        XCTAssertEqual(AudioQualitySummary.format(MediaStream(BitDepth: -1, SampleRate: 0)), "")
        XCTAssertEqual(AudioQualitySummary.label(stream: MediaStream(Codec: "aac", BitDepth: 24, SampleRate: 96_000), method: "DirectPlay"), "AAC")
        XCTAssertEqual(AudioQualitySummary.label(stream: MediaStream(Codec: "flac", BitDepth: 16, SampleRate: 44_100), method: "DirectPlay"), "Lossless")
    }
    func testNativeAssetProbeAcceptsOriginalVideoAndRejectsInvalidData() async throws {
        let video = try XCTUnwrap(MediaItem.preview.first?.previewURL)
        try await NativeAssetAdmission.validate(video)
        let invalid = FileManager.default.temporaryDirectory.appendingPathComponent("BluefinInvalidAsset-\(UUID().uuidString).mp4")
        defer { try? FileManager.default.removeItem(at: invalid) }
        try Data("not a media file".utf8).write(to: invalid)
        do { try await NativeAssetAdmission.validate(invalid); XCTFail("Invalid data must not pass native decoding admission") }
        catch { XCTAssertFalse(error.localizedDescription.isEmpty) }
    }
}
