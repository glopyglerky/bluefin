import SwiftUI

struct SportsPreferences: Codable {
    var schema: Int
    var sports: Set<String>
    var leagues: Set<String>
    var conferences: Set<String>
    var teams: Set<String>
    var players: Set<String>
    var all_conferences: Bool
    var all_teams: Bool
    var all_players: Bool
    var home: Bool
    var away: Bool
    var native_home: Bool
    var auto_record: Bool
    var sports_only: Bool
    var completed: Bool
}

struct SportsSaved: Decodable { var version: UInt64; var preferences: SportsPreferences; var etag: String }

struct SportsOption: Decodable, Identifiable {
    struct Event: Decodable { var name: String; var league_name: String?; var date: String? }
    var kind: String
    var item_id: String
    var channel_id: String?
    var channel_name: String?
    var event: Event?
    var title: String?
    var native_title: String?
    var native_start: String?
    var native_end: String?
    var start: String?
    var end: String?
    var provenance: String?
    var recording_status: String?
    var playback_verified: Bool?
    var id: String { item_id + ":" + kind }
    var displayTitle: String { event?.name ?? title ?? native_title ?? "Sports broadcast" }
    var stateLabel: String {
        switch kind {
        case "LIVE": return "Live event"
        case "ON_NOW": return "On now"
        case "UPCOMING": return "Upcoming"
        case "RECORDING": return "Recording"
        default: return "Status unavailable"
        }
    }
    var canOpenChannel: Bool { ["LIVE", "ON_NOW", "UPCOMING"].contains(kind) && channel_id != nil }
}

struct SportsFeed: Decodable {
    var schema: Int
    var sports_only: Bool
    var games: [SportsOption]
    var local_broadcasts: [SportsOption]
    var presentation_only: Bool
}

@MainActor final class SportsStore: ObservableObject {
    @Published private(set) var feed: SportsFeed?
    @Published private(set) var saved: SportsSaved?
    @Published private(set) var loading = false
    @Published private(set) var available = false
    @Published var failure: String?
    private var identity: String?
    private var generation = UUID()
    func disconnect() {
        generation = UUID(); identity = nil
        feed = nil; saved = nil; failure = nil; available = false; loading = false
    }
    func connect(_ session: AccountSession?) async {
        let next = session.map(OfflineLibrary.identity)
        if identity != next {
            generation = UUID(); identity = next
            feed = nil; saved = nil; failure = nil; available = false; loading = false
        }
        guard let session, !loading else { return }
        let current = generation
        loading = true
        defer { if generation == current { loading = false } }
        do {
            let preferences: SportsSaved = try await RustBridge.call("sportsPreferences", session: session)
            try Task.checkCancellation()
            guard generation == current else { return }
            saved = preferences; available = true
            let home: SportsFeed = try await RustBridge.call("sportsHome", session: session)
            try Task.checkCancellation()
            guard generation == current else { return }
            saved = preferences; feed = home; available = true; failure = nil
        } catch is CancellationError {} catch {
            guard generation == current else { return }
            if error.localizedDescription.contains("HTTP 404") { available = false; feed = nil; saved = nil }
            else { failure = error.localizedDescription }
        }
    }
    func save(_ preferences: SportsPreferences, session: AccountSession) async throws {
        guard let saved, identity == OfflineLibrary.identity(session) else { throw BluefinError.message("Reload your sports preferences before saving.") }
        let current = generation
        let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(preferences))
        let updated: SportsSaved = try await RustBridge.call("saveSportsPreferences", session: session, fields: ["etag": saved.etag, "preferences": value])
        try Task.checkCancellation()
        guard generation == current else { throw CancellationError() }
        self.saved = updated
        await connect(session)
    }
}

