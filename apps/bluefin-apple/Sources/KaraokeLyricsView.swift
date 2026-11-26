import SwiftUI
import AVFoundation

struct KaraokeLyricsView: View {
    @ObservedObject var playback: PlaybackController
    @ObservedObject var audio: KaraokeAudio
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @State private var following = true
    @State private var leaving = false
    @State private var microphoneVisible = false
    var body: some View {
        VStack(spacing: 0) {
            // Read AVPlayer's actual clock each frame. Never extrapolate through pause/buffering.
            TimelineView(.animation(minimumInterval: reduceMotion ? 1.0 / 8 : 1.0 / 30, paused: !playback.isPlaying)) { _ in
                let time = playback.lyricPositionSeconds
                let active = audio.package.timeline.activeLines(at: time)
                ScrollViewReader { reader in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 30) {
                            ForEach(audio.package.timeline.lines) { line in
                                let singing = active.contains { $0.id == line.id }
                                Button { playback.seek(seconds: max(0, line.start + audio.package.timeline.offsetSeconds)) } label: {
                                    VStack(alignment: line.voice == .partner ? .trailing : .leading, spacing: 8) {
                                        Text(voiceLabel(line.voice)).font(.caption.weight(.semibold)).textCase(.uppercase)
                                            .foregroundColor(tint(line.voice)).opacity(singing ? 1 : 0.5)
                                        words(line, at: time - audio.package.timeline.offsetSeconds)
                                            .font(.system(.title, design: .rounded).weight(.bold))
                                            .multilineTextAlignment(line.voice == .partner ? .trailing : .leading)
                                    }.frame(maxWidth: .infinity, alignment: line.voice == .partner ? .trailing : .leading)
                                        .padding(.leading, line.voice == .partner ? 44 : 0)
                                        .padding(.trailing, line.voice == .lead ? 44 : 0)
                                        .opacity(singing ? 1 : 0.5)
                                }.buttonStyle(.plain).id(line.id)
                                    .accessibilityLabel("\(voiceLabel(line.voice)): \(line.text)")
                                    .accessibilityHint("Seek to this lyric")
                            }
                        }.padding(28).frame(maxWidth: 760).frame(maxWidth: .infinity)
                    }
                    #if os(iOS)
                    .simultaneousGesture(DragGesture(minimumDistance: 12).onChanged { _ in following = false })
                    #endif
                    .onChange(of: active.first?.id) { id in
                        guard following, let id else { return }
                        withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.25)) { reader.scrollTo(id, anchor: .center) }
                    }
                    .onChange(of: following) { enabled in
                        guard enabled, let id = active.first?.id else { return }
                        withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.25)) { reader.scrollTo(id, anchor: .center) }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        if !following {
                            Button("Follow lyrics") {
                                following = true
                            }.padding(.horizontal, 16).padding(.vertical, 12)
                                .modifier(BluefinGlassSurface(cornerRadius: 24)).padding()
                        }
                    }
                }
            }
            ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Label(audio.vocalLevel == 0 ? "Instrumental" : "Lead vocal", systemImage: "waveform")
                    Spacer()
                    Text("\(Int(audio.vocalLevel * 100))%").monospacedDigit()
                }.font(.headline)
                #if os(iOS)
                Slider(value: Binding(get: { Double(audio.vocalLevel) }, set: { audio.setVocalLevel(Float($0)) }), in: 0...1)
                    .accessibilityLabel("Lead vocal level")
                #else
                HStack {
                    Button("Less vocal") { audio.setVocalLevel(audio.vocalLevel - 0.1) }
                    Button("More vocal") { audio.setVocalLevel(audio.vocalLevel + 0.1) }
                }
                #endif
                #if os(tvOS)
                Button(following ? "Browse lyrics" : "Follow lyrics") { following.toggle() }
                #endif
                HStack {
                    Button("Instrumental") { audio.setVocalLevel(0) }
                    Button("With singer") { audio.setVocalLevel(1) }
                    Spacer()
                    Button(playback.isPlaying ? "Pause" : "Play") { playback.toggle() }
                }.buttonStyle(.bordered)
                Button { microphoneVisible.toggle() } label: { Label("Microphone", systemImage: "mic.fill") }
                if microphoneVisible {
                    #if os(tvOS)
                    if #available(tvOS 17.0, *) {
                        KaraokeContinuityControls(microphone: playback.continuityMicrophone)
                    } else { Text("An iPhone microphone requires tvOS 17 or later and a compatible Apple TV.").font(.caption).foregroundStyle(.secondary) }
                    #else
                    KaraokeMicrophoneControls(microphone: playback.karaokeMicrophone)
                    #endif
                }
                Button("Return to original audio") {
                    leaving = true
                    Task { await playback.disableKaraoke(); leaving = false }
                }.disabled(leaving)
            }.padding(20).frame(maxWidth: 760).frame(maxWidth: .infinity)
            }.frame(maxHeight: dynamicTypeSize.isAccessibilitySize || microphoneVisible ? 340 : 220)
                .modifier(BluefinGlassSurface(cornerRadius: 28)).padding(.horizontal, 16).padding(.bottom, 12)
        }
        .background(BluefinBackground())
        .onDisappear { playback.stopKaraokeMicrophones() }
    }
    private func tint(_ voice: KaraokeLine.Voice) -> Color { voice == .partner ? .mint : voice == .backing ? .orange : BluefinStyle.accent }
    private func voiceLabel(_ voice: KaraokeLine.Voice) -> String {
        switch voice { case .lead: return "Voice 1"; case .partner: return "Voice 2"; case .together: return "Together"; case .backing: return "Backing vocals" }
    }
    private func words(_ line: KaraokeLine, at seconds: Double) -> some View {
        // Canvas wraps measured tokens and clips each colored overlay using its
        // supplied timing. No word boundaries or timing are inferred from line text.
        return TimedLyricsWordFill(runs: line.tokens.map { .init(text: $0.text, start: $0.start, end: $0.end) },
                                   seconds: seconds, color: tint(line.voice), rightAligned: line.voice == .partner)
            .accessibilityHidden(true)

    }
}

