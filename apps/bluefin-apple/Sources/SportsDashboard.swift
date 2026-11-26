import SwiftUI

/// Standalone destination for the parent's Sports tab or shortcut.
struct SportsDashboard: View {
    @ObservedObject var sports: SportsStore
    @EnvironmentObject private var store: LibraryStore
    @State private var day = Date()
    @State private var dayOnly = false
    @State private var favoritesOnly = false
    @State private var league = ""
    @State private var zoneID = TimeZone.autoupdatingCurrent.identifier
    private var zone: TimeZone { TimeZone(identifier: zoneID) ?? .autoupdatingCurrent }
    private var games: [SportsOption] {
        SportsSchedule.sorted(sports.feed?.games ?? []).filter { option in
            (league.isEmpty || option.event?.league_name == league) &&
            (!favoritesOnly || sports.saved.map { option.matchesFavorites($0.preferences) } == true) &&
            (!dayOnly || option.scheduleDate.map { SportsSchedule.sameDay($0, as: day, timeZone: zone) } == true)
        }
    }
    private var leagues: [String] { Set(sports.feed?.games.compactMap { $0.event?.league_name } ?? []).sorted() }
    var body: some View {
        List {
            Section {
                Toggle("Favorites only", isOn: $favoritesOnly)
                Picker("League", selection: $league) {
                    Text("All leagues").tag("")
                    ForEach(leagues, id: \.self) { Text($0).tag($0) }
                }
                scheduleFilters
            } footer: {
                Text("Dates use \(zone.identifier). All dates shows every game returned by the server, including games without a start time.")
            }
            scheduleResults
        }
        .environment(\.timeZone, zone)
        .navigationTitle("Sports")
        .task(id: store.session.map(OfflineLibrary.identity)) { await sports.connect(store.session) }
        .bluefinRefresh { await sports.connect(store.session) }
        .onChange(of: sports.accountGeneration) { _ in favoritesOnly = false; league = ""; dayOnly = false; zoneID = TimeZone.autoupdatingCurrent.identifier }
    }
    @ViewBuilder private var scheduleFilters: some View {
        #if os(iOS)
        DisclosureGroup("Schedule filters") { scheduleControls }
        #else
        scheduleControls
        #endif
    }
    private var scheduleControls: some View {
        Group {
                Toggle("Choose a day", isOn: $dayOnly)
                if dayOnly {
                    HStack {
                        Button { moveDay(-1) } label: { Label("Previous day", systemImage: "chevron.left") }.labelStyle(.iconOnly).frame(minWidth: 44, minHeight: 44)
                        Spacer()
                        Text(day, style: .date).font(.headline).environment(\.timeZone, zone)
                            .accessibilityLabel("Selected day").accessibilityValue(Text(day, style: .date))
                        Spacer()
                        Button { moveDay(1) } label: { Label("Next day", systemImage: "chevron.right") }.labelStyle(.iconOnly).frame(minWidth: 44, minHeight: 44)
                    }
                    Button("Today") { day = Date() }
                }
                Picker("Schedule time zone", selection: $zoneID) {
                    ForEach(TimeZone.knownTimeZoneIdentifiers, id: \.self) { Text($0.replacingOccurrences(of: "_", with: " ")).tag($0) }
                }
        }
    }
    private var scheduleResults: some View {
        Group {
            if sports.loading { ProgressView("Refreshing scores") }
            if let failure = sports.failure {
                Section {
                    Label(failure, systemImage: "wifi.exclamationmark")
                    if sports.feed != nil { Text("Showing the last received scores. They may have changed.").font(.footnote) }
                    Button("Try again") { Task { await sports.connect(store.session) } }
                }
            }
            if let updated = sports.lastUpdated {
                Text("Updated \(SportsSchedule.label(updated, timeZone: zone))").font(.caption).foregroundStyle(.secondary)
            }
            ForEach(SportsGameStatus.allCases) { status in
                let entries = games.filter { $0.status == status }
                if !entries.isEmpty {
                    Section(status.label) { ForEach(entries) { SportsBroadcastRow(option: $0) } }
                }
            }
            if games.isEmpty && !sports.loading {
                Section {
                    Text("No matching games in the current schedule.")
                    Text("Try all dates or all leagues. Your favorites are kept.").font(.footnote).foregroundStyle(.secondary)
                }
            }
            if let feed = sports.feed, !feed.local_broadcasts.isEmpty {
                Section {
                    Text("Channel guide listings can include sports coverage without a verified game identity.").font(.footnote).foregroundStyle(.secondary)
                    ForEach(SportsSchedule.sorted(feed.local_broadcasts)) { SportsBroadcastRow(option: $0) }
                } header: { Text("Sports on your channels") }
            }
            SportsProviderSection(providers: sports.feed?.providers ?? [])
            Section { NavigationLink("Favorites and preferences", destination: SportsSettingsView(sports: sports)) }
        }
    }
    private func moveDay(_ amount: Int) {
        day = SportsSchedule.calendar(timeZone: zone).date(byAdding: .day, value: amount, to: day) ?? day
    }
}

struct SportsProviderSection: View {
    var providers: [SportsFeed.Provider]
    var body: some View {
        Section {
            ForEach(providers.filter { $0.kind == "STARTING_POINT" && $0.action == "CHECK_ACCESS" }) { provider in
                if let destination = ProviderDestination.validated(name: provider.name, rawURL: provider.url) {
                    ProviderHandoffButton(destination: destination, label: "Check \(provider.name) access")
                }
            }
        } header: { Text("Provider starting points") } footer: {
            Text("These open a provider, not this game. Availability depends on your subscription, region and blackout rules.")
        }
    }
}

struct SportsWatchSheet: View {
    let option: SportsOption
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var store: LibraryStore
    var body: some View {
        NavigationView {
            List {
                Section {
                    Text(option.displayTitle).font(.headline)
                    Text(option.broadcastLabel)
                    if let names = option.event?.broadcasters, !names.isEmpty {
                        Label(names.joined(separator: ", "), systemImage: "tv")
                    }
                    if option.canOpenChannel, let id = option.channel_id {
                        NavigationLink("View \(option.channel_name ?? "channel") and schedule", destination: LiveChannelView(channel: MediaItem(Id: id, Name: option.channel_name ?? "Live channel", Type: "TvChannel")))
                    }
                    Text("No verified provider app link was supplied for this event.").foregroundStyle(.secondary)
                } footer: {
                    Text("A channel listing or broadcaster name does not confirm playback rights. Open the channel to check its current schedule and your account's access.")
                }
                SportsProviderSection(providers: store.sports.feed?.providers ?? [])
            }.navigationTitle("Where to watch")
                .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }.navigationViewStyle(.stack)
    }
}
