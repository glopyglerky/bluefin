import SwiftUI

@MainActor final class SportsScoreStore: ObservableObject {
    @Published private(set) var board: SportsScoreboard?
    @Published private(set) var loading = false
    @Published private(set) var failure: String?
    private var key: String?
    private var request = UUID()

    func load(store: LibraryStore, league: SportsScoreLeague, date: Date) async {
        let context = store.accountContext
        let next = "\(context)|\(league.rawValue)|\(SportsScoreDate.key(date))"
        if key != next { board = nil; failure = nil; key = next }
        let current = UUID(); request = current; loading = true
        defer { if request == current { loading = false } }
        do {
            let value: SportsScoreboard = try await store.authenticated("sportsScoreboard",
                fields: ["league": league.rawValue, "date": SportsScoreDate.key(date)])
            try Task.checkCancellation()
            guard request == current, store.accountContext == context else { return }
            guard value.schema == 1, value.provider == "ESPN", value.league == league.rawValue,
                  value.events.allSatisfy({ $0.league == league.rawValue }) else {
                throw BluefinError.message("The server returned a different sports schedule.")
            }
            board = value; failure = nil
        } catch is CancellationError {} catch {
            if request == current, store.accountContext == context { failure = error.localizedDescription }
        }
    }
}

private enum SportsDay: String, CaseIterable, Identifiable {
    case yesterday = "Yesterday", today = "Today", upcoming = "Upcoming"
    var id: String { rawValue }
}
private enum SportsDetailTab: String, CaseIterable, Identifiable {
    case stats = "Stats", plays = "Play-by-Play", standings = "Standings"
    var id: String { rawValue }
}

