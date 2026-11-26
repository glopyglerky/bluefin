import XCTest
import AVFoundation
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

final class KaraokeTests: XCTestCase {
    private var timeline: KaraokeTimeline {
        KaraokeTimeline(offsetSeconds: 0.1, lines: [
            KaraokeLine(id: "a", voice: .lead, tokens: [KaraokeToken(text: "Blue ", start: 0, end: 0.2), KaraokeToken(text: "sea", start: 0.2, end: 0.5)]),
            KaraokeLine(id: "b", voice: .partner, tokens: [KaraokeToken(text: "Come sing", start: 0.2, end: 0.7)])
        ])
    }
    func testWordsOffsetDuetAndBackwardSeek() throws {
        try timeline.validate(duration: 1)
        XCTAssertEqual(timeline.activeLines(at: 0.35).map(\.id), ["a", "b"])
        XCTAssertEqual(timeline.activeLines(at: 0.1).map(\.id), ["a"])
        XCTAssertTrue(timeline.activeLines(at: 0.9).isEmpty)
        XCTAssertEqual(timeline.activeLines(at: 0.15).map(\.id), ["a"])
        XCTAssertEqual(timeline.lines[0].tokens[0].progress(at: 0.1), 0.5)
        XCTAssertEqual(timeline.lines[0].tokens[0].progress(at: .nan), 0)
    }
    func testRejectsInventedOrMalformedTiming() {
        XCTAssertThrowsError(try KaraokeTimeline(offsetSeconds: 0, lines: []).validate(duration: 1))
        let bad = KaraokeLine(id: "a", voice: .lead, tokens: [KaraokeToken(text: "bad", start: .nan, end: 1)])
        XCTAssertThrowsError(try KaraokeTimeline(offsetSeconds: 0, lines: [bad]).validate(duration: 2))
        XCTAssertThrowsError(try KaraokeTimeline(offsetSeconds: 0, lines: [timeline.lines[0], timeline.lines[0]]).validate(duration: 1))
        XCTAssertThrowsError(try timeline.validate(duration: 0.3))
    }
    /// Original synthetic tones exercise actual AVFoundation mixing without media or a model.
    @MainActor func testRenderedLeadAttenuationPreservesAccompaniment() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("KaraokeTest-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let backing = directory.appendingPathComponent("backing.wav"), lead = directory.appendingPathComponent("lead.wav")
        try tone(backing, frequency: 440); try tone(lead, frequency: 880)
        let package = KaraokePackage(itemID: "synthetic", accountUserID: "fixture", accountServer: "https://example.invalid", accompaniment: backing, leadVocal: lead,
            rightsReference: "Original synthetic test tones", processing: .suppliedStems, processingReference: "", durationSeconds: 1, timeline: timeline)
        let audio = try await KaraokeAudio.prepare(package)
        let full = try render(audio, level: 1), muted = try render(audio, level: 0), half = try render(audio, level: 0.5)
        let backingFull = amplitude(full, frequency: 440), backingMuted = amplitude(muted, frequency: 440)
        XCTAssertGreaterThan(backingFull, 0.03)
        XCTAssertEqual(backingFull, backingMuted, accuracy: 0.0005)
        let leadFull = amplitude(full, frequency: 880)
        XCTAssertGreaterThan(leadFull, 0.03)
        XCTAssertLessThan(amplitude(muted, frequency: 880), leadFull * 0.01)
        XCTAssertEqual(amplitude(half, frequency: 880), leadFull * 0.5, accuracy: 0.0005)
        XCTAssertLessThan(full.map { abs($0) }.max() ?? 1, 1)
        let seeked = try render(audio, level: 0, start: 0.25)
        XCTAssertEqual(seeked.count, 36_000)
        XCTAssertEqual(amplitude(seeked, frequency: 440), backingFull, accuracy: 0.0005)
        XCTAssertLessThan(amplitude(seeked, frequency: 880), leadFull * 0.01)
        audio.setVocalLevel(.nan); XCTAssertEqual(audio.vocalLevel, 1)
        audio.setVocalLevel(-5); XCTAssertEqual(audio.vocalLevel, 0)
    }
    @MainActor func testRejectsUnequalStemsAndMissingRights() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("KaraokeReject-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let a = directory.appendingPathComponent("a.wav"), b = directory.appendingPathComponent("b.wav")
        try tone(a, frequency: 440); try tone(b, frequency: 880, frames: 24_000)
        let invalid = KaraokePackage(itemID: "fixture", accountUserID: "fixture", accountServer: "https://example.invalid", accompaniment: a, leadVocal: b,
            rightsReference: "Original synthetic test tones", processing: .suppliedStems, processingReference: "", durationSeconds: 1, timeline: timeline)
        do { _ = try await KaraokeAudio.prepare(invalid); XCTFail("Mismatched duration accepted") } catch {}
        let unauthorized = KaraokePackage(itemID: "fixture", accountUserID: "fixture", accountServer: "https://example.invalid", accompaniment: a, leadVocal: b,
            rightsReference: "", processing: .suppliedStems, processingReference: "", durationSeconds: 1, timeline: timeline)
        do { _ = try await KaraokeAudio.prepare(unauthorized); XCTFail("Missing rights accepted") } catch {}
    }
    @MainActor func testNativeItemKeepsTemporaryStemsAfterControllerOwnerReleasesThem() async throws {
        var lease: KaraokePreparationLease? = try KaraokePreparationLease()
        let directory = try XCTUnwrap(lease?.directory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let backing = directory.appendingPathComponent("backing.wav"), lead = directory.appendingPathComponent("lead.wav")
        try tone(backing, frequency: 440); try tone(lead, frequency: 880)
        let package = KaraokePackage(itemID: "synthetic", accountUserID: "fixture", accountServer: "https://example.invalid",
            accompaniment: backing, leadVocal: lead, rightsReference: "Original synthetic tones", processing: .suppliedStems,
            processingReference: "", durationSeconds: 1, timeline: timeline)
        var audio: KaraokeAudio? = try await KaraokeAudio.prepare(package)
        audio?.retainResources(try XCTUnwrap(lease))
        var native: AVPlayerItem? = audio?.nativeItem
        weak var observed = native
        audio = nil; lease = nil
        XCTAssertNotNil(native)
        XCTAssertTrue(FileManager.default.fileExists(atPath: backing.path))
        native = nil
        let deadline = ProcessInfo.processInfo.systemUptime + 3
        while observed != nil && ProcessInfo.processInfo.systemUptime < deadline {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertNil(observed)
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.path), "The last native consumer must release its inputs")
    }
    @MainActor func testDeclaredDurationUsesDecodedSampleTolerance() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("KaraokeDuration-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let a = directory.appendingPathComponent("a.wav"), b = directory.appendingPathComponent("b.wav")
        try tone(a, frequency: 440); try tone(b, frequency: 880)
        func package(_ duration: Double) -> KaraokePackage {
            KaraokePackage(itemID: "synthetic", accountUserID: "fixture", accountServer: "https://example.invalid",
                accompaniment: a, leadVocal: b, rightsReference: "Original synthetic tones", processing: .suppliedStems,
                processingReference: "", durationSeconds: duration, timeline: timeline)
        }
        for expected in [1 - 1 / 48_000.0, 1, 1 + 1 / 48_000.0] {
            let admitted = try await KaraokeAudio.prepare(package(expected))
            XCTAssertEqual(admitted.duration, 1)
        }
        for expected in [0, -1, .nan, .infinity, 1 - 2 / 48_000.0, 1 + 2 / 48_000.0] {
            do { _ = try await KaraokeAudio.prepare(package(expected)); XCTFail("Inconsistent duration admitted") }
            catch KaraokeError.invalidStems {} catch { XCTFail("Unexpected rejection: \(error)") }
        }
    }
    private func tone(_ url: URL, frequency: Double, frames: AVAudioFrameCount = 48_000) throws {
        let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1)!
        let file = try AVAudioFile(forWriting: url, settings: format.settings)
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames)!
        buffer.frameLength = frames
        for i in 0..<Int(frames) { buffer.floatChannelData![0][i] = Float(0.2 * sin(2 * .pi * frequency * Double(i) / 48_000)) }
        try file.write(from: buffer)
    }
    @MainActor private func render(_ audio: KaraokeAudio, level: Float, start: Double = 0) throws -> [Float] {
        let reader = try AVAssetReader(asset: audio.nativeItem.asset)
        reader.timeRange = CMTimeRange(start: CMTime(seconds: start, preferredTimescale: 48_000),
            duration: CMTime(seconds: audio.duration - start, preferredTimescale: 48_000))
        let tracks = audio.nativeItem.asset.tracks(withMediaType: .audio)
        let output = AVAssetReaderAudioMixOutput(audioTracks: tracks, audioSettings: [AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 48_000, AVNumberOfChannelsKey: 1, AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true])
        output.audioMix = audio.makeMix(level: level)
        reader.add(output); XCTAssertTrue(reader.startReading())
        var samples: [Float] = []
        while let sample = output.copyNextSampleBuffer() {
            let block = CMSampleBufferGetDataBuffer(sample)!
            var length = 0; var pointer: UnsafeMutablePointer<Int8>?
            XCTAssertEqual(CMBlockBufferGetDataPointer(block, atOffset: 0, lengthAtOffsetOut: nil, totalLengthOut: &length, dataPointerOut: &pointer), noErr)
            samples += pointer!.withMemoryRebound(to: Float.self, capacity: length / 4) { Array(UnsafeBufferPointer(start: $0, count: length / 4)) }
        }
        XCTAssertEqual(reader.status, .completed)
        return samples
    }
    private func amplitude(_ samples: [Float], frequency: Double) -> Double {
        var re = 0.0, im = 0.0
        for (i, sample) in samples.enumerated() {
            let phase = 2 * Double.pi * frequency * Double(i) / 48_000
            re += Double(sample) * cos(phase); im += Double(sample) * sin(phase)
        }
        return 2 * sqrt(re * re + im * im) / Double(max(1, samples.count))
    }
}
