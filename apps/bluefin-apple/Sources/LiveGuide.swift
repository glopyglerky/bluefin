import SwiftUI

/// The account's channel and programme IDs remain the playback authority.
/// Channel pages are fetched near the viewport and evicted outside three pages.
@MainActor struct LiveGuideView: View {
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dynamicTypeSize) private var dynamicType
    @State private var pages: [Int: ChannelPage] = [:]
    @State private var requests: [Int: UUID] = [:]
    @State private var pageTasks: [Int: Task<Void, Never>] = [:]
    @State private var total = 0
    @State private var anchorOffset = 0
    @State private var hours = Self.initialHours
    @State private var windowStart = Self.initialStart()
    @State private var favorites = false
    @State private var reload = UUID()
    @State private var generation = UUID()
    @State private var loadedIdentity: String?
    @State private var loadedChannelScope: String?
    @State private var loadedReload: UUID?
    @State private var horizontalOffset: CGFloat = 0
    @State private var selectedProgram: GuideProgramSelection?

    private static let pageSize = 40
    private struct ProgramPage: Decodable { let Items: [LiveProgram]; let TotalRecordCount: Int? }
    private struct ChannelPage {
        var channels: [MediaItem]
        var programs: [String: [LiveProgram]] = [:]
        var complete = false
        var failure: String?
    }
    private static var initialHours: Int {
        #if os(tvOS)
        2
        #else
        6
        #endif
    }
    private static func initialStart() -> Date {
        let now = Date()
        guard let hour = Calendar.current.dateInterval(of: .hour, for: now)?.start else { return now }
        return now.timeIntervalSince(hour) >= 1800 ? hour.addingTimeInterval(1800) : hour
    }
    private var identity: String {
        "\(store.accountContext)|\(hours)|\(windowStart.timeIntervalSince1970)|\(favorites)|\(reload)"
    }
    private var rowHeight: CGFloat {
        #if os(tvOS)
        100
        #else
        dynamicType.isAccessibilitySize ? 116 : 84
        #endif
    }
    private var failure: String? { pages[anchorOffset]?.failure ?? pages[0]?.failure }

    var body: some View {
        VStack(spacing: 12) {
            controls
            if store.preview {
                EmptyLibrary(title: "Live TV", message: "Connect a server with live channels to open its programme guide.", symbol: "tv")
            } else if total == 0 && pageTasks.isEmpty {
                if let failure {
                    Text(failure).foregroundStyle(.secondary)
                    Button("Try again") { requestPage(0, retry: true) }
                } else {
                    EmptyLibrary(title: "No channels", message: favorites ? "Your account has no favorite live channels." : "Your server has not provided live channels for this account.", symbol: "tv")
                }
                Spacer()
            } else {
                schedule
                if let failure {
                    HStack {
                        Text(failure).font(.caption).foregroundStyle(.secondary)
                        Button("Retry") { requestPage(anchorOffset, retry: true) }
                    }.padding(.horizontal)
                }
            }
        }
        .background(BluefinBackground())
        .navigationTitle("Guide")
        .task(id: identity) { startWindowIfNeeded() }
        .onDisappear { stopRequests() }
        .bluefinRefresh { windowStart = Self.initialStart(); reload = UUID() }
        .sheet(item: $selectedProgram) { selection in
            BluefinNavigation { GuideProgramDetails(selection: selection) }
                .environmentObject(store)
        }
    }

    @ViewBuilder private var controls: some View {
        #if os(tvOS)
        HStack(spacing: 20) {
            Button { windowStart = windowStart.addingTimeInterval(-7200) } label: { Label("Earlier", systemImage: "chevron.left") }
            Button("Now") { windowStart = Self.initialStart(); reload = UUID() }
            Button { windowStart = windowStart.addingTimeInterval(7200) } label: { Label("Later", systemImage: "chevron.right") }
            Spacer()
            Text(windowStart.formatted(date: .abbreviated, time: .omitted)).foregroundStyle(.secondary)
            Button(favorites ? "All channels" : "Favorites") { favorites.toggle() }
            MediaNavigationLink(destination: .channels) { Text("Channels") }
            if store.sports.available {
                NavigationLink("Sports", destination: NativeSportsGuide(sports: store.sports).bluefinBackNavigation())
            }
        }.buttonStyle(.bordered).padding(.horizontal, 32).focusSection()
        #else
        VStack(spacing: 10) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(0..<7) { offset in
                        let day = Calendar.current.date(byAdding: .day, value: offset, to: Calendar.current.startOfDay(for: Date())) ?? Date()
                        let selected = Calendar.current.isDate(day, inSameDayAs: windowStart)
                        Button {
                            windowStart = offset == 0 ? Self.initialStart() : day
                        } label: {
                            VStack(spacing: 3) {
                                Text(offset == 0 ? "Today" : day.formatted(.dateTime.weekday(.abbreviated))).font(.caption.weight(.semibold))
                                Text(day.formatted(.dateTime.day())).font(.body.weight(.semibold))
                            }
                            .frame(minWidth: 48).padding(.vertical, 8).padding(.horizontal, 7)
                            .foregroundStyle(selected ? BluefinStyle.onPrimary : BluefinStyle.text)
                            .background(selected ? BluefinStyle.primaryFill : BluefinStyle.secondaryBackground, in: Capsule())
                        }.buttonStyle(.plain)
                        .accessibilityLabel(day.formatted(date: .complete, time: .omitted))
                        .accessibilityAddTraits(selected ? .isSelected : [])
                    }
                }.padding(.horizontal, 16)
            }
            HStack(spacing: 14) {
                Button("Now") { windowStart = Self.initialStart(); reload = UUID() }
                Spacer()
                Picker("Guide window", selection: $hours) {
                    Text("6 hours").tag(6)
                    Text("12 hours").tag(12)
                    Text("24 hours").tag(24)
                }.pickerStyle(.menu)
                Button { favorites.toggle() } label: {
                    Image(systemName: favorites ? "star.fill" : "star")
                }.accessibilityLabel(favorites ? "Show all channels" : "Show favorite channels")
            }.padding(.horizontal, 18).font(.subheadline)
        }
        #endif
    }

    private var schedule: some View {
        GeometryReader { viewport in
            #if os(tvOS)
            let channelWidth: CGFloat = 272
            let halfHourWidth = max(180, (viewport.size.width - channelWidth) / 4)
            #else
            let channelWidth: CGFloat = dynamicType.isAccessibilitySize ? 132 : 100
            let halfHourWidth: CGFloat = dynamicType.isAccessibilitySize ? 210 : 160
            #endif
            let timelineWidth = max(viewport.size.width - channelWidth, CGFloat(hours * 2) * halfHourWidth)
            let contentWidth = channelWidth + timelineWidth
            TimelineView(.periodic(from: .now, by: 60)) { clock in
                ScrollView(.horizontal, showsIndicators: true) {
                    ScrollView(.vertical, showsIndicators: true) {
                        LazyVStack(spacing: 1, pinnedViews: [.sectionHeaders]) {
                            Section {
                                ForEach(0..<total, id: \.self) { index in
                                    scheduleRow(index, channelWidth: channelWidth, timelineWidth: timelineWidth, now: clock.date)
                                        .onAppear { rowAppeared(index) }
                                }
                            } header: {
                                timeHeader(channelWidth: channelWidth, timelineWidth: timelineWidth)
                            }
                        }.frame(width: contentWidth)
                    }
                    .frame(width: contentWidth, height: viewport.size.height)
                    .background {
                        GeometryReader { position in
                            Color.clear.preference(key: GuideHorizontalOffset.self,
                                value: position.frame(in: .named("bluefin-guide-horizontal")).minX)
                        }
                    }
                }
                .coordinateSpace(name: "bluefin-guide-horizontal")
                .onPreferenceChange(GuideHorizontalOffset.self) { horizontalOffset = max(0, -$0) }
            }
        }
    }

    private func timeHeader(channelWidth: CGFloat, timelineWidth: CGFloat) -> some View {
        HStack(spacing: 0) {
            Text("Channel").font(.caption.weight(.semibold))
                .frame(width: channelWidth, height: 40)
                .background(BluefinStyle.background)
                .offset(x: horizontalOffset).zIndex(2)
            HStack(spacing: 0) {
                ForEach(0..<(hours * 2), id: \.self) { offset in
                    Text(windowStart.addingTimeInterval(Double(offset) * 1800).formatted(date: .omitted, time: .shortened))
                        .font(.caption.monospacedDigit()).padding(.leading, 10)
                        .frame(width: timelineWidth / CGFloat(hours * 2), alignment: .leading)
                }
            }.frame(height: 40).background(BluefinStyle.background)
        }.accessibilityAddTraits(.isHeader)
    }

    @ViewBuilder private func scheduleRow(_ index: Int, channelWidth: CGFloat, timelineWidth: CGFloat, now: Date) -> some View {
        let offset = (index / Self.pageSize) * Self.pageSize
        let local = index - offset
        if let page = pages[offset], page.channels.indices.contains(local) {
            let channel = page.channels[local]
            HStack(spacing: 0) {
                channelLabel(channel, width: channelWidth)
                    .offset(x: horizontalOffset).zIndex(2)
                programRow(channel, programs: page.programs[channel.Id] ?? [], width: timelineWidth, now: now,
                           loading: !page.complete && page.failure == nil)
            }.frame(height: rowHeight)
        } else {
            HStack(spacing: 0) {
                Group {
                    if pages[offset]?.failure != nil {
                        Button("Retry") { requestPage(offset, retry: true) }
                    } else { ProgressView().accessibilityLabel("Loading channel") }
                }
                .frame(width: channelWidth, height: rowHeight)
                .background(BluefinStyle.background).offset(x: horizontalOffset).zIndex(2)
                Color.clear.frame(width: timelineWidth, height: rowHeight)
                    .background(BluefinStyle.secondaryBackground.opacity(0.45))
            }
        }
    }

    private func channelLabel(_ channel: MediaItem, width: CGFloat) -> some View {
        MediaNavigationLink(destination: .channel(channel)) {
            VStack(spacing: 4) {
                Artwork(item: channel, thumbnailWidth: 160, contentMode: .fit)
                    .frame(width: min(width - 20, 90), height: 36)
                Text([channel.Number, channel.title].compactMap { $0 }.joined(separator: " · "))
                    .font(.caption2.weight(.medium)).lineLimit(2)
            }
            .frame(width: width, height: rowHeight)
            .background(BluefinStyle.background)
        }.buttonStyle(.plain).accessibilityLabel("\(channel.Number ?? "") \(channel.title)")
    }

    private func programRow(_ channel: MediaItem, programs: [LiveProgram], width: CGFloat, now: Date, loading: Bool) -> some View {
        let timeline = GuideTimeline(start: windowStart, end: windowStart.addingTimeInterval(Double(hours) * 3600))
        return HStack(spacing: 0) {
            ForEach(timeline.segments(programs)) { segment in
                let cellWidth = width * (timeline.fraction(at: segment.end) - timeline.fraction(at: segment.start))
                programCell(segment, channel: channel, width: cellWidth, now: now, loading: loading)
            }
        }
        .frame(width: width, height: rowHeight)
        .overlay(alignment: .leading) {
            if now >= timeline.start && now < timeline.end {
                Rectangle().fill(BluefinStyle.accent).frame(width: 2)
                    .offset(x: width * timeline.fraction(at: now)).allowsHitTesting(false).accessibilityHidden(true)
            }
        }
    }

    @ViewBuilder private func programCell(_ segment: GuideTimeline.Segment, channel: MediaItem, width: CGFloat, now: Date, loading: Bool) -> some View {
        let program = segment.program
        let active = program.map { ($0.start ?? .distantFuture) <= now && ($0.end ?? .distantPast) > now } ?? false
        let title = program?.title ?? (loading ? "Loading schedule…" : "No guide data")
        let episode = program?.EpisodeTitle?.trimmingCharacters(in: .whitespacesAndNewlines)
        let subtitle = (episode?.isEmpty == false ? episode : nil) ?? (active ? "On now" : "")
        if width < 12 {
            Color.clear.frame(width: width, height: rowHeight)
        } else if let program {
            Button { selectedProgram = GuideProgramSelection(channel: channel, program: program, accountContext: store.accountContext) } label: {
                programLabel(title: title, subtitle: subtitle, width: width, active: active)
            }.buttonStyle(.plain)
                .accessibilityLabel("\(title), \(subtitle), \(segment.start.formatted(date: .omitted, time: .shortened)), \(channel.title)")
                .contextMenu {
                    Button("Watch channel") { Task { await store.play(channel) } }
                    Button("Programme details") {
                        selectedProgram = GuideProgramSelection(channel: channel, program: program, accountContext: store.accountContext)
                    }
                }
        } else {
            Button { Task { await store.play(channel) } } label: {
                programLabel(title: title, subtitle: subtitle, width: width, active: active)
            }.buttonStyle(.plain).disabled(store.startingPlayback)
                .accessibilityLabel("Watch \(title) on \(channel.title)" + (subtitle.isEmpty ? "" : ", \(subtitle)"))
        }
    }

    private func programLabel(title: String, subtitle: String, width: CGFloat, active: Bool) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title).font(.subheadline.weight(.semibold)).lineLimit(2)
            if !subtitle.isEmpty { Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
        }
        .padding(.horizontal, 10).frame(width: max(0, width - 1), height: rowHeight, alignment: .leading)
        .clipped()
        .background(active ? BluefinStyle.accent.opacity(0.18) : BluefinStyle.secondaryBackground)
        #if os(tvOS)
        .modifier(TVGuideFocus(active: active))
        #endif
        .padding(.trailing, 1)
    }

    private func startWindowIfNeeded() {
        guard !store.preview, store.session != nil else {
            stopRequests(); pages = [:]; total = 0; loadedIdentity = identity
            return
        }
        if loadedIdentity != identity {
            stopRequests()
            generation = UUID()
            let channelScope = "\(store.accountContext)|\(favorites)"
            if loadedChannelScope != channelScope {
                pages = [:]
                total = Self.pageSize
                anchorOffset = 0
            } else if loadedReload != reload {
                // Refresh data without changing the channel scroll position.
                pages = [:]
            } else {
                // A different day/window keeps the same channel rows and offset.
                for offset in Array(pages.keys) {
                    pages[offset]?.programs = [:]
                    pages[offset]?.complete = false
                    pages[offset]?.failure = nil
                }
            }
            loadedIdentity = identity
            loadedChannelScope = channelScope
            loadedReload = reload
        }
        requestPage(anchorOffset)
        for offset in pages.keys where offset != anchorOffset { requestPage(offset) }
    }

    private func rowAppeared(_ index: Int) {
        let offset = (index / Self.pageSize) * Self.pageSize
        anchorOffset = offset
        requestPage(offset)
        let local = index - offset
        if local >= Self.pageSize - 8 && offset + Self.pageSize < total { requestPage(offset + Self.pageSize) }
        if local < 8 && offset > 0 { requestPage(offset - Self.pageSize) }
        let retained = GuidePageRetention.offsets(around: offset, pageSize: Self.pageSize)
        for stale in Array(pages.keys) where !retained.contains(stale) { pages[stale] = nil }
        for stale in Array(pageTasks.keys) where !retained.contains(stale) {
            pageTasks.removeValue(forKey: stale)?.cancel()
            requests[stale] = nil
        }
    }

    private func requestPage(_ offset: Int, retry: Bool = false) {
        guard offset >= 0, pageTasks[offset] == nil, !store.preview, store.session != nil else { return }
        guard retry || (pages[offset]?.complete != true && pages[offset]?.failure == nil) else { return }
        let request = UUID()
        let scope = generation
        requests[offset] = request
        if retry { pages[offset]?.failure = nil }
        pageTasks[offset] = Task { await fetchPage(offset, request: request, scope: scope) }
    }

    private func stopRequests() {
        let pending = Array(pageTasks.values)
        requests = [:]
        pageTasks = [:]
        for task in pending { task.cancel() }
    }

    private func fetchPage(_ offset: Int, request: UUID, scope: UUID) async {
        guard let session = store.session else { return }
        let account = OfflineLibrary.identity(session)
        func current() -> Bool {
            generation == scope && requests[offset] == request && loadedIdentity == identity
                && store.session.map(OfflineLibrary.identity) == account && !Task.isCancelled
        }
        defer {
            if requests[offset] == request {
                pageTasks[offset] = nil
                requests[offset] = nil
            }
        }
        do {
            var channels = pages[offset]?.channels ?? []
            if channels.isEmpty {
                var fields: [String: Any] = ["startIndex": offset, "limit": Self.pageSize]
                if favorites { fields["isFavorite"] = true }
                let response: ItemPage = try await RustBridge.call("liveChannels", session: session, fields: fields)
                guard current() else { return }
                channels = response.Items
                if let reported = response.TotalRecordCount { total = max(0, reported) }
                else if channels.count < Self.pageSize { total = offset + channels.count }
                else { total = max(total, offset + channels.count + Self.pageSize) }
                pages[offset] = ChannelPage(channels: channels)
            }
            guard !channels.isEmpty else { pages[offset]?.complete = true; return }
            let minimum = windowStart.ISO8601Format()
            let maximum = windowStart.addingTimeInterval(Double(hours) * 3600).ISO8601Format()
            var programOffset = 0
            var seen: Set<String> = []
            var programs: [LiveProgram] = []
            while programOffset < 10_000 {
                let response: ProgramPage = try await RustBridge.call("livePrograms", session: session, fields: [
                    "channelIds": channels.map(\.Id), "startIndex": programOffset, "limit": 200,
                    "minEndDate": minimum, "maxStartDate": maximum
                ])
                guard current() else { return }
                programs += response.Items.filter { seen.insert($0.Id).inserted }
                programOffset += response.Items.count
                pages[offset]?.programs = Dictionary(grouping: programs, by: { $0.ChannelId ?? "" })
                if response.Items.isEmpty || (response.TotalRecordCount.map { programOffset >= $0 } ?? (response.Items.count < 200)) {
                    pages[offset]?.complete = true
                    return
                }
            }
            pages[offset]?.complete = true
            pages[offset]?.failure = "This window has more listings. Choose a shorter guide window to see the complete schedule."
        } catch is CancellationError {
        } catch {
            if current() {
                if pages[offset] == nil { pages[offset] = ChannelPage(channels: []) }
                pages[offset]?.failure = error.localizedDescription
            }
        }
    }
}

