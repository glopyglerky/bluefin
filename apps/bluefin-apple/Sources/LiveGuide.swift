import SwiftUI

/// Reads the authenticated server's guide. Provider rights and playback policy
/// remain server authority; missing programme data never becomes a fabricated event.
struct LiveGuideView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var channels: [MediaItem] = []
    @State private var programmes: [LiveProgram] = []
    @State private var start = 0
    @State private var total = 0
    @State private var hours = Self.initialHours
    @State private var windowStart = Self.initialStart()
    @State private var favorites = false
    @State private var reload = UUID()
    @State private var loading = false
    @State private var failure: String?
    @State private var generation = UUID()
    @State private var channelsLoadedFor: ChannelPage?
    private static var initialHours: Int {
        #if os(tvOS)
        2
        #else
        6
        #endif
    }
    private static func initialStart() -> Date {
        let now = Date()
        #if os(tvOS)
        guard let hour = Calendar.current.dateInterval(of: .hour, for: now)?.start else { return now }
        return now.timeIntervalSince(hour) >= 1800 ? hour.addingTimeInterval(1800) : hour
        #else
        return now
        #endif
    }
    private let pageSize = 40
    private struct Page: Decodable { let Items: [LiveProgram]; let TotalRecordCount: Int? }
    private struct ChannelPage: Equatable {
        let account: UUID
        let offset: Int
        let favorites: Bool
        let reload: UUID
    }
    private var identity: String {
        "\(store.accountContext)|\(start)|\(hours)|\(windowStart.timeIntervalSince1970)|\(favorites)|\(reload)"
    }
    var body: some View {
        #if os(tvOS)
        televisionGuide.navigationTitle("Live TV").task(id: identity) { await load() }
        #else
        let byChannel = Dictionary(grouping: programmes, by: \.ChannelId)
        List {
            Section {
                Picker("Guide window", selection: $hours) {
                    Text("6 hours").tag(6); Text("12 hours").tag(12); Text("24 hours").tag(24)
                }
                Text("\(windowStart.formatted(date: .abbreviated, time: .shortened)) – \(windowStart.addingTimeInterval(Double(hours) * 3600).formatted(date: .omitted, time: .shortened))")
                    .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            ForEach(channels) { channel in
                Section {
                    let entries = (byChannel[channel.Id] ?? []).sorted { ($0.start ?? .distantFuture) < ($1.start ?? .distantFuture) }
                    if entries.isEmpty {
                        MediaNavigationLink(destination: .channel(channel)) {
                            Text(channel.CurrentProgram?.title ?? "No programme information supplied").foregroundStyle(.secondary)
                        }
                    } else {
                        ScrollView(.horizontal, showsIndicators: true) {
                            LazyHStack(alignment: .top, spacing: 12) {
                                ForEach(entries) { programme in
                                    MediaNavigationLink(destination: .channel(channel)) {
                                        VStack(alignment: .leading, spacing: 8) {
                                            if let item = programme.artwork {
                                                Artwork(item: item, thumbnailWidth: 480)
                                                    .frame(height: 128)
                                                    .clipShape(RoundedRectangle(cornerRadius: 8))
                                            }
                                            if let begin = programme.start, let end = programme.end {
                                                Text("\(begin.formatted(date: .omitted, time: .shortened)) – \(end.formatted(date: .omitted, time: .shortened))").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                                            }
                                            Text(programme.title).font(.headline).lineLimit(3)
                                            if let episode = programme.EpisodeTitle { Text(episode).font(.subheadline).foregroundStyle(.secondary).lineLimit(2) }
                                            if let begin = programme.start, let end = programme.end, begin <= windowStart, end > windowStart {
                                                Label("On now", systemImage: "dot.radiowaves.left.and.right").font(.caption.weight(.semibold)).foregroundStyle(BluefinStyle.accent)
                                            }
                                        }.frame(width: 230, alignment: .leading).padding(16)
                                            .background(BluefinStyle.secondaryBackground, in: RoundedRectangle(cornerRadius: 12))
                                    }.modifier(MusicContentLinkStyle())
                                }
                            }.padding(.vertical, 6)
                        }
                    }
                } header: {
                    HStack(spacing: 8) {
                        if let tag = channel.ImageTags?["Primary"], !tag.isEmpty {
                            Artwork(item: channel, thumbnailWidth: 144, contentMode: .fit)
                                .frame(width: 44, height: 26)
                                .clipShape(RoundedRectangle(cornerRadius: 4))
                        }
                        Text([channel.Number, channel.title].compactMap { $0 }.joined(separator: " · "))
                    }
                }
            }
            if loading { ProgressView("Loading channel guide") }
            if let failure { Text(failure).foregroundStyle(.secondary); Button("Try again") { reload = UUID() } }
            if !loading && channels.isEmpty && failure == nil { Text("No channels are available to this account. Connect a library with authorized live TV sources.").foregroundStyle(.secondary) }
            if total > 0 {
                Section {
                    Text("Channels \(start + 1)–\(min(total, start + channels.count)) of \(total)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                    HStack {
                        Button("Previous channels") { start = max(0, start - pageSize) }.disabled(start == 0 || loading)
                        Spacer()
                        Button("Next channels") { start += pageSize }.disabled(start + channels.count >= total || loading)
                    }
                }
            }
        }.navigationTitle("Channel guide").task(id: identity) { await load() }
            .bluefinRefresh { windowStart = Date(); reload = UUID() }
        #endif
    }
    #if os(tvOS)
    private var televisionGuide: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 18) {
                Button { windowStart = windowStart.addingTimeInterval(-7200) } label: {
                    Label("Earlier", systemImage: "chevron.left")
                }
                Button("Now") { windowStart = Self.initialStart(); reload = UUID() }
                Button { windowStart = windowStart.addingTimeInterval(7200) } label: {
                    Label("Later", systemImage: "chevron.right")
                }
                Spacer()
                Text(windowStart.formatted(date: .abbreviated, time: .omitted))
                    .foregroundStyle(.secondary).accessibilityAddTraits(.isHeader)
                Spacer()
                Button(favorites ? "All channels" : "Favorites") {
                    start = 0
                    favorites.toggle()
                }
                MediaNavigationLink(destination: .channels) { Text("Channels") }
                if store.sports.available {
                    NavigationLink("Sports", destination: NativeSportsGuide(sports: store.sports).bluefinBackNavigation())
                }
            }.buttonStyle(.bordered).focusSection()

            if store.preview {
                EmptyLibrary(title: "Live TV", message: "Connect a server with live channels to open its programme guide.", symbol: "tv")
            } else if channels.isEmpty {
                if loading { ProgressView("Loading the guide") }
                else if let failure { Text(failure).foregroundStyle(.secondary); Button("Try again") { reload = UUID() } }
                else { EmptyLibrary(title: "No channels", message: favorites ? "Your account has no favorite live channels." : "Your server hasn’t provided live channels for this account.", symbol: "tv") }
            } else {
                TVScheduleGrid(channels: channels, programs: programmes, windowStart: windowStart,
                               windowEnd: windowStart.addingTimeInterval(Double(hours) * 3600))
                if loading { ProgressView("Loading programme times").font(.caption) }
                if let failure {
                    HStack(spacing: 22) {
                        Text("Guide data is unavailable. " + failure).font(.caption).foregroundStyle(.secondary)
                        Button("Retry") { reload = UUID() }
                    }.focusSection()
                }
                if total > pageSize || start > 0 {
                    HStack(spacing: 26) {
                        Button("Previous channels") { start = max(0, start - pageSize) }.disabled(start == 0 || loading)
                        Text("Channels \(start + 1)–\(start + channels.count) of \(total)").font(.caption)
                        Button("Next channels") { start += pageSize }.disabled(start + channels.count >= total || loading)
                    }.buttonStyle(.bordered).focusSection()
                }
            }
        }.padding(.horizontal, 32).padding(.vertical, 18).background(BluefinBackground())
    }
    #endif

    private func load() async {
        guard let session = store.session else { channels = []; programmes = []; total = 0; return }
        let account = OfflineLibrary.identity(session); let request = UUID(); generation = request
        let channelPage = ChannelPage(account: store.accountContext, offset: start, favorites: favorites, reload: reload)
        loading = true; failure = nil; programmes = []
        defer { if generation == request { loading = false } }
        func current() -> Bool { generation == request && store.session.map(OfflineLibrary.identity) == account && !Task.isCancelled }
        do {
            if channelsLoadedFor != channelPage {
                channels = []; total = 0; channelsLoadedFor = nil
                var fields: [String: Any] = ["startIndex": channelPage.offset, "limit": pageSize]
                if channelPage.favorites { fields["isFavorite"] = true }
                let page: ItemPage = try await RustBridge.call("liveChannels", session: session, fields: fields)
                guard current() else { return }
                channels = page.Items
                total = page.TotalRecordCount ?? (channelPage.offset + channels.count)
                channelsLoadedFor = channelPage
            }
            guard !channels.isEmpty else { return }
            let channelIDs = channels.map(\.Id)
            let minimum = windowStart.ISO8601Format()
            let maximum = windowStart.addingTimeInterval(Double(hours) * 3600).ISO8601Format()
            var offset = 0
            var seen: Set<String> = []
            while offset < 10_000 {
                let guide: Page = try await RustBridge.call("livePrograms", session: session, fields: [
                    "channelIds": channelIDs, "startIndex": offset, "limit": 200,
                    "minEndDate": minimum, "maxStartDate": maximum
                ])
                guard current() else { return }
                programmes += guide.Items.filter { seen.insert($0.Id).inserted }
                offset += guide.Items.count
                if guide.Items.isEmpty
                    || (guide.TotalRecordCount.map({ offset >= $0 }) ?? (guide.Items.count < 200)) { return }
            }
            failure = "More guide entries are available. Open a channel for its schedule."
        } catch is CancellationError {} catch { if current() { failure = error.localizedDescription } }
    }
}