/// Scores are independent of live TV access. Only the existing guide can authorize playback.
struct SportsDashboard: View {
    @ObservedObject var sports: SportsStore
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.bluefinCategoryIsActive) private var active
    @StateObject private var scores = SportsScoreStore()
    @State private var league = SportsScoreLeague.nhl
    @State private var dayOffset = 0
    @State private var favoritesOnly = false
    @State private var selected: SportsScoreEvent?
    private var date: Date { Calendar.autoupdatingCurrent.date(byAdding: .day, value: dayOffset, to: Date()) ?? Date() }
    private var selection: Binding<SportsDay> {
        Binding(get: { dayOffset < 0 ? .yesterday : dayOffset == 0 ? .today : .upcoming },
                set: { dayOffset = $0 == .yesterday ? -1 : $0 == .today ? 0 : 1 })
    }
    private var events: [SportsScoreEvent] {
        (scores.board?.events ?? []).filter { event in
            !favoritesOnly || event.home.isFollowed(league: event.league, preferences: sports.saved?.preferences)
                || event.away.isFollowed(league: event.league, preferences: sports.saved?.preferences)
        }
    }
    private var refreshKey: String {
        "\(store.accountContext)|\(league.rawValue)|\(SportsScoreDate.key(date))|\(active)|\(scenePhase == .active)"
    }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 20) {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(SportsScoreLeague.allCases) { item in
                            Button { league = item } label: {
                                Text(item.title).font(.subheadline.weight(.semibold)).padding(.horizontal, 18).padding(.vertical, 12)
                                    .background(league == item ? BluefinStyle.text.opacity(0.15) : .clear, in: Capsule())
                            }.buttonStyle(.plain)
                                .accessibilityAddTraits(league == item ? .isSelected : [])
                        }
                    }
                }
                VStack(spacing: 0) {
                    VStack(spacing: 16) {
                        Picker("Schedule", selection: selection) {
                            ForEach(SportsDay.allCases) { Text($0.rawValue).tag($0) }
                        }.pickerStyle(.segmented)
                        HStack {
                            Button { dayOffset = max(-365, dayOffset - 1) } label: {
                                Image(systemName: "chevron.left").frame(width: 44, height: 44)
                            }.accessibilityLabel("Previous day")
                            Spacer()
                            Text(date, style: .date).font(.headline)
                            Spacer()
                            Button { dayOffset = min(365, dayOffset + 1) } label: {
                                Image(systemName: "chevron.right").frame(width: 44, height: 44)
                            }.accessibilityLabel("Next day")
                        }.buttonStyle(.plain)
                        HStack {
                            Button { favoritesOnly.toggle() } label: {
                                Label("My Teams", systemImage: favoritesOnly ? "star.fill" : "star").font(.headline)
                            }.buttonStyle(.plain).accessibilityValue(favoritesOnly ? "Only followed teams" : "All teams")
                            Spacer()
                            NavigationLink(destination: SportsSettingsView(sports: sports)) {
                                Image(systemName: "slider.horizontal.3").frame(width: 44, height: 44)
                            }.accessibilityLabel("Sports preferences")
                        }
                    }.padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 6)
                    ForEach(Array(events.enumerated()), id: \.element.id) { index, event in
                        if index > 0 { Divider().padding(.horizontal, 20) }
                        Button { selected = event } label: {
                            SportsScoreRow(event: event, preferences: sports.saved?.preferences)
                        }.modifier(MusicContentLinkStyle())
                    }
                    if scores.loading && scores.board == nil {
                        ProgressView("Loading scores").frame(maxWidth: .infinity).padding(24)
                    }
                    if let failure = scores.failure {
                        VStack(alignment: .leading, spacing: 12) {
                            Label("Scores could not refresh", systemImage: "wifi.exclamationmark").font(.headline)
                            Text(failure).font(.callout).foregroundStyle(.secondary)
                            Button("Try again") { Task { await scores.load(store: store, league: league, date: date) } }
                        }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
                    } else if events.isEmpty && !scores.loading {
                        MusicEmptyState(title: favoritesOnly ? "No games for your teams" : "No games on this day",
                            message: favoritesOnly ? "Choose All teams or another date. Follow teams from a game." : "Choose another league or date.")
                    }
                }
                .padding(.bottom, 8)
                .background(BluefinStyle.secondaryBackground, in: RoundedRectangle(cornerRadius: 30, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 30).strokeBorder(BluefinStyle.text.opacity(0.10), lineWidth: 0.5))
                if let board = scores.board {
                    HStack {
                        Text(board.stale || scores.failure != nil ? "Last available scores" : "ESPN")
                        Spacer()
                        if let updated = SportsScoreDate.parse(board.fetchedAt) { Text(updated, style: .time) }
                    }.font(.caption).foregroundStyle(.secondary)
                }
                NavigationLink(destination: SportsGuideDashboard(sports: sports)) {
                    Label("Sports on your live TV", systemImage: "tv").font(.headline)
                        .frame(maxWidth: .infinity, minHeight: 52)
                }.buttonStyle(.bordered)
            }.padding(20).frame(maxWidth: 960).frame(maxWidth: .infinity)
        }
        .background(BluefinBackground()).navigationTitle("Sports")
        .modifier(MusicNavigationTitle(large: true))
        .sheet(item: $selected) { event in SportsEventView(event: event, sports: sports).environmentObject(store) }
        .task(id: store.accountContext) { await sports.connect(store.session) }
        .task(id: refreshKey) {
            guard active, scenePhase == .active, !store.preview else { return }
            repeat {
                await scores.load(store: store, league: league, date: date)
                do { try await Task.sleep(nanoseconds: 30_000_000_000) } catch { return }
            } while !Task.isCancelled
        }
        .bluefinRefresh { await scores.load(store: store, league: league, date: date) }
        .onChange(of: store.accountContext) { _ in selected = nil; favoritesOnly = false }
    }
}