private struct GuideHorizontalOffset: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

private struct GuideProgramSelection: Identifiable {
    let channel: MediaItem
    let program: LiveProgram
    let accountContext: UUID
    var id: String { program.Id }
}

private struct GuideProgramDetails: View {
    let selection: GuideProgramSelection
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    private var program: LiveProgram { selection.program }
    private var isCurrentAccount: Bool { store.accountContext == selection.accountContext }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(program.title).font(.largeTitle.bold()).accessibilityAddTraits(.isHeader)
                if let episode = program.EpisodeTitle, !episode.isEmpty {
                    Text(episode).font(.title3).foregroundStyle(.secondary)
                }
                Text([selection.channel.Number, selection.channel.title].compactMap { $0 }.joined(separator: " · "))
                    .font(.subheadline).foregroundStyle(.secondary)
                if let start = program.start, let end = program.end {
                    Text("\(start.formatted(date: .abbreviated, time: .shortened)) – \(end.formatted(date: .omitted, time: .shortened))")
                        .font(.subheadline.monospacedDigit())
                }
                if let overview = program.Overview, !overview.isEmpty { Text(overview) }
                Button { Task { if isCurrentAccount { await store.play(selection.channel) } } } label: {
                    Label("Watch channel live", systemImage: "play.fill")
                }
                .buttonStyle(.borderedProminent).tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary)
                .disabled(!isCurrentAccount || store.startingPlayback)
                MediaNavigationLink(destination: .channel(selection.channel)) { Label("Channel schedule", systemImage: "list.bullet") }
                    .disabled(!isCurrentAccount)
            }.padding(24).frame(maxWidth: 760, alignment: .leading).frame(maxWidth: .infinity)
        }
        .background(BluefinBackground())
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
        }
        .task(id: store.accountContext) { if !isCurrentAccount { dismiss() } }
    }
}

enum GuidePageRetention {
    static func offsets(around offset: Int, pageSize: Int) -> Set<Int> {
        guard offset >= 0, pageSize > 0 else { return [] }
        let page = (offset / pageSize) * pageSize
        return Set([page - pageSize, page, page + pageSize].filter { $0 >= 0 })
    }
}

/// Clips server intervals onto one shared timeline; gaps remain honest gaps.
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