/// Maps server intervals onto one shared timeline. Missing data fills the gaps;
/// overlapping listings never push a cell past the end of the displayed window.
struct GuideTimeline {
    struct Segment: Identifiable {
        let start: Date
        let end: Date
        let program: LiveProgram?
        var id: Date { start }
    }

    let start: Date
    let end: Date
    private var duration: TimeInterval { end.timeIntervalSince(start) }

    func fraction(at time: Date) -> Double {
        guard duration > 0 else { return 0 }
        return min(1, max(0, time.timeIntervalSince(start) / duration))
    }

    func segments(_ programs: [LiveProgram]) -> [Segment] {
        guard start < end else { return [] }
        let candidates = programs.compactMap { program -> Segment? in
            guard let starts = program.start, let ends = program.end, starts < ends,
                  starts < end, ends > start else { return nil }
            return .init(start: max(start, starts), end: min(end, ends), program: program)
        }.sorted {
            if $0.start != $1.start { return $0.start < $1.start }
            if $0.end != $1.end { return $0.end < $1.end }
            return ($0.program?.Id ?? "") < ($1.program?.Id ?? "")
        }
        var cursor = start
        var result: [Segment] = []
        for candidate in candidates where candidate.end > cursor {
            if candidate.start > cursor { result.append(.init(start: cursor, end: candidate.start, program: nil)) }
            result.append(.init(start: max(cursor, candidate.start), end: candidate.end, program: candidate.program))
            cursor = candidate.end
        }
        if cursor < end { result.append(.init(start: cursor, end: end, program: nil)) }
        return result
    }
}

