import SwiftUI
import UIKit

/// Mount inside the parent's NavigationView and LibraryStore environment.
struct TrailersView: View {
    var onBack: (() -> Void)? = nil
    @EnvironmentObject private var library: LibraryStore
    @State private var page: TrailersPage?
    @State private var failure: String?
    @State private var loading = false
    @State private var start = 0
    @State private var retry = 0
    @State private var loadedIdentity = ""
    private var identity: String { TrailersAccount.identity(library.session) }
    private var requestID: String { "\(identity)|\(start)|\(retry)" }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                Text("Trailers").font(.largeTitle.bold()).accessibilityAddTraits(.isHeader)
                if loading { ProgressView("Finding trailers…") }
                if let failure { Text(failure).foregroundColor(.secondary); Button("Try again") { retry += 1 } }
                if let page, loadedIdentity == identity {
                    if page.Items.isEmpty { Text("No trailers here yet. Add authorized trailers to your server library to see them here.").padding(.vertical, 32) }
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 260, maximum: 440), spacing: 24)], spacing: 30) {
                        ForEach(page.Items) { title in
                            MediaNavigationLink(destination: .trailers(title.Id)) {
                                VStack(alignment: .leading, spacing: 10) {
                                    Artwork(item: title.media, backdrop: true).aspectRatio(16 / 9, contentMode: .fit).cornerRadius(14)
                                    Text(title.Name ?? "Untitled").font(.headline).foregroundColor(.primary)
                                    Text("Server library" + (title.ProductionYear.map { " · \($0)" } ?? ""))
                                        .font(.caption).foregroundColor(.secondary)
                                }.padding(8).contentShape(Rectangle())
                            }.buttonStyle(.plain)
                        }
                    }
                    HStack {
                        if start > 0 { Button("Previous") { start = max(0, start - 30) } }
                        Spacer()
                        if page.Items.count == 30 && start + page.Items.count < (page.TotalRecordCount ?? Int.max) {
                            Button("Next") { start += 30 }
                        }
                    }.disabled(loading)
                }
            }.padding(28)
        }.navigationTitle("Trailers")
        .task(id: requestID) { await load() }
        .onChange(of: identity) { _ in start = 0 }
        .bluefinBackNavigation(onBack)
    }
    @MainActor private func load() async {
        let key = requestID
        page = nil; failure = nil; loading = true
        defer { if key == requestID { loading = false } }
        do {
            let result: TrailersPage = try await library.authenticated("trailersCatalog", fields: ["startIndex": start, "limit": 30])
            try Task.checkCancellation()
            guard key == requestID else { return }
            page = result; loadedIdentity = identity
        } catch is CancellationError {} catch {
            if key == requestID { failure = "Trailers could not be loaded. Check your server connection and try again." }
        }
    }
}

struct TrailersDetailView: View {
    let itemID: String
    var onBack: (() -> Void)? = nil
    @EnvironmentObject private var library: LibraryStore
    @State private var detail: TrailersDetail?
    @State private var failure: String?
    @State private var loadedIdentity = ""
    @State private var retry = 0
    @State private var playID: String?
    @State private var busy = false
    private var identity: String { TrailersAccount.identity(library.session) }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if let detail, loadedIdentity == identity, MediaItem.matchesID(detail.item.Id, itemID) {
                    #if os(tvOS)
                    HStack(alignment: .top, spacing: 42) {
                        artwork(detail).frame(width: 510)
                        trailerActions(detail).frame(maxWidth: .infinity, alignment: .leading).focusSection()
                    }.frame(maxWidth: 1450)
                    #else
                    artwork(detail).frame(maxWidth: 1100)
                    trailerActions(detail)
                    #endif
                } else if failure == nil { ProgressView("Loading trailers…") }
                if busy { ProgressView("Preparing trailer…") }
                if let failure { Text(failure); Button("Reload trailers") { retry += 1 } }
            }.padding(28)
        }.navigationTitle("Trailers")
        .task(id: "\(identity)|\(itemID)|\(retry)") { await load() }
        .task(id: "\(identity)|\(playID ?? "")") { if let playID { await play(playID) } }
        .bluefinBackNavigation(onBack)
    }
    private func artwork(_ detail: TrailersDetail) -> some View {
        Artwork(item: detail.item.media, backdrop: detail.item.BackdropImageTags?.isEmpty == false)
            .aspectRatio(16 / 9, contentMode: .fit).cornerRadius(18)
    }

    private func trailerActions(_ detail: TrailersDetail) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(detail.item.Name ?? "Untitled").font(.largeTitle.bold()).accessibilityAddTraits(.isHeader)
            ForEach(detail.localTrailers) { trailer in
                Button { playID = trailer.Id } label: {
                    Label("Play \(trailer.Name ?? "trailer")", systemImage: "play.fill")
                }.disabled(busy || library.startingPlayback)
            }
            ForEach(detail.providers) { provider in TrailersProviderButton(provider: provider) }
            if detail.localTrailers.isEmpty && detail.providers.isEmpty {
                Text("No trailers available for this title.").foregroundStyle(.secondary)
            }
        }.buttonStyle(.bordered)
    }

    @MainActor private func load() async {
        let key = identity
        detail = nil; failure = nil; playID = nil
        do {
            let value: TrailersDetail = try await library.authenticated("trailersDetail", fields: ["itemId": itemID])
            try Task.checkCancellation()
            guard key == identity else { throw CancellationError() }
            guard MediaItem.matchesID(value.item.Id, itemID) else {
                failure = "The server returned trailers for a different title."
                return
            }
            detail = value; loadedIdentity = key
        } catch is CancellationError {} catch {
            if key == identity { failure = "This title's trailers could not be loaded. Check your connection or library access." }
        }
    }
    @MainActor private func play(_ id: String) async {
        let key = identity
        busy = true
        defer { busy = false; if key == identity { playID = nil } }
        do {
            // Reauthorize the exact association, never play the owner movie or a remote URL.
            let fresh: TrailersDetail = try await library.authenticated("trailersDetail", fields: ["itemId": itemID])
            try Task.checkCancellation()
            guard key == identity else { throw CancellationError() }
            guard let trailer = fresh.local(id: id, ownerID: itemID) else {
                throw BluefinError.message("The trailer association changed.")
            }
            await library.play(trailer, fromBeginning: true)
            if key == identity, let error = library.error { failure = error }
        } catch is CancellationError {} catch {
            if key == identity { failure = "This trailer is no longer available or could not be played. Reload trailers to try again." }
        }
    }
}

private struct TrailersProviderButton: View {
    let provider: TrailersProvider
    @State private var fallback = false
    @State private var unavailable = false
    var body: some View {
        Button {
            UIApplication.shared.open(provider.url, options: [.universalLinksOnly: true]) { opened in
                Task { @MainActor in if !opened { fallback = true } }
            }
        } label: { Label("Watch on \(provider.name)", systemImage: "arrow.up.forward.app") }
        .confirmationDialog("Open \(provider.name)", isPresented: $fallback, titleVisibility: .visible) {
            #if os(iOS)
            Button("Open official website") {
                UIApplication.shared.open(provider.url) { opened in
                    Task { @MainActor in unavailable = !opened }
                }
            }
            #endif
            Button("Cancel", role: .cancel) {}
        } message: {
            #if os(tvOS)
            Text("No installed app accepted this link. Open this title in the provider's app on another device.")
            #else
            Text("No installed app accepted this link. You can continue on the provider's website.")
            #endif
        }
        .alert("Provider unavailable", isPresented: $unavailable) { Button("OK", role: .cancel) {} }
    }
}
