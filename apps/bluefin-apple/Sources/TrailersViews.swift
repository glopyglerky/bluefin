import SwiftUI
import UIKit

/// Mount inside the parent's NavigationView and LibraryStore environment.
struct TrailersView: View {
    var onBack: (() -> Void)? = nil
    @EnvironmentObject private var library: LibraryStore
    @StateObject private var model = PagedMediaModel()
    private var identity: String { "\(library.accountContext)|\(TrailersAccount.identity(library.session))" }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 28) {
                Text("Trailers").font(.largeTitle.bold()).accessibilityAddTraits(.isHeader)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 260, maximum: 440), spacing: 24)], spacing: 30) {
                    ForEach(model.window.pages) { page in
                        ForEach(page.range, id: \.self) { index in
                            if !model.window.isDuplicate(at: index) {
                                Group {
                                    if let item = model.window.item(at: index) {
                                        MediaNavigationLink(destination: .trailers(item.Id)) {
                                            VStack(alignment: .leading, spacing: 10) {
                                                Artwork(item: item, backdrop: true, thumbnailWidth: 720)
                                                    .aspectRatio(16 / 9, contentMode: .fit).cornerRadius(14)
                                                Text(item.displayTitle).font(.headline).foregroundColor(.primary).lineLimit(1)
                                                Text("Server library" + (item.ProductionYear.map { " · \($0)" } ?? ""))
                                                    .font(.caption).foregroundColor(.secondary).lineLimit(1)
                                            }.padding(8).contentShape(Rectangle())
                                        }.buttonStyle(.plain)
                                    } else {
                                        VStack(alignment: .leading, spacing: 10) {
                                            RoundedRectangle(cornerRadius: 14).fill(BluefinStyle.secondaryBackground)
                                                .aspectRatio(16 / 9, contentMode: .fit)
                                            Text(page.items == nil ? "Loading title…" : "No longer available").font(.headline).foregroundColor(.secondary).lineLimit(1)
                                            Text(" ").font(.caption)
                                        }.padding(8).accessibilityLabel(page.items == nil ? "Loading trailer title" : "Title no longer available")
                                    }
                                }
                                .onAppear { model.appear(index) }
                                .onDisappear { model.disappear(index) }
                            }
                        }
                    }
                }
                if model.loading { ProgressView("Finding trailers…") }
                ForEach(model.failures.keys.sorted(), id: \.self) { offset in
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Trailers could not be loaded. Check your server connection and try again.").foregroundStyle(.secondary)
                        Button("Try again") { model.retry(offset) }
                    }
                }
                if model.window.pages.isEmpty && !model.loading && model.failures.isEmpty {
                    Text("No trailers here yet. Add authorized trailers to your server library to see them here.").padding(.vertical, 32)
                }
            }.padding(28)
        }
        .navigationTitle("Trailers")
        .task(id: identity) {
            await model.start(identity: identity) { start, limit in
                let count = min(limit, 30)
                let page: TrailersPage = try await library.authenticated("trailersCatalog", fields: ["startIndex": start, "limit": count])
                let total = page.TotalRecordCount ?? (start + page.Items.count + (page.Items.count == count ? 1 : 0))
                return ItemPage(Items: page.Items.map(\.media), TotalRecordCount: total)
            }
        }
        .onDisappear { model.cancel() }
        .bluefinBackNavigation(onBack)
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
