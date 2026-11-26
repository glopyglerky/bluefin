import SwiftUI

enum BluefinAuthorizedUse {
    static let connectionNotice = "Connect only libraries and provider accounts you own or are authorized to use. Bluefin does not provide commercial media or unlock streaming subscriptions."
}

struct AuthorizedUseView: View {
    var body: some View {
        Form {
            Section("Your libraries and accounts") {
                Text(BluefinAuthorizedUse.connectionNotice)
                Text("Follow applicable law, rights-holder permissions and provider terms. Owning a copy or account does not authorize every form of copying, recording or sharing.")
            }
            Section("Discovery and playback") {
                Text("A title, guide entry or provider link does not prove subscription access, regional availability, recording permission or sharing rights. The server or provider authorizes access.")
                Text("Some titles must open in the provider's app and require a separate subscription. Bluefin does not bypass DRM, payment, authentication, geographic restrictions or device limits.")
            }
            Section("Sharing and downloads") {
                Text("Use downloads, recordings, karaoke processing, casting and SharePlay only when your permissions and provider terms allow them. Every participant uses their own authorized account.")
            }
            Section("Independent software") {
                Text("Bluefin is independent of Apple, Jellyfin, Plex and connected providers. Their names identify compatibility. Media, artwork, lyrics and other third-party materials retain their owners' rights.")
                Text("Bluefin does not bundle, install or require the arr applications. Optional integrations connect separate applications through supported interfaces.")
                NavigationLink("Software licenses", destination: SoftwareLicensesView())
                Link("Project source", destination: URL(string: "https://github.com/glopyglerky/bluefin")!)
            }
        }.navigationTitle("Libraries and rights")
    }
}
