@preconcurrency import Foundation
@preconcurrency import AVFoundation
import CoreVideo
import AudioToolbox

/// Original synthetic media only. No network input or bundled media asset.
enum NativePlaybackFixtures {
    struct Files { let video: URL; let audio: URL }
    enum Failure: Error { case invalid(String) }

    static func generate(in root: URL) async throws -> Files {
        let video = root.appendingPathComponent("original-colours.mp4")
        let audio = root.appendingPathComponent("original-tone.m4a")
        try makeAudio(audio)
        try await makeVideo(video)
        return Files(video: video, audio: audio)
    }

    static func makeAudio(_ url: URL) throws {
        let rate = 44_100.0
        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatAppleLossless,
            AVSampleRateKey: rate, AVNumberOfChannelsKey: 1, AVEncoderBitDepthHintKey: 16]
        // Closing AVAudioFile finalizes the M4A before the caller reads it.
        let file = try AVAudioFile(forWriting: url, settings: settings,
                                   commonFormat: .pcmFormatFloat32, interleaved: false)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 176_400),
              let samples = buffer.floatChannelData?[0] else { throw Failure.invalid("PCM allocation") }
        buffer.frameLength = buffer.frameCapacity
        for i in 0..<Int(buffer.frameLength) {
            samples[i] = Float(sin(Double(i) * 2 * .pi * 440 / rate) * 0.125)
        }
        try file.write(from: buffer)
    }

    // All mutable writer state and AVFoundation objects are confined to the writer queue.
    private final class WriterState: @unchecked Sendable {
        let writer: AVAssetWriter
        let input: AVAssetWriterInput
        let adapter: AVAssetWriterInputPixelBufferAdaptor
        var frame = 0
        var finished = false
        var finalizing = false
        var watchdog: DispatchWorkItem?
        init(_ writer: AVAssetWriter, _ input: AVAssetWriterInput, _ adapter: AVAssetWriterInputPixelBufferAdaptor) {
            self.writer = writer; self.input = input; self.adapter = adapter
        }
    }

    static func makeVideo(_ url: URL, width: Int = 160, height: Int = 90, seconds: Int = 3) async throws {
        guard (1...640).contains(width), (1...360).contains(height), (1...30).contains(seconds) else { throw Failure.invalid("Fixture dimensions") }
        let frameCount = seconds * 30
        let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: width == 160 ? 96_000 : 400_000,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264BaselineAutoLevel]])
        let adapter = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input,
            sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height])
        guard writer.canAdd(input) else { throw Failure.invalid("H264 writer admission") }
        writer.add(input)
        guard writer.startWriting() else { throw writer.error ?? Failure.invalid("H264 writer start") }
        writer.startSession(atSourceTime: .zero)
        let queue = DispatchQueue(label: "bluefin.tests.fixture-writer")
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let state = WriterState(writer, input, adapter)
            let watchdog = DispatchWorkItem {
                guard !state.finished else { return }; state.finished = true
                state.watchdog = nil
                state.writer.cancelWriting()
                continuation.resume(throwing: Failure.invalid("H264 writer deadline exceeded"))
            }
            state.watchdog = watchdog
            queue.asyncAfter(deadline: .now() + 60, execute: watchdog)
            state.input.requestMediaDataWhenReady(on: queue) {
                guard !state.finished && !state.finalizing else { return }
                do {
                    while state.input.isReadyForMoreMediaData && state.frame < frameCount {
                        var raw: CVPixelBuffer?
                        let status = CVPixelBufferCreate(kCFAllocatorDefault, width, height,
                            kCVPixelFormatType_32BGRA, nil, &raw)
                        guard status == kCVReturnSuccess, let pixel = raw else {
                            throw Failure.invalid("CVPixelBuffer allocation \(status)")
                        }
                        CVPixelBufferLockBaseAddress(pixel, [])
                        let address = CVPixelBufferGetBaseAddress(pixel)!.assumingMemoryBound(to: UInt8.self)
                        let stride = CVPixelBufferGetBytesPerRow(pixel)
                        let phase = Double(state.frame) / 30
                        for y in 0..<height { for x in 0..<width {
                            let i = y * stride + x * 4
                            let nx = Double(x) / Double(width), ny = Double(y) / Double(height)
                            let wave = sin(nx * 7 + phase * 0.4) * 0.2 + cos(nx * 12 - phase * 0.25) * 0.08
                            let glow = pow(max(0, sin((ny + wave) * .pi)), 3)
                            address[i] = UInt8(35 + 95 * glow)
                            address[i+1] = UInt8(12 + 155 * glow)
                            address[i+2] = UInt8(4 + 18 * glow)
                            address[i+3] = 255
                        } }
                        CVPixelBufferUnlockBaseAddress(pixel, [])
                        guard state.adapter.append(pixel, withPresentationTime: CMTime(value: Int64(state.frame), timescale: 30)) else {
                            throw state.writer.error ?? Failure.invalid("H264 append frame \(state.frame)")
                        }
                        state.frame += 1
                    }
                    if state.frame == frameCount {
                        state.finalizing = true; state.input.markAsFinished()
                        state.writer.endSession(atSourceTime: CMTime(seconds: Double(seconds), preferredTimescale: 30))
                        state.writer.finishWriting {
                            queue.async {
                                guard !state.finished else { return }
                                state.finished = true; state.watchdog?.cancel(); state.watchdog = nil
                                if state.writer.status == .completed { continuation.resume() }
                                else { continuation.resume(throwing: state.writer.error ?? Failure.invalid("H264 finalization")) }
                            }
                        }
                    }
                } catch {
                    state.finished = true; state.watchdog?.cancel(); state.watchdog = nil; state.writer.cancelWriting()
                    continuation.resume(throwing: error)
                }
            }
        }
    }
}
