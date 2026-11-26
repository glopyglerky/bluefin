import SwiftUI

struct LocalAssistanceSearchSection: View {
    let query: String
    let candidates: [MediaItem]
    let loadedContext: UUID?
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model = LocalAssistanceModel()
    @State private var preference = ""

    private var eligible: Bool {
        LocalAssistanceAvailability.isAvailable && store.signedIn &&
        loadedContext == store.accountContext && !candidates.isEmpty
    }

    var body: some View {
        if eligible {
            Section("Suggestions on this device") {
                TextField("Describe what you want from these titles", text: $preference)
                LocalAssistanceControls(model: model, actionTitle: "Find relevant titles") {
                    let context = store.accountContext
                    model.start(
                        input: LocalAssistanceInput.catalog(query: preference, items: candidates),
                        context: context,
                        isCurrent: { store.signedIn && store.accountContext == context }
                    )
                }
                if let result = model.result {
                    ForEach(result.items, id: \.id) { suggestion in
                        if let item = candidates.first(where: { $0.Id == suggestion.id }) {
                            MediaNavigationLink(destination: .detail(item)) {
                                Label(item.title, systemImage: "sparkles")
                            }
                        }
                    }
                }
                Text("Chooses from up to 12 loaded results. Suggestions stay on this device; opening a title uses the normal library controls.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            .onAppear { preference = query }
            .onChange(of: query) { _ in model.cancel(); preference = query }
            .onChange(of: loadedContext) { _ in model.cancel() }
            .onChange(of: store.accountContext) { _ in model.cancel() }
            .onChange(of: scenePhase) { if $0 != .active { model.cancel() } }
            .onDisappear { model.cancel() }
        }
    }
}

struct LocalAssistanceSummary: View {
    let item: MediaItem
    let description: String
    @State private var inputContext: UUID
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model = LocalAssistanceModel()

    init(item: MediaItem, description: String, context: UUID) {
        self.item = item
        self.description = description
        _inputContext = State(initialValue: context)
    }

    var body: some View {
        if LocalAssistanceAvailability.isAvailable && store.signedIn && inputContext == store.accountContext {
            VStack(alignment: .leading, spacing: 12) {
                LocalAssistanceControls(model: model, actionTitle: "Highlight this description") {
                    model.start(
                        input: LocalAssistanceInput.summary(item: item, description: description),
                        context: inputContext,
                        isCurrent: { store.signedIn && store.accountContext == inputContext }
                    )
                }
                if let result = model.result, result.source_item_id == item.Id {
                    ForEach(Array(result.excerpts.enumerated()), id: \.offset) { _, excerpt in
                        Text(excerpt).font(.callout).fixedSize(horizontal: false, vertical: true)
                    }
                    if !result.excerpts.isEmpty {
                        Text("Selected excerpts from this description. The description may include spoilers.")
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                }
            }
            .onChange(of: description) { _ in model.cancel() }
            .onChange(of: store.accountContext) { _ in model.cancel() }
            .onChange(of: scenePhase) { if $0 != .active { model.cancel() } }
            .onDisappear { model.cancel() }
        }
    }
}

struct LocalAssistancePlayback: View {
    let status: LocalAssistancePlaybackStatus
    @State private var inputContext: UUID
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model = LocalAssistanceModel()

    init(status: LocalAssistancePlaybackStatus, context: UUID) {
        self.status = status
        _inputContext = State(initialValue: context)
    }

    var body: some View {
        if LocalAssistanceAvailability.isAvailable && store.signedIn && inputContext == store.accountContext {
            VStack(alignment: .leading, spacing: 10) {
                LocalAssistanceControls(model: model, actionTitle: "Explain playback status") {
                    model.start(input: LocalAssistanceInput.playback(status: status), context: inputContext,
                        isCurrent: { store.signedIn && store.accountContext == inputContext })
                }
                if let result = model.result {
                    ForEach(Array(result.excerpts.enumerated()), id: \.offset) { _, excerpt in
                        Text(excerpt).font(.callout).foregroundStyle(.secondary)
                    }
                }
            }
            .onChange(of: status) { _ in model.cancel() }
            .onChange(of: store.accountContext) { _ in model.cancel() }
            .onChange(of: scenePhase) { if $0 != .active { model.cancel() } }
            .onDisappear { model.cancel() }
        }
    }
}

private struct LocalAssistanceControls: View {
    @ObservedObject var model: LocalAssistanceModel
    let actionTitle: String
    let action: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Button(action: action) { Label(actionTitle, systemImage: "sparkles") }
                    .disabled(model.busy)
                if model.busy {
                    ProgressView()
                    Button("Cancel") { model.cancel() }
                }
            }
            if let message = model.message { Text(message).font(.footnote).foregroundStyle(.secondary) }
        }
    }
}
