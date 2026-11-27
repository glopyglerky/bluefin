import Foundation

@MainActor private final class MicrophoneFixture: KaraokeContinuityAudio {
    var starts = 0
    var stops = 0
    var outputGain: Float = 0
    var permission = true
    var suspend = false
    var pending: CheckedContinuation<Bool, Never>?
    func requestPermission() async -> Bool {
        if suspend { return await withCheckedContinuation { pending = $0 } }
        return permission
    }
    func start(gain: Float) throws { starts += 1; outputGain = gain }
    func setGain(_ gain: Float) { outputGain = gain }
    func stop() throws { stops += 1; outputGain = 0 }
}

@main struct MusicLyricsChecks {
    @MainActor static func main() async throws {
        let programme = try JSONDecoder().decode(MediaItem.self, from: Data(#"{"Id":"programme","Type":"Program","StartDate":"2026-11-27T01:00:00Z","EndDate":"2026-11-27T02:00:00.000Z"}"#.utf8))
        precondition(programme.start != nil && programme.end?.timeIntervalSince(programme.start!) == 3600)
        let unscheduled = try JSONDecoder().decode(MediaItem.self, from: Data(#"{"Id":"programme","Type":"Program","StartDate":"bad","EndDate":"bad"}"#.utf8))
        precondition(unscheduled.start == nil && unscheduled.end == nil, "Do not invent a live broadcast window")

        let fixture = #"{"Metadata":{"IsSynced":true,"Offset":3000000},"Lyrics":[{"Text":"Go 🎵 now","Start":20000000,"Cues":[{"Position":0,"EndPosition":3,"Start":20000000,"End":25000000},{"Position":3,"EndPosition":5,"Start":25000000,"End":30000000},{"Position":5,"EndPosition":9,"Start":30000000,"End":40000000}]},{"Text":"Next line","Start":50000000}]}"#
        let lyrics = try JSONDecoder().decode(BluefinLyrics.self, from: Data(fixture.utf8))
        precondition(lyrics.activeIndex(at: 2.299) == nil)
        precondition(lyrics.activeIndex(at: 2.3) == 0, "Exact positive offset boundary")
        precondition(lyrics.activeIndex(at: 5.299) == 0)
        precondition(lyrics.activeIndex(at: 5.3) == 1)
        precondition(lyrics.activeIndex(at: .nan) == nil && lyrics.activeIndex(at: -.infinity) == nil)
        precondition(lyrics.seekSeconds(for: 1) == 5.3)
        precondition(lyrics.activeIndex(at: lyrics.seekSeconds(for: 0)!) == 0, "Seek and highlight share offset semantics")
        precondition(lyrics.seekSeconds(for: -1) == nil && lyrics.seekSeconds(for: 2) == nil)
        let runs = lyrics.Lyrics[0].displayRuns
        precondition(runs.map(\.text) == ["Go ", "🎵", " now"], "UTF-16 ranges must preserve emoji")
        precondition(runs.map(\.text).joined() == lyrics.Lyrics[0].Text)
        precondition(runs[1].progress(at: 2.5) == 0)
        precondition(runs[1].progress(at: 2.75) == 0.5)
        precondition(runs[1].progress(at: 3) == 1)
        precondition(runs[1].progress(at: 4) == 1)
        precondition(runs[1].progress(at: 2) == 0)
        precondition(runs[1].progress(at: .nan) == 0)
        let malformed = #"{"Metadata":{"IsSynced":true},"Lyrics":[{"Text":"A🎵B","Start":0,"Cues":[{"Position":1,"EndPosition":2,"Start":0,"End":10000000},{"Position":1,"EndPosition":3,"Start":0,"End":10000000},{"Position":0,"EndPosition":1,"Start":20000000,"End":10000000},{"Position":3,"EndPosition":4,"Start":10000000}]}]}"#
        let guarded = try JSONDecoder().decode(BluefinLyrics.self, from: Data(malformed.utf8)).Lyrics[0].displayRuns
        precondition(guarded.map(\.text) == ["A", "🎵", "B"], "Split surrogates/overlap and missing end must preserve plain text")
        precondition(guarded.filter { $0.start != nil }.count == 1)
        precondition(guarded.last?.progress(at: 10) == 0, "Never invent final-cue duration")
        let plain = try JSONDecoder().decode(BluefinLyrics.self, from: Data(#"{"Metadata":{"IsSynced":false},"Lyrics":[{"Text":"Plain text","Start":0}]}"#.utf8))
        precondition(plain.activeIndex(at: 1) == nil && plain.seekSeconds(for: 0) == nil)
        let missingFlag = try JSONDecoder().decode(BluefinLyrics.self, from: Data(#"{"Metadata":{},"Lyrics":[{"Text":"Timed line","Start":20000000,"Cues":[{"Position":0,"EndPosition":5,"Start":20000000,"End":30000000}]},{"Text":"Next line","Start":50000000}]}"#.utf8))
        precondition(missingFlag.isSynced && missingFlag.activeIndex(at: 1.999) == nil)
        precondition(missingFlag.activeIndex(at: 2) == 0 && missingFlag.activeIndex(at: 5) == 1)
        precondition(missingFlag.seekSeconds(for: 1) == 5, "Empty server metadata must not discard actual line timing")
        precondition(missingFlag.Lyrics[0].displayRuns[0].progress(at: 2.5) == 0.5)
        let missingFlagPlain = try JSONDecoder().decode(BluefinLyrics.self, from: Data(#"{"Metadata":{},"Lyrics":[{"Text":"Plain text"}]}"#.utf8))
        precondition(!missingFlagPlain.isSynced && missingFlagPlain.activeIndex(at: 1) == nil && missingFlagPlain.seekSeconds(for: 0) == nil)
        let invalidTiming = try JSONDecoder().decode(BluefinLyrics.self, from: Data(#"{"Metadata":{},"Lyrics":[{"Text":"Invalid timing","Start":-1}]}"#.utf8))
        precondition(!invalidTiming.isSynced && invalidTiming.activeIndex(at: 1) == nil && invalidTiming.seekSeconds(for: 0) == nil)

        let negative = try JSONDecoder().decode(BluefinLyrics.self, from: Data(#"{"Metadata":{"IsSynced":true,"Offset":-10000000},"Lyrics":[{"Text":"Early","Start":5000000}]}"#.utf8))
        precondition(negative.seekSeconds(for: 0) == 0 && negative.activeIndex(at: 0) == 0)

        let input = MicrophoneFixture(), mic = KaraokeContinuityLifecycle(audio: MicrophoneFixture())
        await mic.enable()
        precondition(mic.state == .off)
        let model = KaraokeContinuityLifecycle(audio: input)
        precondition(model.selected(token: model.beginSelection()))
        precondition(input.starts == 0 && model.isMuted, "Selecting a phone never starts capture")
        await model.enable()
        precondition(input.starts == 1 && !model.isMuted && model.state == .monitoring)
        model.setMuted(true)
        precondition(input.outputGain == 0 && model.isMuted)
        model.setGain(0.2)
        precondition(input.outputGain == 0, "Level adjustment cannot unmute")
        model.setMuted(false)
        precondition(input.outputGain == 0.2)
        model.setGain(100)
        precondition(input.outputGain == 0.25)
        model.interrupted()
        precondition(input.outputGain == 0 && model.isMuted && model.state == .interrupted)
        model.setMuted(false)
        await model.enable()
        precondition(input.starts == 1, "Interruption requires a new explicit connection")

        let delayed = MicrophoneFixture(); delayed.suspend = true
        let pending = KaraokeContinuityLifecycle(audio: delayed)
        let token = pending.beginSelection()
        precondition(pending.selected(token: token))
        let start = Task { await pending.enable() }
        for _ in 0..<100 where delayed.pending == nil { await Task.yield() }
        precondition(delayed.pending != nil)
        pending.stop()
        delayed.pending?.resume(returning: true); delayed.pending = nil
        await start.value
        precondition(delayed.starts == 0 && pending.state == .off)
        precondition(!pending.selected(token: token), "Dismissed/account-changed picker callback stays rejected")
        print("PASS: absent sync metadata, lyric offset/seek boundaries, UTF-16 emoji ranges, supplied word progress, invalid cues, plain lyrics, mute/level/stop, interruption and late-permission cleanup")
    }
}