#if os(tvOS)
private struct TVScheduleGrid: View {
    @EnvironmentObject private var store: LibraryStore
    let channels: [MediaItem]
    let programs: [LiveProgram]
    let windowStart: Date
    let windowEnd: Date
    private let channelWidth: CGFloat = 272
    private let rowHeight: CGFloat = 95
    private var timeline: GuideTimeline { GuideTimeline(start: windowStart, end: windowEnd) }

    var body: some View {
        let byChannel = Dictionary(grouping: programs, by: \.ChannelId)
        TimelineView(.periodic(from: .now, by: 60)) { clock in
            VStack(spacing: 6) {
                HStack(spacing: 8) {
                    Text("Channel").font(.callout.weight(.semibold))
                        .frame(width: channelWidth, alignment: .leading)
                    HStack(spacing: 0) {
                        ForEach(0..<4) { offset in
                            Text(windowStart.addingTimeInterval(Double(offset) * 1800).formatted(date: .omitted, time: .shortened))
                                .font(.callout.monospacedDigit()).frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }.foregroundStyle(.secondary)
                }.padding(.horizontal, 4).frame(height: 42).accessibilityAddTraits(.isHeader)
                ScrollView {
                    LazyVStack(spacing: 6) {
                        ForEach(channels) { channel in
                            row(channel, programs: byChannel[channel.Id] ?? [], now: clock.date)
                        }
                    }
                }
            }.focusSection()
        }
    }

    private func row(_ channel: MediaItem, programs: [LiveProgram], now: Date) -> some View {
        let cells = timeline.segments(programs)
        return HStack(spacing: 8) {
            MediaNavigationLink(destination: .channel(channel)) {
                HStack(spacing: 12) {
                    Artwork(item: channel, thumbnailWidth: 224, contentMode: .fit)
                        .frame(width: 72, height: 52).clipShape(RoundedRectangle(cornerRadius: 6))
                    VStack(alignment: .leading, spacing: 4) {
                        if let number = channel.Number { Text(number).font(.caption2.monospacedDigit()).foregroundStyle(.secondary) }
                        Text(channel.title).font(.caption.weight(.semibold)).lineLimit(2)
                    }
                }.padding(.horizontal, 12)
                .frame(width: channelWidth, height: rowHeight, alignment: .leading)
                .modifier(TVGuideFocus())
            }.buttonStyle(.plain)
            GeometryReader { geometry in
                HStack(spacing: 0) {
                    ForEach(cells) { segment in
                        let width = geometry.size.width * (timeline.fraction(at: segment.end) - timeline.fraction(at: segment.start))
                        let active = segment.program.map { ($0.start ?? .distantFuture) <= now && ($0.end ?? .distantPast) > now } ?? false
                        cell(segment, channel: channel, width: width, active: active)
                    }
                }
            }.frame(height: rowHeight)
        }.frame(height: rowHeight)
    }

    @ViewBuilder private func cell(_ segment: GuideTimeline.Segment, channel: MediaItem,
                                   width: CGFloat, active: Bool) -> some View {
        let program = segment.program
        let name = program?.title ?? "No guide data"
        let time = segment.start.formatted(date: .omitted, time: .shortened)
        if width < 16 {
            Color.clear.frame(width: width, height: rowHeight)
        } else if active || program == nil {
            Button {
                Task { await store.play(channel) }
            } label: {
                cellLabel(title: name, subtitle: active ? "On now" : "Watch channel",
                          artwork: program?.artwork, width: width, active: active)
            }.buttonStyle(.plain)
                .accessibilityLabel(active ? "Watch \(name) on \(channel.title), on now" : "Watch \(channel.title), guide data unavailable")
                .disabled(store.startingPlayback)
        } else {
            MediaNavigationLink(destination: .channel(channel)) {
                cellLabel(title: name, subtitle: time, artwork: program?.artwork, width: width, active: false)
            }.buttonStyle(.plain).accessibilityLabel("\(name), \(time), \(channel.title)")
        }
    }

    private func cellLabel(title: String, subtitle: String, artwork: MediaItem?, width: CGFloat, active: Bool) -> some View {
        HStack(spacing: 12) {
            if width >= 250, let artwork {
                Artwork(item: artwork, thumbnailWidth: 224)
                    .frame(width: 88, height: 58).clipShape(RoundedRectangle(cornerRadius: 6))
            }
            VStack(alignment: .leading, spacing: 5) {
                Text(title).font(.caption.weight(.semibold)).lineLimit(2)
                Text(subtitle).font(.caption2).foregroundStyle(active ? BluefinStyle.accent : .secondary).lineLimit(1)
            }
        }
        .padding(.horizontal, 12).frame(width: max(0, width - 4), height: rowHeight, alignment: .leading)
        .clipped().modifier(TVGuideFocus(active: active)).padding(.trailing, 4)
    }
}

private struct TVGuideFocus: ViewModifier {
    var active = false
    @Environment(\.isFocused) private var focused

    func body(content: Content) -> some View {
        content.background(
            focused ? BluefinStyle.accent.opacity(0.34) : active ? BluefinStyle.accent.opacity(0.16) : BluefinStyle.secondaryBackground,
            in: RoundedRectangle(cornerRadius: 8)
        ).overlay {
            RoundedRectangle(cornerRadius: 8).strokeBorder(BluefinStyle.accent.opacity(focused ? 1 : 0), lineWidth: 3)
        }
    }
}
#endif
