import SwiftUI

struct SportsSettingsView: View {
    @ObservedObject var sports: SportsStore
    @EnvironmentObject private var store: LibraryStore
    @State private var preferences: SportsPreferences?
    @State private var revision: String?
    @State private var saving = false
    @State private var failure: String?
    @State private var reloadRequired = false
    @State private var draftAccount: String?
    private var account: String? { store.session.map(OfflineLibrary.identity) }
    private var teams: [SportsOption.Competitor] {
        let entries = sports.feed?.games.flatMap { $0.event?.competitors ?? [] } ?? []
        var seen = Set<String>()
        return entries.filter { seen.insert($0.id).inserted }.sorted { $0.name < $1.name }
    }
    var body: some View {
        Form {
            if let preferences, draftAccount == account {
                Section {
                    Toggle("Sports Home", isOn: boolean(\.sports_only))
                    Toggle("Include native Home", isOn: boolean(\.native_home))
                    Toggle("Home games", isOn: boolean(\.home))
                    Toggle("Away games", isOn: boolean(\.away))
                } footer: { Text("These choices change your sports presentation. They do not grant access to media or providers.") }
                Section {
                    Toggle("All teams", isOn: boolean(\.all_teams))
                    ForEach(teams) { team in
                        Toggle(team.name, isOn: membership(\.teams, id: team.id)).disabled(preferences.all_teams)
                    }
                    let named = Set(teams.map(\.id))
                    ForEach(preferences.teams.subtracting(named).sorted(), id: \.self) { id in
                        Toggle("Saved team · \(id)", isOn: membership(\.teams, id: id)).disabled(preferences.all_teams)
                    }
                } header: { Text("Favorite teams") } footer: {
                    Text("Teams in verified events are listed here. Saved favorites are kept. Channel guide listings are not filtered by favorites.")
                }
                Section("Favorite leagues") {
                    let known = Set(sports.feed?.games.compactMap { $0.event?.league_name } ?? [])
                    // Display names are not API league IDs. Never write them as identifiers.
                    ForEach(preferences.leagues.sorted(), id: \.self) { id in
                        Toggle("Saved league · \(id)", isOn: membership(\.leagues, id: id))
                    }
                    Text("Schedule leagues: \(known.sorted().joined(separator: ", "))").font(.footnote).foregroundStyle(.secondary)
                    Text("Add league favorites when the server supplies a league catalog with stable IDs.").font(.footnote).foregroundStyle(.secondary)
                }
                Section("Other selections") {
                    LabeledContentCompat("Sports", value: "\(preferences.sports.count)")
                    LabeledContentCompat("Conferences", value: preferences.all_conferences ? "All" : "\(preferences.conferences.count) selected")
                    LabeledContentCompat("Players", value: preferences.all_players ? "All" : "\(preferences.players.count) selected")
                    Text("Existing sport, conference, player and recording choices are preserved.").font(.footnote).foregroundStyle(.secondary)
                }
                Button(saving ? "Saving…" : "Save preferences") { save() }
                    .disabled(saving || sports.loading || reloadRequired || (!preferences.home && !preferences.away))
                if !preferences.home && !preferences.away { Text("Select home games, away games, or both.") }
            } else if sports.loading { ProgressView("Loading preferences") }
            else { Text("Connect to load sports preferences.") }
            if let failure { Text(failure).foregroundStyle(.secondary).accessibilityLabel("Preferences error. \(failure)") }
            Button("Reload server preferences") {
                Task {
                    await sports.connect(store.session)
                    if sports.failure == nil { loadDraft() } else { failure = sports.failure }
                }
            }.disabled(saving || sports.loading)
        }.navigationTitle("Sports preferences")
        .disabled(saving)
        .task(id: account) {
            preferences = nil; revision = nil; failure = nil; reloadRequired = false
            if sports.saved == nil { await sports.connect(store.session) }
            loadDraft()
            failure = sports.failure
        }
        .onChange(of: sports.accountGeneration) { _ in
            preferences = nil; revision = nil; failure = nil; reloadRequired = false; draftAccount = nil
        }
    }
    private func loadDraft() {
        preferences = sports.saved?.preferences; revision = sports.saved?.etag
        draftAccount = account; reloadRequired = false; failure = nil
    }
    private func save() {
        guard let session = store.session, let updated = preferences, let revision, draftAccount == account else { return }
        let savingAccount = account
        saving = true; failure = nil
        Task {
            defer { saving = false }
            do {
                try await sports.save(updated, session: session, expectedETag: revision)
                guard account == savingAccount else { return }
                loadDraft()
            } catch is CancellationError {} catch {
                guard account == savingAccount else { return }
                failure = error.localizedDescription
                reloadRequired = error is SportsPreferenceError
            }
        }
    }
    private func boolean(_ key: WritableKeyPath<SportsPreferences, Bool>) -> Binding<Bool> {
        Binding(get: { preferences?[keyPath: key] ?? false }, set: { preferences?[keyPath: key] = $0 })
    }
    private func membership(_ key: WritableKeyPath<SportsPreferences, Set<String>>, id: String) -> Binding<Bool> {
        Binding(get: { preferences?[keyPath: key].contains(id) ?? false }, set: { selected in
            if selected { preferences?[keyPath: key].insert(id) } else { preferences?[keyPath: key].remove(id) }
        })
    }
}