private struct SportsScoreLogo: View {
    let team: SportsScoreTeam
    @EnvironmentObject private var store: LibraryStore
    @State private var image: UIImage?
    @State private var loadedKey: String?
    private var key: String { "\(store.accountContext)|\(team.logo ?? "")" }
    var body: some View {
        Group {
            if loadedKey == key, let image { Image(uiImage: image).resizable().scaledToFit() }
            else {
                Text(team.abbreviation ?? String(team.name.prefix(3))).font(.caption.bold())
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(BluefinStyle.text.opacity(0.08), in: Circle())
            }
        }
        .accessibilityHidden(true)
        .task(id: key) {
            image = nil; loadedKey = nil
            guard let session = store.session else { return }
            let expected = key
            let result = await Artwork.nativeSportsLogo(resource: team.logo, session: session)
            guard !Task.isCancelled, key == expected else { return }
            image = result; loadedKey = expected
        }
    }
}

private struct SportsScoreRow: View {
    let event: SportsScoreEvent
    let preferences: SportsPreferences?
    var body: some View {
        VStack(spacing: 14) {
            Text(event.league.uppercased()).font(.caption.bold()).foregroundStyle(.secondary)
            HStack(alignment: .center, spacing: 10) {
                team(event.away, trailing: false)
                VStack(spacing: 6) {
                    if event.isScheduled, let date = event.dateValue {
                        Text(date, style: .time).font(.headline.monospacedDigit()).lineLimit(1)
                    } else {
                        HStack(spacing: 8) {
                            Text(event.away.score ?? "—")
                            Text("·").foregroundStyle(.secondary)
                            Text(event.home.score ?? "—")
                        }.font(.title2.bold().monospacedDigit())
                    }
                    if event.isLive {
                        Label("Live", systemImage: "circle.fill").font(.caption2.bold()).foregroundStyle(BluefinStyle.accent)
                        if let clock = event.status.clock { Text(clock).font(.caption.monospacedDigit()) }
                    } else if !event.isScheduled {
                        Text(event.statusLabel).font(.caption).foregroundStyle(.secondary)
                    }
                }.frame(maxWidth: .infinity)
                team(event.home, trailing: true)
            }
        }.padding(.horizontal, 20).padding(.vertical, 22).frame(maxWidth: .infinity)
            .contentShape(Rectangle()).accessibilityElement(children: .combine)
    }
    private func team(_ team: SportsScoreTeam, trailing: Bool) -> some View {
        VStack(alignment: trailing ? .trailing : .leading, spacing: 10) {
            HStack(spacing: 7) {
                if trailing, let record = team.record {
                    Text(record).font(.caption.weight(.semibold)).foregroundStyle(.secondary).lineLimit(1)
                        .minimumScaleFactor(0.8)
                }
                SportsScoreLogo(team: team).frame(width: logoSize, height: logoSize)
                if !trailing, let record = team.record {
                    Text(record).font(.caption.weight(.semibold)).foregroundStyle(.secondary).lineLimit(1)
                        .minimumScaleFactor(0.8)
                }
            }.frame(maxWidth: .infinity, alignment: trailing ? .trailing : .leading)
            HStack(spacing: 4) {
                if !trailing && team.isFollowed(league: event.league, preferences: preferences) {
                    Image(systemName: "star.fill").font(.caption2)
                }
                Text(team.displayName).font(.subheadline.weight(.semibold)).lineLimit(2)
                if trailing && team.isFollowed(league: event.league, preferences: preferences) {
                    Image(systemName: "star.fill").font(.caption2)
                }
            }
            .frame(maxWidth: .infinity, alignment: trailing ? .trailing : .leading)
            .multilineTextAlignment(trailing ? .trailing : .leading)
        }.frame(maxWidth: .infinity)
    }
    private var logoSize: CGFloat {
        #if os(tvOS)
        64
        #else
        42
        #endif
    }
}

