import SwiftUI

struct LiveTVView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var channels: [MediaItem] = []
    @State private var start = 0
    @State private var total = 0
    @State private var favorites = false
    @State private var loading = false
    @State private var failure: String?
    @State private var request = UUID()
    private var identity: String { "\(store.session?.server ?? "")|\(store.session?.userId ?? "")|\(favorites)" }
    var body: some View {
        List {
            Section {
                NavigationLink(destination: LiveGuideView()) { Label("Channel guide", systemImage: "rectangle.grid.1x2") }
                if store.sports.available { NavigationLink(destination: NativeSportsGuide(sports: store.sports)) { Label("Sports", systemImage: "sportscourt") } }
            }
            Section {
                Toggle("Favorites", isOn: $favorites)
            }
            Section("On now") {
                ForEach(channels) { channel in
                    NavigationLink(destination: LiveChannelView(channel: channel)) {
                        HStack(spacing: 16) {
                            Artwork(item: channel).frame(width: 96, height: 64).clipShape(RoundedRectangle(cornerRadius: 9))
                            VStack(alignment: .leading, spacing: 6) {
                                Text(channel.CurrentProgram?.title ?? channel.title).font(.headline)
                                Text([channel.Number, channel.title].compactMap { $0 }.joined(separator: " · ")).font(.subheadline).foregroundStyle(.secondary)
                                if let programme = channel.CurrentProgram, let begin = programme.start, let end = programme.end, end > begin {
                                    TimelineView(.periodic(from: .now, by: 30)) { timeline in
                                        ProgressView(value: min(1, max(0, timeline.date.timeIntervalSince(begin) / end.timeIntervalSince(begin)))).accessibilityLabel("Programme progress")
                                    }
                                }
                            }
                        }.padding(.vertical, 7)
                    }
                }
            }
            if loading { ProgressView("Loading channels") }
            if let failure { Text(failure).foregroundStyle(.secondary); Button("Try again") { Task { await load(reset: channels.isEmpty) } } }
            if !loading && channels.isEmpty && failure == nil { Text("No live channels are available to this account.").foregroundStyle(.secondary) }
            if !loading && start < total { Button("Load more channels") { Task { await load(reset: false) } } }
        }.navigationTitle("Live TV")
        .task(id: identity) { await load(reset: true) }
        .bluefinRefresh { await load(reset: true) }
    }
    private func load(reset: Bool) async {
        guard let session = store.session else { return }
        let requested = identity
        let operation = UUID()
        request = operation
        loading = true; failure = nil
        defer { if request == operation { loading = false } }
        do {
            var fields: [String: Any] = ["startIndex": reset ? 0 : start, "limit": 60]
            if favorites { fields["isFavorite"] = true }
            let page: ItemPage = try await RustBridge.call("liveChannels", session: session, fields: fields)
            try Task.checkCancellation()
            guard identity == requested, request == operation else { return }
            if reset { channels = page.Items }
            else { let existing = Set(channels.map(\.Id)); channels += page.Items.filter { !existing.contains($0.Id) } }
            start = (reset ? 0 : start) + page.Items.count
            total = page.TotalRecordCount ?? start
        } catch is CancellationError {} catch { if request == operation { failure = error.localizedDescription } }
    }
}

struct NativeSportsGuide: View {
    @ObservedObject var sports: SportsStore
    var body: some View {
        List {
            if let feed = sports.feed {
                Section("Your games") {
                    ForEach(feed.games) { SportsBroadcastRow(option: $0).listRowInsets(EdgeInsets()) }
                    if feed.games.isEmpty { Text("No games match your current selections.").foregroundStyle(.secondary) }
                }
                if !feed.local_broadcasts.isEmpty {
                    Section("Sports on your channels") {
                        Text("Guide coverage can include sports shows rather than a verified game.").font(.footnote).foregroundStyle(.secondary)
                        ForEach(feed.local_broadcasts) { SportsBroadcastRow(option: $0).listRowInsets(EdgeInsets()) }
                    }
                }
                if let providers = feed.providers, !providers.isEmpty {
                    Section {
                        ForEach(providers.filter { $0.kind == "STARTING_POINT" && $0.action == "CHECK_ACCESS" }) { provider in
                            if let destination = ProviderDestination.validated(name: provider.name, rawURL: provider.url) {
                                ProviderHandoffButton(destination: destination, label: "Check \(provider.name) access")
                            }
                        }
                    } header: { Text("Where to watch") } footer: {
                        Text("Provider starting points do not confirm access to a particular game. Your subscription, region and blackout rules still apply.")
                    }
                }
            }
        }.navigationTitle("Sports")
            .toolbar { ToolbarItem(placement: .primaryAction) { NavigationLink(destination: SportsSettingsView(sports: sports)) { Image(systemName: "line.3.horizontal.decrease.circle") }.accessibilityLabel("Sports preferences") } }
    }
}

struct LiveChannelView: View {
    let channel: MediaItem
    @EnvironmentObject private var store: LibraryStore
    @State private var programmes: [LiveProgram] = []
    @State private var failure: String?
    private struct Page: Decodable { var Items: [LiveProgram] }
    var body: some View {
        List {
            Section {
                Artwork(item: channel).aspectRatio(16 / 9, contentMode: .fit).clipShape(RoundedRectangle(cornerRadius: 14))
                Text(channel.title).font(.largeTitle.bold())
                Button { Task { await store.play(channel) } } label: { Label("Watch live", systemImage: "play.fill") }
                    .buttonStyle(.borderedProminent).disabled(store.startingPlayback)
            }
            Section("Schedule") {
                ForEach(programmes) { programme in
                    VStack(alignment: .leading, spacing: 8) {
                        if let start = programme.start, let end = programme.end { Text("\(start.formatted(date: .omitted, time: .shortened)) – \(end.formatted(date: .omitted, time: .shortened))").font(.caption.monospacedDigit()).foregroundStyle(.secondary) }
                        Text(programme.title).font(.headline)
                        if programme.IsSports == true { Label("Sports", systemImage: "sportscourt").font(.caption).foregroundStyle(.secondary) }
                        if let episode = programme.EpisodeTitle { Text(episode).font(.subheadline) }
                        if let overview = programme.Overview { Text(overview).font(.callout).foregroundStyle(.secondary) }
                    }.padding(.vertical, 8)
                }
                if programmes.isEmpty && failure == nil { Text("No programme information is available.").foregroundStyle(.secondary) }
                if let failure { Text(failure).foregroundStyle(.secondary) }
            }
        }.navigationTitle(channel.title)
        .task(id: "\(store.session?.server ?? "")|\(store.session?.userId ?? "")|\(channel.Id)") {
            guard let session = store.session else { return }
            let identity = OfflineLibrary.identity(session)
            do {
                let page: Page = try await RustBridge.call("livePrograms", session: session, fields: ["channelIds": [channel.Id], "minEndDate": ISO8601DateFormatter().string(from: Date()), "limit": 40])
                try Task.checkCancellation()
                guard store.session.map(OfflineLibrary.identity) == identity else { return }
                programmes = page.Items
            } catch is CancellationError {} catch { if store.session.map(OfflineLibrary.identity) == identity { failure = error.localizedDescription } }
        }
    }
}
