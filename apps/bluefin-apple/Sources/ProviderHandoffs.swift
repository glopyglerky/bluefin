import SwiftUI
import UIKit
#if os(iOS)
import StoreKit
#endif

struct ProviderDestination: Identifiable {
    let name: String
    let url: URL
    let storeID: String?
    var id: String { url.absoluteString }
    private static let stores = ["tv.apple.com": "1174078549", "music.apple.com": "1108187390", "open.spotify.com": "324684580",
        "www.espn.com": "317469184", "www.peacocktv.com": "1508186374", "www.paramountplus.com": "530168168",
        "www.fubo.tv": "905401434", "tv.youtube.com": "1193350206", "www.hulu.com": "376510438", "www.amazon.com": "545519333"]
    private static let hosts: Set<String> = Set(stores.keys).union(["www.disneyplus.com", "www.netflix.com", "www.fox.com", "www.sling.com", "www.dazn.com", "www.nfl.com", "www.nba.com", "www.mlb.com", "www.nhl.com", "www.bigtenplus.com", "www.flosports.tv", "f1tv.formula1.com", "www.tennistv.com"])
    static func validated(name: String, rawURL: String) -> Self? {
        guard rawURL.count <= 2048, !name.isEmpty, name.count <= 128, let p = URLComponents(string: rawURL),
              p.scheme == "https", let host = p.host, hosts.contains(host), p.user == nil,
              p.password == nil, p.port == nil, p.fragment == nil, p.query == nil, let url = p.url else { return nil }
        if host == "www.amazon.com" && !p.path.hasPrefix("/gp/video/") { return nil }
        return Self(name: name, url: url, storeID: stores[host])
    }
    static let common: [Self] = [
        ("Apple TV", "https://tv.apple.com/"), ("Apple Music", "https://music.apple.com/"), ("Spotify", "https://open.spotify.com/"),
        ("ESPN", "https://www.espn.com/watch/"), ("YouTube TV", "https://tv.youtube.com/"), ("Paramount+", "https://www.paramountplus.com/live-tv/"),
        ("Hulu", "https://www.hulu.com/hub/sports"), ("Peacock", "https://www.peacocktv.com/sports"), ("Prime Video", "https://www.amazon.com/gp/video/sports"),
        ("Fubo", "https://www.fubo.tv/")
    ].compactMap { validated(name: $0.0, rawURL: $0.1) }
}

struct ProviderHandoffButton: View {
    let destination: ProviderDestination
    var label: String? = nil
    @State private var fallback = false
    @State private var product = false
    var body: some View {
        Button {
            UIApplication.shared.open(destination.url, options: [.universalLinksOnly: true]) { opened in
                Task { @MainActor in if !opened { fallback = true } }
            }
        } label: { Label(label ?? "Open \(destination.name)", systemImage: "arrow.up.forward.app") }
        .confirmationDialog("Open \(destination.name)", isPresented: $fallback, titleVisibility: .visible) {
            Button("Open official website") { UIApplication.shared.open(destination.url) }
            if let id = destination.storeID {
                #if os(iOS)
                Button("View app in App Store") { product = true }
                #else
                Button("View app in App Store") { if let url = URL(string: "https://apps.apple.com/app/id" + id) { UIApplication.shared.open(url) } }
                #endif
            }
            Button("Cancel", role: .cancel) {}
        } message: { Text("The native app did not open. A provider account or subscription may be required.") }
        #if os(iOS)
        .sheet(isPresented: $product) { if let id = destination.storeID { NativeStoreProduct(productID: id) } }
        #endif
    }
}

struct ConnectedAppsView: View {
    var body: some View {
        List {
            Section {
                ForEach(ProviderDestination.common) { ProviderHandoffButton(destination: $0) }
            } footer: { Text("These shortcuts use native app handoff and official App Store listings. They do not sign you in, confirm a subscription, or grant Bluefin access to a provider's catalog.") }
            Section { NavigationLink("Libraries and rights", destination: AuthorizedUseView()) }
        }.navigationTitle("Connected apps")
    }
}
