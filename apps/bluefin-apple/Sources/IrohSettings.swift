import SwiftUI
import Network

struct IrohSettingsView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var enabled = false
    @State private var pin = ""
    @State private var address = ""
    @State private var failure: String?
    @State private var saved = false
    var body: some View {
        Form {
            Section {
                Toggle("Use a pinned Iroh peer", isOn: $enabled)
                if enabled {
                    TextField("Endpoint public key", text: $pin).bluefinAddressInput()
                    TextField("Numeric IP address and port", text: $address).bluefinAddressInput()
                }
            } footer: {
                Text("Advanced setup for a separately configured Bluefin media adapter. Add only a pin supplied by your own trusted server; it receives your media authorization. A generic Iroh or MCP endpoint is not a media adapter.")
            }
            Section {
                Button("Save transport") { save() }
                if saved { Label("Saved for new streams", systemImage: "checkmark.circle").foregroundStyle(.secondary) }
                if let failure { Text(failure).foregroundStyle(.secondary) }
            } footer: {
                Text("Metadata and downloads use your selected server's HTTPS API. Media streams use the pinned peer without silently falling back to HTTPS. Existing streams keep their captured route. This does not change your account's media permissions.")
            }
        }.navigationTitle("Direct Iroh transport")
        .onAppear {
            pin = store.session?.irohPeerId ?? ""
            address = store.session?.irohPeerAddress ?? ""
            enabled = !pin.isEmpty && !address.isEmpty
        }
    }
    private func save() {
        failure = nil; saved = false
        do {
            if enabled {
                let pin = pin.trimmingCharacters(in: .whitespacesAndNewlines)
                let address = address.trimmingCharacters(in: .whitespacesAndNewlines)
                guard (32...128).contains(pin.count), pin.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }),
                      let endpoint = URLComponents(string: "https://" + address), let host = endpoint.host,
                      let port = endpoint.port, (1...65535).contains(port), endpoint.user == nil,
                      endpoint.password == nil, endpoint.path.isEmpty, endpoint.query == nil, endpoint.fragment == nil,
                      IPv4Address(host) != nil || IPv6Address(host.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))) != nil else {
                    throw BluefinError.message("Enter the server's public endpoint key and a numeric IP:port. IPv6 addresses need brackets.")
                }
                guard store.session?.server.lowercased().hasPrefix("https://") == true else {
                    throw BluefinError.message("Pinned Iroh requires a canonical HTTPS server address.")
                }
                try store.saveIrohPeer(id: pin, address: address)
            } else { try store.saveIrohPeer(id: nil, address: nil) }
            saved = true
        } catch { failure = error.localizedDescription }
    }
}
