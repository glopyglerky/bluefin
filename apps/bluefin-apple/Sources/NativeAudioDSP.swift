import AVFoundation
import AudioToolbox
import MediaToolbox

struct NativeAudioSettings: Equatable {
    enum Normalization: Equatable { case off, track, album }
    var eqEnabled = false
    var gainsDB: [Double] = Array(repeating: 0, count: 6)
    var normalization: Normalization = .off
    static let frequencies: [Double] = [60, 150, 400, 1_000, 2_400, 10_000]

    func gainDB(for item: MediaItem) -> Double {
        let value: Double?
        switch normalization {
        case .off: value = nil
        case .track: value = item.NormalizationGain
        case .album: value = item.AlbumNormalizationGain ?? item.NormalizationGain
        }
        return Self.bounded(value ?? 0, limit: 24)
    }
    var bands: [Double] {
        (0..<6).map { eqEnabled && gainsDB.indices.contains($0) ? Self.bounded(gainsDB[$0], limit: 12) : 0 }
    }
    static func bounded(_ value: Double, limit: Double) -> Double { value.isFinite ? min(limit, max(-limit, value)) : 0 }
}

/// Peaking biquad, transposed direct form II. One state per band per channel.
struct NativeAudioBiquad {
    var b0 = 1.0, b1 = 0.0, b2 = 0.0, a1 = 0.0, a2 = 0.0
    var z1 = 0.0, z2 = 0.0
    init(frequency: Double, gainDB: Double, sampleRate: Double) {
        guard gainDB != 0, frequency < sampleRate * 0.49 else { return }
        let a = pow(10, gainDB / 40), w = 2 * Double.pi * frequency / sampleRate
        let alpha = sin(w) / 2 // Q = 1
        let a0 = 1 + alpha / a
        b0 = (1 + alpha * a) / a0; b1 = -2 * cos(w) / a0; b2 = (1 - alpha * a) / a0
        a1 = b1; a2 = (1 - alpha / a) / a0
    }
    mutating func sample(_ x: Double) -> Double {
        let y = b0 * x + z1
        z1 = b1 * x - a1 * y + z2; z2 = b2 * x - a2 * y
        return y
    }
}

/// Render state is exclusively owned by the tap callbacks, never the UI.
final class NativeAudioDSP {
    let bands: [Double]
    let gain: Double
    let status: (String) -> Void
    private var filters = UnsafeMutablePointer<NativeAudioBiquad>.allocate(capacity: 48)
    private var channels = 0
    private var planar = false
    private var maximumFrames = 0
    private var supported = false

    init(bands: [Double], gainDB: Double, status: @escaping (String) -> Void) {
        self.bands = bands; gain = pow(10, gainDB / 20); self.status = status
        filters.initialize(repeating: NativeAudioBiquad(frequency: 0, gainDB: 0, sampleRate: 48_000), count: 48)
    }
    deinit { filters.deinitialize(count: 48); filters.deallocate() }

    func prepare(_ format: AudioStreamBasicDescription, maximumFrames: Int) {
        channels = Int(format.mChannelsPerFrame)
        planar = format.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0
        self.maximumFrames = maximumFrames
        supported = format.mFormatID == kAudioFormatLinearPCM && format.mFormatFlags & kAudioFormatFlagIsFloat != 0
            && format.mFormatFlags & kAudioFormatFlagIsBigEndian == 0 && format.mBitsPerChannel == 32
            && (1...8).contains(channels) && format.mSampleRate.isFinite && format.mSampleRate >= 8_000
            && format.mBytesPerFrame == UInt32(4 * (planar ? 1 : channels))
        if supported {
            for channel in 0..<channels {
                for band in 0..<6 {
                    filters[channel * 6 + band] = NativeAudioBiquad(frequency: NativeAudioSettings.frequencies[band], gainDB: bands[band], sampleRate: format.mSampleRate)
                }
            }
        }
        status(supported ? "Active: six-band EQ and metadata gain" : "Unavailable: decoded PCM format is unsupported; bypassed")
    }

    /// No allocation, locks, dispatch or channel mixing in the render callback.
    func process(_ list: UnsafeMutablePointer<AudioBufferList>, frames: Int, reset: Bool) {
        guard supported, frames >= 0, frames <= maximumFrames else { return }
        let buffers = UnsafeMutableAudioBufferListPointer(list)
        guard buffers.count == (planar ? channels : 1) else { return }
        for buffer in buffers {
            guard buffer.mNumberChannels == UInt32(planar ? 1 : channels), buffer.mData != nil,
                  Int(buffer.mDataByteSize) >= frames * 4 * (planar ? 1 : channels) else { return }
        }
        if reset { for i in 0..<(channels * 6) { filters[i].z1 = 0; filters[i].z2 = 0 } }
        for channel in 0..<channels {
            let samples = buffers[planar ? channel : 0].mData!.assumingMemoryBound(to: Float.self)
            for frame in 0..<frames {
                let index = planar ? frame : frame * channels + channel
                var value = Double(samples[index])
                if !value.isFinite { value = 0 }
                for band in 0..<6 { value = filters[channel * 6 + band].sample(value) }
                // Explicit sample ceiling prevents positive metadata/EQ gain from clipping output.
                samples[index] = Float(min(1, max(-1, value * gain)))
            }
        }
    }

    static func mix(track: AVAssetTrack, bands: [Double], gainDB: Double, status: @escaping (String) -> Void) -> AVAudioMix? {
        let state = NativeAudioDSP(bands: bands, gainDB: gainDB, status: status)
        let retained = Unmanaged.passRetained(state)
        var callbacks = MTAudioProcessingTapCallbacks(version: kMTAudioProcessingTapCallbacksVersion_0, clientInfo: retained.toOpaque(), init: { _, info, storage in
            storage.pointee = info
        }, finalize: { tap in
            Unmanaged<NativeAudioDSP>.fromOpaque(MTAudioProcessingTapGetStorage(tap)).release()
        }, prepare: { tap, frames, format in
            Unmanaged<NativeAudioDSP>.fromOpaque(MTAudioProcessingTapGetStorage(tap)).takeUnretainedValue().prepare(format.pointee, maximumFrames: frames)
        }, unprepare: { _ in }, process: { tap, frames, _, buffers, framesOut, flagsOut in
            let result = MTAudioProcessingTapGetSourceAudio(tap, frames, buffers, flagsOut, nil, framesOut)
            guard result == noErr else { framesOut.pointee = 0; return }
            Unmanaged<NativeAudioDSP>.fromOpaque(MTAudioProcessingTapGetStorage(tap)).takeUnretainedValue().process(buffers, frames: framesOut.pointee, reset: flagsOut.pointee & kMTAudioProcessingTapFlag_StartOfStream != 0)
        })
        var tap: MTAudioProcessingTap?
        guard MTAudioProcessingTapCreate(kCFAllocatorDefault, &callbacks, kMTAudioProcessingTapCreationFlag_PostEffects, &tap) == noErr, let tap else {
            retained.release(); return nil
        }
        let parameters = AVMutableAudioMixInputParameters(track: track)
        parameters.audioTapProcessor = tap
        let mix = AVMutableAudioMix(); mix.inputParameters = [parameters]
        return mix
    }
}