struct SportsHomeSection: View {
    @ObservedObject var sports: SportsStore
    var body: some View {
        if let feed = sports.feed {
            VStack(alignment: .leading, spacing: 18) {
                HStack {
                    Label("Sports", systemImage: "sportscourt").font(.title2.bold())
                    Spacer()
                    NavigationLink(destination: SportsSettingsView(sports: sports)) { Image(systemName: "slider.horizontal.3").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Sports preferences")
                }.padding(.horizontal, 24)
                if feed.games.isEmpty && feed.local_broadcasts.isEmpty {
                    Text("No broadcasts match your current selections. Your existing preferences are kept.").foregroundStyle(.secondary).padding(.horizontal, 24)
                }
                ForEach(Array(feed.games.prefix(8))) { item in SportsBroadcastRow(option: item).padding(.horizontal, 24) }
                if feed.games.count > 8 { NavigationLink("All matched events", destination: SportsBroadcastList(title: "Matched events", options: feed.games)).padding(.horizontal, 24) }
                if !feed.local_broadcasts.isEmpty {
                    Text("Sports on your channels").font(.headline).padding(.horizontal, 24)
                    Text("Guide listings. An on-now broadcast may include general coverage rather than a verified game.").font(.footnote).foregroundStyle(.secondary).padding(.horizontal, 24)
                    ForEach(Array(feed.local_broadcasts.prefix(8))) { item in SportsBroadcastRow(option: item).padding(.horizontal, 24) }
                    if feed.local_broadcasts.count > 8 { NavigationLink("All sports broadcasts", destination: SportsBroadcastList(title: "Sports broadcasts", options: feed.local_broadcasts)).padding(.horizontal, 24) }
                }
            }
        }
    }
}

private struct SportsBroadcastList: View {
    let title: String
    let options: [SportsOption]
    var body: some View { List(options) { SportsBroadcastRow(option: $0) }.navigationTitle(title) }
}

private struct SportsBroadcastRow: View {
    let option: SportsOption
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(option.stateLabel).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                if let league = option.event?.league_name { Text(league).font(.caption).foregroundStyle(.secondary) }
            }
            Text(option.displayTitle).font(.headline)
            if let channel = option.channel_name { Text(channel).font(.subheadline).foregroundStyle(.secondary) }
            if let status = option.recording_status { Text(status).font(.caption).foregroundStyle(.secondary) }
            if option.canOpenChannel, let id = option.channel_id {
                NavigationLink(destination: LiveChannelView(channel: MediaItem(Id: id, Name: option.channel_name ?? "Live channel", Type: "TvChannel"))) {
                    Label(option.kind == "UPCOMING" ? "View channel and schedule" : "Open channel", systemImage: "tv")
                }.buttonStyle(.bordered)
            } else if option.kind == "RECORDING" {
                NavigationLink(destination: DetailView(item: MediaItem(Id: option.item_id, Name: option.displayTitle, Type: "Recording"))) { Label("Open recording", systemImage: "play.rectangle") }.buttonStyle(.bordered)
            }
        }.padding(18).frame(maxWidth: .infinity, alignment: .leading).background(BluefinStyle.secondaryBackground, in: RoundedRectangle(cornerRadius: 16))
    }
}

struct SportsSettingsView: View {
    @ObservedObject var sports: SportsStore
    @EnvironmentObject private var store: LibraryStore
    @State private var preferences: SportsPreferences?
    @State private var saving = false
    @State private var failure: String?
    var body: some View {
        Form {
            if let preferences {
                Section {
                    Toggle("Sports Home", isOn: boolean(\.sports_only))
                    Toggle("Include native Home", isOn: boolean(\.native_home))
                    Toggle("Home games", isOn: boolean(\.home))
                    Toggle("Away games", isOn: boolean(\.away))
                } footer: { Text("These choices change your sports presentation. Your server's account and media permissions still apply.") }
                Section("Current selections") {
                    LabeledContentCompat("Sports", value: "\(preferences.sports.count)")
                    LabeledContentCompat("Leagues", value: "\(preferences.leagues.count)")
                    LabeledContentCompat("Teams", value: preferences.all_teams ? "All" : "\(preferences.teams.count) selected")
                    LabeledContentCompat("Conferences", value: preferences.all_conferences ? "All" : "\(preferences.conferences.count) selected")
                    LabeledContentCompat("Players", value: preferences.all_players ? "All" : "\(preferences.players.count) selected")
                    Text("Existing league, team, conference and player selections are preserved.").font(.footnote).foregroundStyle(.secondary)
                }
                Button(saving ? "Saving…" : "Save preferences") {
                    guard let session = store.session, let updated = self.preferences else { return }
                    saving = true; failure = nil
                    Task {
                        defer { saving = false }
                        do { try await sports.save(updated, session: session); self.preferences = sports.saved?.preferences }
                        catch { failure = error.localizedDescription }
                    }
                }.disabled(saving || (!preferences.home && !preferences.away))
            } else { ProgressView("Loading preferences") }
            if let failure { Text(failure).foregroundStyle(.secondary) }
            Button("Reload preferences") { Task { await sports.connect(store.session); preferences = sports.saved?.preferences } }.disabled(saving)
        }.navigationTitle("Sports preferences")
        .task { preferences = sports.saved?.preferences }
    }
    private func boolean(_ key: WritableKeyPath<SportsPreferences, Bool>) -> Binding<Bool> {
        Binding(get: { preferences?[keyPath: key] ?? false }, set: { preferences?[keyPath: key] = $0 })
    }
}
