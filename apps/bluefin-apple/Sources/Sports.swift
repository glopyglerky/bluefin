import SwiftUI

struct SportsHomeSection: View {
    @EnvironmentObject private var store: LibraryStore
    @ObservedObject var sports: SportsStore
    var body: some View {
        if let feed = sports.feed {
            VStack(alignment: .leading, spacing: 18) {
                HStack {
                    NavigationLink(destination: SportsDashboard(sports: sports).bluefinBackNavigation()) { Label("Sports", systemImage: "sportscourt").font(.title2.bold()) }
                    Spacer()
                    NavigationLink(destination: SportsSettingsView(sports: sports).bluefinBackNavigation()) { Image(systemName: "slider.horizontal.3").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Sports preferences")
                }.padding(.horizontal, 24)
                if feed.games.isEmpty && feed.local_broadcasts.isEmpty {
                    Text("No sports guide listings are available in this page. Your preferences are kept.").foregroundStyle(.secondary).padding(.horizontal, 24)
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
        } else if sports.loading {
            ProgressView("Loading sports").padding(.horizontal, 24)
        } else if let failure = sports.failure {
            VStack(alignment: .leading, spacing: 10) {
                Text("Sports couldn't load").font(.headline)
                Text(failure).font(.footnote).foregroundStyle(.secondary)
                Button("Try again") { Task { await sports.connect(store.session) } }
            }.padding(.horizontal, 24)
        }
    }
}

private struct SportsBroadcastList: View {
    let title: String
    let options: [SportsOption]
    var body: some View { List(options) { SportsBroadcastRow(option: $0) }.navigationTitle(title) }
}

struct SportsBroadcastRow: View {
    let option: SportsOption
    @EnvironmentObject private var store: LibraryStore
    @State private var watching = false
    @Environment(\.timeZone) private var timeZone
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(option.broadcastLabel).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                if let league = option.event?.league_name { Text(league).font(.caption).foregroundStyle(.secondary) }
            }
            if let competitors = option.event?.competitors, !competitors.isEmpty {
                ForEach(competitors) { team in
                    HStack(spacing: 12) {
                        SportsTeamLogo(team: team).frame(width: 32, height: 32)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(team.short_name ?? team.name).font(.headline)
                            if let record = team.record, !record.isEmpty { Text(record).font(.caption).foregroundStyle(.secondary) }
                        }
                        Spacer()
                        if option.showsScores, let score = team.score, !score.isEmpty { Text(score).font(.title2.weight(.semibold).monospacedDigit()) }
                    }.accessibilityElement(children: .combine)
                }
                if let detail = option.event?.detail, !detail.isEmpty { Text(detail).font(.caption).foregroundStyle(.secondary) }
            } else { Text(option.displayTitle).font(.headline) }
            if let broadcasters = option.event?.broadcasters, !broadcasters.isEmpty {
                Label(broadcasters.joined(separator: ", "), systemImage: "tv").font(.caption).foregroundStyle(.secondary)
            }
            if let date = option.scheduleDate {
                Text(SportsSchedule.label(date, timeZone: timeZone)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            Button { watching = true } label: { Label("Where to watch", systemImage: "tv") }.buttonStyle(.bordered)
            if let channel = option.channel_name { Text(channel).font(.subheadline).foregroundStyle(.secondary) }
            if let status = option.recording_status { Text(status).font(.caption).foregroundStyle(.secondary) }
            if option.canOpenChannel, let id = option.channel_id {
                MediaNavigationLink(destination: .channel(MediaItem(Id: id, Name: option.channel_name ?? "Live channel", Type: "TvChannel"))) {
                    Label(option.kind == "UPCOMING" ? "View channel and schedule" : "Open channel", systemImage: "tv")
                }.buttonStyle(.bordered)
            } else if option.kind == "RECORDING" {
                NavigationLink(destination: DetailView(item: MediaItem(Id: option.item_id, Name: option.displayTitle, Type: "Recording"))) { Label("Open recording", systemImage: "play.rectangle") }.buttonStyle(.bordered)
            }
        }.sheet(isPresented: $watching) { SportsWatchSheet(option: option).environmentObject(store) }
        .padding(18).frame(maxWidth: .infinity, alignment: .leading).background(BluefinStyle.secondaryBackground, in: RoundedRectangle(cornerRadius: 16))
    }
}

private struct SportsTeamLogo: View {
    let team: SportsOption.Competitor
    private var url: URL? {
        guard let raw = team.logo, raw.count <= 1024, let p = URLComponents(string: raw),
              p.scheme == "https", p.host == "a.espncdn.com", p.user == nil,
              p.password == nil, p.port == nil, p.fragment == nil, p.query == nil else { return nil }
        return p.url
    }
    var body: some View {
        AsyncImage(url: url) { image in image.resizable().scaledToFit() } placeholder: {
            Text(String((team.abbreviation ?? team.name).prefix(3))).font(.caption2.bold())
                .frame(maxWidth: .infinity, maxHeight: .infinity).background(.secondary.opacity(0.1), in: Circle())
        }.accessibilityHidden(true)
    }
}