private struct SportsTeamBackdrop: View {
    let home: String?
    let away: String?
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    var body: some View {
        ZStack {
            BluefinStyle.secondaryBackground
            if !reduceTransparency {
                LinearGradient(colors: [color(away).opacity(0.22), .clear, color(home).opacity(0.22)],
                    startPoint: .topLeading, endPoint: .bottomTrailing)
            }
        }
    }
    private func color(_ raw: String?) -> Color {
        guard let raw, raw.utf8.count == 6, raw.utf8.allSatisfy({ (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0) }),
              let value = UInt32(raw, radix: 16) else { return BluefinStyle.brand }
        return Color(red: Double((value >> 16) & 255) / 255, green: Double((value >> 8) & 255) / 255, blue: Double(value & 255) / 255)
    }
}

private struct SportsEventView: View {
    let event: SportsScoreEvent
    @ObservedObject var sports: SportsStore
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @State private var detail: SportsScoreDetail?
    @State private var standings: SportsScoreStandings?
    @State private var tab = SportsDetailTab.stats
    @State private var keyPlays = true
    @State private var loading = true
    @State private var failure: String?
    @State private var standingsFailure: String?
    @State private var preferenceFailure: String?
    private var current: SportsScoreEvent { detail?.event ?? event }
    private var plays: [SportsScoreEvent.Play] { (current.plays ?? []).filter { !keyPlays || $0.scoring } }
    var body: some View {
        NavigationView {
            ScrollView {
                VStack(spacing: 24) {
                    matchup
                    if let names = current.broadcasts, !names.isEmpty {
                        Label(names.joined(separator: ", "), systemImage: "tv").font(.subheadline).foregroundStyle(.secondary)
                    }
                    Picker("Game details", selection: $tab) {
                        ForEach(SportsDetailTab.allCases) { Text($0.rawValue).tag($0) }
                    }.pickerStyle(.segmented)
                    if loading && detail == nil { ProgressView("Loading game") }
                    if let failure { Text(failure).font(.callout).foregroundStyle(.secondary) }
                    if detail?.stale == true { Text("Showing the last available game update.").font(.caption).foregroundStyle(.secondary) }
                    switch tab {
                    case .stats: statistics
                    case .plays: playByPlay
                    case .standings: standingsView
                    }
                    VStack(alignment: .leading, spacing: 14) {
                        Text("Game Information").font(.title3.bold())
                        if let date = current.dateValue { Label { Text(date, style: .date) + Text(" · ") + Text(date, style: .time) } icon: { Image(systemName: "clock") } }
                        if let venue = current.venue { Label(venue, systemImage: "mappin.and.ellipse") }
                        Text("Scores and game information from ESPN.").font(.caption).foregroundStyle(.secondary)
                        NavigationLink(destination: SportsGuideDashboard(sports: sports)) {
                            Label("Find your live TV coverage", systemImage: "play.tv")
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 12)
                }.padding(24).frame(maxWidth: 1000).frame(maxWidth: .infinity)
            }
            .background(SportsTeamBackdrop(home: current.home.color, away: current.away.color).ignoresSafeArea())
            .navigationTitle(current.league.uppercased())
            .modifier(MusicNavigationTitle(large: false))
            .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { dismiss() } } }
        }.navigationViewStyle(.stack)
            .task(id: "\(store.accountContext)|\(event.id)|\(scenePhase == .active)") {
                guard scenePhase == .active else { return }
                repeat {
                    await load()
                    guard current.isLive else { return }
                    do { try await Task.sleep(nanoseconds: 30_000_000_000) } catch { return }
                } while !Task.isCancelled
            }
            .onChange(of: store.accountContext) { _ in dismiss() }
    }
    private var matchup: some View {
        VStack(spacing: 24) {
            HStack(alignment: .top, spacing: 12) {
                detailTeam(current.away)
                VStack(spacing: 10) {
                    Text(current.statusLabel).font(.headline).multilineTextAlignment(.center)
                    if current.isScheduled, let date = current.dateValue { Text(date, style: .time).font(.title.bold()) }
                    if let clock = current.status.clock, current.isLive { Text(clock).font(.headline.monospacedDigit()) }
                }.frame(maxWidth: .infinity).padding(.top, 30)
                detailTeam(current.home)
            }
            if let preferenceFailure { Text(preferenceFailure).font(.caption).foregroundStyle(.secondary) }
        }
    }
    private func detailTeam(_ team: SportsScoreTeam) -> some View {
        VStack(spacing: 12) {
            if !current.isScheduled { Text(team.score ?? "—").font(.system(size: 64, weight: .bold, design: .rounded).monospacedDigit()) }
            SportsScoreLogo(team: team).frame(width: 76, height: 76)
            Text(team.displayName).font(.title3.bold()).multilineTextAlignment(.center)
            if let record = team.record { Text(record).font(.subheadline).foregroundStyle(.secondary) }
            let followed = team.isFollowed(league: event.league, preferences: sports.saved?.preferences)
            Button { Task { await follow(team) } } label: {
                Label(followed ? "Following" : "Follow", systemImage: followed ? "star.fill" : "star")
                    .font(.caption.weight(.semibold)).lineLimit(1).minimumScaleFactor(0.8)
                    .padding(.horizontal, 12).padding(.vertical, 10)
            }.buttonStyle(.plain).modifier(BluefinGlassSurface(cornerRadius: 24))
                .disabled(sports.saving || sports.loading || sports.saved == nil)
                .accessibilityLabel((followed ? "Unfollow " : "Follow ") + team.name)
        }.frame(maxWidth: .infinity)
    }
    private var statistics: some View {
        VStack(spacing: 22) {
            Text("Team Stats").font(.title3.bold())
            if let stats = current.stats, !stats.isEmpty {
                ForEach(Array(stats.enumerated()), id: \.offset) { _, stat in
                    VStack(spacing: 10) {
                        HStack {
                            Text(stat.away ?? "—").font(.title3.bold().monospacedDigit())
                            Spacer()
                            Text(stat.label).font(.subheadline.weight(.medium)).multilineTextAlignment(.center).foregroundStyle(.secondary)
                            Spacer()
                            Text(stat.home ?? "—").font(.title3.bold().monospacedDigit())
                        }
                        if let away = number(stat.away), let home = number(stat.home), away + home > 0 {
                            GeometryReader { geometry in
                                HStack(spacing: 6) {
                                    Capsule().fill(BluefinStyle.accent.opacity(0.7)).frame(width: max(0, (geometry.size.width - 6) * away / (away + home)))
                                    Capsule().fill(BluefinStyle.brand).frame(maxWidth: .infinity)
                                }
                            }.frame(height: 5).accessibilityHidden(true)
                        }
                    }
                }
            } else { Text("Team statistics are not available for this game yet.").foregroundStyle(.secondary).padding(.vertical, 16) }
        }.padding(22).frame(maxWidth: .infinity).modifier(BluefinGlassSurface(cornerRadius: 28))
    }
    private var playByPlay: some View {
        LazyVStack(spacing: 16) {
            Picker("Plays", selection: $keyPlays) { Text("All Plays").tag(false); Text("Key Plays").tag(true) }.pickerStyle(.segmented)
            if plays.isEmpty {
                Text(keyPlays ? "No scoring plays are available yet." : "Play-by-play is not available for this game yet.")
                    .foregroundStyle(.secondary).padding(24)
            }
            ForEach(Array(plays.reversed().prefix(250))) { play in
                VStack(alignment: .leading, spacing: 16) {
                    Text(play.text).font(.headline).fixedSize(horizontal: false, vertical: true)
                    HStack {
                        if let period = play.period { Text("Period \(period)") }
                        if let clock = play.clock { Text(clock).monospacedDigit() }
                        Spacer()
                        if let away = play.awayScore, let home = play.homeScore { Text("\(away) · \(home)").monospacedDigit() }
                    }.font(.caption).foregroundStyle(.secondary)
                }.padding(20).frame(maxWidth: .infinity, alignment: .leading).background(BluefinStyle.secondaryBackground, in: RoundedRectangle(cornerRadius: 26, style: .continuous))
            }
        }
    }
    @ViewBuilder private var standingsView: some View {
        if let standings, let league = SportsScoreLeague(rawValue: event.league) {
            if standings.stale { Text("Standings show the last available update.").font(.caption).foregroundStyle(.secondary) }
            ForEach(standings.groups) { group in
                VStack(alignment: .leading, spacing: 18) {
                    Text(group.name).font(.headline)
                    ScrollView(.horizontal, showsIndicators: false) {
                        VStack(spacing: 0) {
                            HStack {
                                Text("Team").frame(width: 188, alignment: .leading)
                                ForEach(league.standingsColumns, id: \.key) { column in Text(column.label).frame(width: 46) }
                            }.font(.caption.weight(.semibold)).foregroundStyle(.secondary).padding(.bottom, 12)
                            ForEach(group.entries) { entry in
                                HStack {
                                    HStack(spacing: 8) {
                                        if let rank = entry.rank { Text(String(rank)).monospacedDigit().frame(width: 20, alignment: .leading) }
                                        SportsScoreLogo(team: entry.team).frame(width: 24, height: 24)
                                        Text(entry.team.displayName).lineLimit(1)
                                    }.frame(width: 188, alignment: .leading)
                                    ForEach(league.standingsColumns, id: \.key) { column in
                                        Text(entry.stats[column.key] ?? "—").monospacedDigit().frame(width: 46)
                                    }
                                }.font(.subheadline).padding(.vertical, 12)
                                Divider()
                            }
                        }
                    }
                }.padding(20).modifier(BluefinGlassSurface(cornerRadius: 28))
            }
            if standings.groups.isEmpty { Text("Standings are not available yet.").foregroundStyle(.secondary).padding() }
        } else {
            Text(standingsFailure ?? "Standings are not available yet.").foregroundStyle(.secondary).padding()
        }
    }
    private func number(_ value: String?) -> Double? {
        guard let value, let number = Double(value.replacingOccurrences(of: "%", with: "")), number.isFinite, number >= 0 else { return nil }
        return number
    }
    private func follow(_ team: SportsScoreTeam) async {
        guard let saved = sports.saved, let session = store.session else { return }
        var preferences = saved.preferences
        let key = team.preferenceKey(league: event.league)
        if team.isFollowed(league: event.league, preferences: preferences) {
            preferences.teams.remove(key); preferences.teams.remove(team.id)
        } else { preferences.teams.insert(key) }
        preferenceFailure = nil
        do { try await sports.save(preferences, session: session, expectedETag: saved.etag) }
        catch is CancellationError {} catch { preferenceFailure = error.localizedDescription }
    }
    private func load() async {
        let context = store.accountContext
        loading = true
        defer { if store.accountContext == context { loading = false } }
        do {
            let response: SportsScoreDetail = try await store.authenticated("sportsEvent", fields: ["league": event.league, "eventId": event.id])
            try Task.checkCancellation()
            guard store.accountContext == context else { return }
            guard response.league == event.league, response.event.id == event.id else { throw BluefinError.message("The server returned a different game.") }
            detail = response; failure = nil
        } catch is CancellationError { return } catch { if store.accountContext == context { failure = error.localizedDescription } }
        if standings == nil {
            do {
                let response: SportsScoreStandings = try await store.authenticated("sportsStandings", fields: ["league": event.league])
                try Task.checkCancellation()
                guard store.accountContext == context else { return }
                guard response.league == event.league else { throw BluefinError.message("The server returned different standings.") }
                standings = response; standingsFailure = nil
            } catch is CancellationError {} catch { if store.accountContext == context { standingsFailure = error.localizedDescription } }
        }
    }
}
