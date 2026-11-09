import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

final class NativeFormatTests: XCTestCase {
    func testLosslessBadgeDoesNotHideServerConversion() {
        let lossless = MediaStream(Type: "Audio", Codec: "alac", BitDepth: 24, SampleRate: 96_000)
        XCTAssertEqual(AudioQualitySummary.label(stream: lossless, method: "DirectPlay"), "Lossless")
        XCTAssertEqual(AudioQualitySummary.label(stream: lossless, method: "Transcode"), "Converted audio")
        XCTAssertEqual(AudioQualitySummary.format(lossless), "24-bit / 96 kHz")
        XCTAssertEqual(AudioQualitySummary.label(stream: nil, method: "DirectPlay"), "Audio format")
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
