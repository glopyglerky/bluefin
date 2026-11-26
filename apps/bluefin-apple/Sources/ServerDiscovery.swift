import SwiftUI

struct ServerDiscoveryCapability: Decodable { let search: Bool; let requests: Bool }
struct ServerDiscoveryPage: Decodable { let results: [ServerDiscoveryItem]; let page: Int; let totalPages: Int }
struct ServerDiscoveryItem: Decodable, Identifiable {
    let id: String; let title: String; let overview: String; let year: String?; let kind: String; let availability: String
    let actions: [ServerDiscoveryAction]
}
struct ServerDiscoveryAction: Decodable, Identifiable { let id: String; let label: String }
struct ServerActionForm: Decodable {
    struct Choice: Decodable, Identifiable { let id: String; let label: String }
    let id: String; let title: String; let notice: String; let choices: [Choice]; let requiresSelection: Bool
}
struct ServerActionReceipt: Decodable { let status: String; let message: String }

/// Server-enabled generic results. Provider settings and processing stay on the server.
struct ServerDiscoverySection: View {
    let query: String
    var scope = UnifiedSearchScope.all
    @EnvironmentObject private var store: LibraryStore
    @State private var items: [ServerDiscoveryItem] = []
    @State private var enabled = false
    @State private var page = 0
    @State private var totalPages = 0
    @State private var busy = false
    @State private var failure: String?
    @State private var generation = UUID()
    private var account: String { TrailersAccount.identity(store.session) + "|" + store.accountContext.uuidString }
    private var heading: String { scope == .music ? "Music catalog" : "Discover" }
    var body: some View {
        Group {
            if enabled {
                Section(heading) {
                    ForEach(items) { item in
                        NavigationLink(destination: ServerDiscoveryDetail(item: item)) {
                            VStack(alignment: .leading, spacing: 5) {
                                Text(item.title).font(.headline)
                                Text([item.year, item.kind == "series" ? "TV show" : item.kind.capitalized].compactMap { $0 }.joined(separator: " · ")).font(.subheadline).foregroundStyle(.secondary)
                            }
                        }
                    }
                    if busy { ProgressView("Searching connected catalog") }
                    if let failure { Text(failure).foregroundStyle(.secondary); Button("Try again") { Task { await load(more: false) } } }
                    if !busy && items.isEmpty && failure == nil { Text("No matching catalog titles.").foregroundStyle(.secondary) }
                    if page < totalPages { Button("More catalog results") { Task { await load(more: true) } }.disabled(busy) }
                    Text("Catalog results do not establish playback access. Your server controls available actions.").font(.footnote).foregroundStyle(.secondary)
                }
            }
        }.task(id: query + "|" + scope.rawValue + "|" + account) { await load(more: false) }
    }
    @MainActor private func load(more: Bool) async {
        guard let session = store.session, !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { items = []; enabled = false; return }
        let identity = account; let request = UUID(); generation = request; busy = true; failure = nil
        if !more { items = []; page = 0; totalPages = 0; enabled = false }
        defer { if generation == request { busy = false } }
        do {
            if !more {
                try await Task.sleep(nanoseconds: 250_000_000); try Task.checkCancellation()
                let capabilities: BluefinCapabilities = try await RustBridge.call("bluefinCapabilities", session: session)
                guard !Task.isCancelled, generation == request, identity == account else { return }
                guard capabilities.protocolVersion == 1, capabilities.discovery?.search == true else { enabled = false; return }
                enabled = true
            }
            let response: ServerDiscoveryPage = try await RustBridge.call("serverDiscoverySearch", session: session, fields: ["query": query, "scope": scope.rawValue, "page": more ? page + 1 : 1])
            try Task.checkCancellation(); guard !Task.isCancelled, generation == request, identity == account else { return }
            let existing = Set(items.map(\.id)); items += response.results.filter { !existing.contains($0.id) }
            page = response.page; totalPages = response.totalPages
        } catch is CancellationError {} catch {
            guard !Task.isCancelled, generation == request, identity == account else { return }
            if error.localizedDescription.contains("404") { enabled = false; items = [] }
            else if enabled { failure = error.localizedDescription }
        }
    }
}

private struct ServerDiscoveryDetail: View {
    let item: ServerDiscoveryItem
    @EnvironmentObject private var store: LibraryStore
    @State private var selected: ServerDiscoveryAction?
    var body: some View {
        List {
            Section { Text(item.title).font(.title.bold()); if let year = item.year { Text(year).foregroundStyle(.secondary) }; if !item.overview.isEmpty { Text(item.overview) } }
            Section("Catalog status") {
                Text(item.availability == "available" ? "In the connected library" : item.availability == "requested" ? "Already requested" : item.availability == "partiallyAvailable" ? "Partially available" : "Availability unknown")
            }
            if !item.actions.isEmpty { Section { ForEach(item.actions) { action in Button(action.label) { selected = action } } } }
            Text("Playback access is checked separately. A catalog entry does not authorize copying, sharing or recording.").font(.footnote).foregroundStyle(.secondary)
        }.navigationTitle(item.title).sheet(item: $selected) { action in ServerActionView(action: action).environmentObject(store) }
    }
}

private struct ServerActionView: View {
    let action: ServerDiscoveryAction
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var form: ServerActionForm?
    @State private var selections = Set<String>()
    @State private var confirmed = false
    @State private var busy = false
    @State private var failure: String?
    @State private var receipt: ServerActionReceipt?
    private var account: String { TrailersAccount.identity(store.session) }
    var body: some View {
        BluefinNavigation {
            Form {
                if let form {
                    Section { Text(form.title).font(.headline); Text(form.notice) }
                    if !form.choices.isEmpty {
                        Section("Choose options") {
                            ForEach(form.choices) { choice in
                                Toggle(choice.label, isOn: Binding(get: { selections.contains(choice.id) }, set: { if $0 { selections.insert(choice.id) } else { selections.remove(choice.id) } }))
                            }
                        }
                    }
                    if let receipt { Text(receipt.message).accessibilityAddTraits(.updatesFrequently) }
                    else {
                        Toggle("I am authorized to request this media", isOn: $confirmed)
                        Button(action.label) { Task { await submit() } }
                            .disabled(busy || !confirmed || (form.requiresSelection && selections.isEmpty))
                    }
                }
                if busy { ProgressView() }
                if let failure { Text(failure).foregroundStyle(.secondary) }
            }.navigationTitle(action.label).toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { dismiss() } } }
                .task(id: account + "|" + action.id) { await load() }
        }.onChange(of: account) { _ in dismiss() }
    }
    @MainActor private func load() async {
        guard let session = store.session else { dismiss(); return }
        let identity = account; busy = true; form = nil; failure = nil; confirmed = false; selections = []; receipt = nil
        defer { busy = false }
        do {
            let value: ServerActionForm = try await RustBridge.call("serverDiscoveryAction", session: session, fields: ["actionId": action.id])
            try Task.checkCancellation(); guard identity == account else { return }; form = value
        } catch is CancellationError {} catch { if identity == account { failure = error.localizedDescription } }
    }
    @MainActor private func submit() async {
        guard !busy, confirmed, let session = store.session else { return }
        let identity = account; busy = true; failure = nil
        defer { busy = false }
        do {
            let value: ServerActionReceipt = try await RustBridge.call("submitServerAction", session: session, fields: ["actionId": action.id, "selections": selections.sorted(), "authorizedUseConfirmed": true])
            guard identity == account else { return }; receipt = value
        } catch { if identity == account { failure = error.localizedDescription } }
    }
}
