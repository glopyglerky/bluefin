import SwiftUI
import UIKit
#if os(iOS)
import StoreKit
#endif

/// Only explicit server metadata is accepted. ProviderIds are deliberately never converted to URLs.
struct NativeProviderLink: Identifiable, Hashable {
    let url: URL
    var id: String { url.absoluteString }
    var title: String { url.host == "music.apple.com" ? "Apple Music" : url.host == "tv.apple.com" ? "Apple TV" : "Spotify" }
    var storeID: String { url.host == "music.apple.com" ? "1108187390" : url.host == "tv.apple.com" ? "1174078549" : "324684580" }

    static func validated(_ raw: String) -> NativeProviderLink? {
        guard raw.count <= 2048, let parts = URLComponents(string: raw),
              parts.scheme == "https", parts.user == nil, parts.password == nil,
              parts.port == nil, parts.fragment == nil,
              let host = parts.host, ["open.spotify.com", "music.apple.com", "tv.apple.com"].contains(host),
              let url = parts.url else { return nil }
        let path = parts.path.split(separator: "/")
        if host == "open.spotify.com" {
            guard parts.query == nil, path.count == 2, ["track", "album", "artist", "playlist", "episode", "show"].contains(String(path[0])),
                  path[1].count == 22, path[1].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else { return nil }
        } else if host == "music.apple.com" {
            guard path.count == 4, path[0].count == 2, ["album", "song", "artist", "playlist"].contains(String(path[1])),
                  path[3].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "." || $0 == "-") }) else { return nil }
            // Apple's catalog song links can use an album URL with an i= song identifier.
            guard (parts.queryItems ?? []).allSatisfy({ $0.name == "i" && !($0.value ?? "").isEmpty && ($0.value ?? "").allSatisfy(\.isNumber) }) else { return nil }
        } else {
            guard parts.query == nil, path.count == 4, path[0].count == 2,
                  ["movie", "show", "episode", "season"].contains(String(path[1])), path[3].hasPrefix("umc."),
                  path[3].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == ".") }) else { return nil }
        }
        return .init(url: url)
    }

    static func links(for item: MediaItem) -> [NativeProviderLink] {
        Array(Set((item.ExternalUrls ?? []).compactMap { $0.Url.flatMap(validated) })).sorted { $0.id < $1.id }.prefix(8).map { $0 }
    }
}

@MainActor struct NativeIntegrationActions: View {
    let item: MediaItem
    @EnvironmentObject private var store: LibraryStore
    @State private var share = false
    @State private var fallback: NativeProviderLink?
    @State private var message: String?
    #if os(iOS)
    @State private var product = false
    @State private var productID = ""
    #endif

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            #if os(iOS)
            Button { share = true } label: { Label("Share title", systemImage: "square.and.arrow.up") }
                .sheet(isPresented: $share) { NativeShareSheet(values: [item.title]) }
            #endif
            ForEach(NativeProviderLink.links(for: item)) { link in
                Button("Open on \(link.title)") {
                    UIApplication.shared.open(link.url, options: [.universalLinksOnly: true]) { opened in
                        Task { @MainActor in if !opened { fallback = link } }
                    }
                }
            }
            #if BLUEFIN_SHAREPLAY
            Button("SharePlay") {
                Task { do { try await NativeIntegrations.shared.startSharing(item) } catch { message = error.localizedDescription } }
            }.disabled(store.session == nil || store.preview)
            #endif
        }
        .confirmationDialog("The provider app did not open", isPresented: Binding(get: { fallback != nil }, set: { if !$0 { fallback = nil } }), titleVisibility: .visible) {
            if let link = fallback {
                Button("Open provider website") { UIApplication.shared.open(link.url); fallback = nil }
                #if os(iOS)
                Button("View \(link.title) in App Store") { productID = link.storeID; product = true; fallback = nil }
                #else
                Button("View \(link.title) in App Store") {
                    // Official listing. Availability on this device is not asserted.
                    UIApplication.shared.open(URL(string: "https://apps.apple.com/app/id" + link.storeID)!)
                    fallback = nil
                }
                #endif
            }
            Button("Cancel", role: .cancel) { fallback = nil }
        }
        .alert("Native integration", isPresented: Binding(get: { message != nil }, set: { if !$0 { message = nil } })) {
            Button("OK") { message = nil }
        } message: { Text(message ?? "") }
        #if os(iOS)
        .sheet(isPresented: $product) { NativeStoreProduct(productID: productID) }
        #endif
    }
}

#if os(iOS)
private struct NativeShareSheet: UIViewControllerRepresentable {
    let values: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: values, applicationActivities: nil)
    }
    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}

struct NativeStoreProduct: UIViewControllerRepresentable {
    let productID: String
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeUIViewController(context: Context) -> SKStoreProductViewController {
        let controller = SKStoreProductViewController()
        controller.delegate = context.coordinator
        controller.loadProduct(withParameters: [SKStoreProductParameterITunesItemIdentifier: productID]) { success, _ in
            if !success { controller.dismiss(animated: true) }
        }
        return controller
    }
    func updateUIViewController(_ controller: SKStoreProductViewController, context: Context) {}
    final class Coordinator: NSObject, SKStoreProductViewControllerDelegate {
        func productViewControllerDidFinish(_ viewController: SKStoreProductViewController) { viewController.dismiss(animated: true) }
    }
}
#endif