struct KaraokeMicrophoneControls: View {
    @ObservedObject var microphone: KaraokeMicrophone
    @State private var requesting = false
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            #if os(iOS)
            Button(microphone.isMonitoring ? "Stop microphone" : "Start headphone microphone") {
                if microphone.isMonitoring { microphone.stop() }
                else { requesting = true; Task { await microphone.start(); requesting = false } }
            }.disabled(requesting)
            #endif
            Label(microphone.status, systemImage: microphone.isMonitoring ? "mic.fill" : "mic.slash.fill")
                .font(.caption).foregroundStyle(microphone.isMonitoring ? BluefinStyle.accent : .secondary)
            #if os(iOS)
            Text("For your TV, open Bluefin’s Sing controls on Apple TV and choose Connect iPhone microphone.")
                .font(.caption).foregroundStyle(.secondary)
            #endif
        }.onDisappear { microphone.stop() }
    }
}

/// Word/syllable fill uses supplied token intervals. Canvas measures each token,
/// wraps at token boundaries and clips only its colored overlay as time advances.
struct TimedLyricsWordFill: View {
    let runs: [LyricDisplayRun]
    let seconds: Double
    let color: Color
    var rightAligned = false
    @State private var height: CGFloat = 80
    #if os(tvOS)
    @ScaledMetric(relativeTo: .title) private var fontSize: CGFloat = 48
    #else
    @ScaledMetric(relativeTo: .title) private var fontSize: CGFloat = 28
    #endif
    var body: some View {
        GeometryReader { _ in
            Canvas { context, size in
                let row = fontSize * 1.6
                var rows: [[(LyricDisplayRun, CGFloat)]] = [[]]
                var widths: [CGFloat] = [0]
                for token in runs {
                    let text = Text(token.text).font(.system(size: fontSize, weight: .bold, design: .rounded))
                    let width = min(size.width, context.resolve(text).measure(in: CGSize(width: .infinity, height: row)).width)
                    let index = rows.count - 1
                    if widths[index] + width > size.width && !rows[index].isEmpty { rows.append([]); widths.append(0) }
                    rows[rows.count - 1].append((token, width)); widths[widths.count - 1] += width
                }
                var y: CGFloat = 0
                for (index, tokens) in rows.enumerated() {
                    var x: CGFloat = rightAligned ? max(0, size.width - widths[index]) : 0
                    var rowHeight = row
                    for (token, width) in tokens {
                        let text = Text(token.text).font(.system(size: fontSize, weight: .bold, design: .rounded))
                        let measured = context.resolve(text).measure(in: CGSize(width: width, height: .infinity))
                        let h = max(row, measured.height)
                        rowHeight = max(rowHeight, h)
                        let rect = CGRect(x: x, y: y, width: width, height: h)
                        context.draw(text.foregroundColor(BluefinStyle.text.opacity(0.5)), in: rect)
                        var fill = context
                        fill.clip(to: Path(CGRect(x: x, y: y, width: width * token.progress(at: seconds), height: h)))
                        fill.draw(text.foregroundColor(color), in: rect)
                        x += width
                    }
                    y += rowHeight
                }
                let needed = y
                if abs(height - needed) > 1 { DispatchQueue.main.async { height = needed } }
            }
        }.frame(height: height)
    }
}
