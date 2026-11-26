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
            NavigationLink(destination: filters) {
                Label("Schedule filters", systemImage: "slider.horizontal.3")
            }
            scheduleResults
            SportsProviderSection(providers: sports.feed?.providers ?? [])
            Section {
                NavigationLink("Favorites and preferences", destination: SportsSettingsView(sports: sports).bluefinBackNavigation())
            }
        }
        .environment(\.timeZone, zone)
        .navigationTitle("Sports")
        .task(id: store.session.map(OfflineLibrary.identity)) { await sports.connect(store.session) }
        .bluefinRefresh { await sports.connect(store.session) }
        .onChange(of: sports.accountGeneration) { _ in favoritesOnly = false; league = ""; dayOnly = false; zoneID = TimeZone.autoupdatingCurrent.identifier }
    }

    private var filters: some View {
        Form {
            Section {
                Toggle("Favorites only", isOn: $favoritesOnly)
                Picker("League", selection: $league) {
                    Text("All leagues").tag("")
                    ForEach(leagues, id: \.self) { Text($0).tag($0) }
                }
                scheduleControls
            } footer: {
                Text("Dates use \(zone.identifier). All dates shows every game returned by the server, including games without a start time.")
            }
        }
        .navigationTitle("Schedule filters")
        .bluefinBackNavigation()
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
        }
    }
    private func moveDay(_ amount: Int) {
        day = SportsSchedule.calendar(timeZone: zone).date(byAdding: .day, value: amount, to: day) ?? day
    }
}

struct SportsProviderSection: View {
    var providers: [SportsFeed.Provider]
    var body: some View {
        let destinations = providers.compactMap { provider -> ProviderDestination? in
            guard provider.kind == "STARTING_POINT", provider.action == "CHECK_ACCESS" else { return nil }
            return ProviderDestination.validated(name: provider.name, rawURL: provider.url)
        }
        if !destinations.isEmpty {
            Section("Provider shortcuts") {
                ForEach(destinations) { destination in
                    ProviderHandoffButton(destination: destination, label: "Check \(destination.name) access")
                }
            }
        }
    }
}

struct SportsWatchSheet: View {
    let option: SportsOption
    @Environment(\.dismiss) private var dismiss
    @Environment(\.bluefinShowHome) private var showHome
    @EnvironmentObject private var store: LibraryStore
    @State private var listingChanged = false

    private var broadcasterDestinations: [ProviderDestination] {
        let names = option.event?.broadcasters ?? []
        return ProviderDestination.common.filter { destination in
            names.contains { $0.caseInsensitiveCompare(destination.name) == .orderedSame }
        }
    }

    var body: some View {
        NavigationView {
            List {
                Section {
                    Text(option.displayTitle).font(.headline)
                    Text(option.broadcastLabel)
                    if let names = option.event?.broadcasters, !names.isEmpty {
                        Label(names.joined(separator: ", "), systemImage: "tv")
                    }
                }
                if option.canOpenChannel, let id = option.channel_id {
                    let channel = MediaItem(Id: id, Name: option.channel_name ?? "Live channel", Type: "TvChannel")
                    Section("Your live TV") {
                        Text(channel.title).font(.headline)
                        if !listingChanged && option.isAiring(on: Date()) {
                            Button {
                                guard option.isAiring(on: Date()) else { listingChanged = true; return }
                                dismiss()
                                Task { await store.play(channel) }
                            } label: { Label("Watch channel", systemImage: "play.fill") }
                                .disabled(store.startingPlayback)
                        }
                        if listingChanged { Text("Check the channel schedule for the current program.").foregroundStyle(.secondary) }
                        NavigationLink("View channel schedule", destination: LiveChannelView(channel: channel))
                    }
                }
                if !broadcasterDestinations.isEmpty {
                    Section {
                        ForEach(broadcasterDestinations) { destination in
                            ProviderHandoffButton(destination: destination, label: "Open \(destination.name)")
                        }
                    } header: {
                        Text("Broadcaster apps")
                    } footer: {
                        Text("Check the provider for this game and your subscription access.")
                    }
                }
                SportsProviderSection(providers: store.sports.feed?.providers ?? [])
            }.navigationTitle("Where to watch")
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                    if let showHome {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Home") { dismiss(); showHome() }.accessibilityIdentifier("bluefinHome")
                        }
                    }
                }
        }.navigationViewStyle(.stack)
    }
}
