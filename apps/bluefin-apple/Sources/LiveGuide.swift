import SwiftUI

/// Reads the authenticated server's guide. Provider rights and playback policy
/// remain server authority; missing programme data never becomes a fabricated event.
struct LiveGuideView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var channels: [MediaItem] = []
    @State private var programmes: [LiveProgram] = []
    @State private var start = 0
    @State private var total = 0
    @State private var hours = 6
    @State private var windowStart = Date()
    @State private var loading = false
    @State private var failure: String?
    @State private var generation = UUID()
    private let pageSize = 40
    private struct Page: Decodable { let Items: [LiveProgram]; let TotalRecordCount: Int? }
    private var identity: String { "\(store.session.map(OfflineLibrary.identity) ?? "preview")|\(start)|\(hours)|\(windowStart.timeIntervalSince1970)" }
    var body: some View {
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
                    let entries = programmes.filter { $0.ChannelId == channel.Id }.sorted { ($0.start ?? .distantFuture) < ($1.start ?? .distantFuture) }
                    if entries.isEmpty {
                        NavigationLink(destination: LiveChannelView(channel: channel)) {
                            Text(channel.CurrentProgram?.title ?? "No programme information supplied").foregroundStyle(.secondary)
                        }
                    } else {
                        ScrollView(.horizontal, showsIndicators: true) {
                            LazyHStack(alignment: .top, spacing: 12) {
                                ForEach(entries) { programme in
                                    NavigationLink(destination: LiveChannelView(channel: channel)) {
                                        VStack(alignment: .leading, spacing: 8) {
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
                } header: { Text([channel.Number, channel.title].compactMap { $0 }.joined(separator: " · ")) }
            }
            if loading { ProgressView("Loading channel guide") }
            if let failure { Text(failure).foregroundStyle(.secondary); Button("Try again") { windowStart = Date() } }
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
            .bluefinRefresh { windowStart = Date() }
    }
    private func load() async {
        guard let session = store.session else { channels = []; programmes = []; return }
        let account = OfflineLibrary.identity(session); let request = UUID(); generation = request
        loading = true; failure = nil; channels = []; programmes = []
        defer { if generation == request { loading = false } }
        func current() -> Bool { generation == request && store.session.map(OfflineLibrary.identity) == account && !Task.isCancelled }
        do {
            let page: ItemPage = try await RustBridge.call("liveChannels", session: session, fields: ["startIndex": start, "limit": pageSize])
            guard current() else { return }
            channels = page.Items; total = page.TotalRecordCount ?? (start + channels.count)
            guard !channels.isEmpty else { return }
            let formatter = ISO8601DateFormatter(); var offset = 0
            while offset < 10_000 {
                let guide: Page = try await RustBridge.call("livePrograms", session: session, fields: [
                    "channelIds": channels.map(\.Id), "startIndex": offset, "limit": 200,
                    "minEndDate": formatter.string(from: windowStart),
                    "maxStartDate": formatter.string(from: windowStart.addingTimeInterval(Double(hours) * 3600))
                ])
                guard current() else { return }
                let existing = Set(programmes.map(\.Id)); programmes += guide.Items.filter { !existing.contains($0.Id) }
                offset += guide.Items.count
                if guide.Items.isEmpty || offset >= (guide.TotalRecordCount ?? offset) { return }
            }
            failure = "More guide entries are available. Open a channel for its schedule."
        } catch is CancellationError {} catch { if current() { failure = error.localizedDescription } }
    }
}
