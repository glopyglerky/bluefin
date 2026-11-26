import SwiftUI

struct IrohSettingsView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var enabled = false
    @State private var pin = ""
    @State private var address = ""
    @State private var failure: String?
    @State private var saved = false
    @State private var loading = false
    @State private var advertised: ServerIrohTransport?
    @State private var formContext: UUID?
    @State private var requestID: UUID?
    @State private var loadTask: Task<Void, Never>?
    var body: some View {
        Form {
            Section {
                Button("Load connection from this server") {
                    guard loadTask == nil else { return }
                    loadTask = Task { await loadServerConnection() }
                }.disabled(loading || store.preview || store.session?.server.lowercased().hasPrefix("https://") != true)
                if loading {
                    ProgressView("Checking your server")
                    Button("Cancel") {
                        loadTask?.cancel(); loadTask = nil; requestID = nil; loading = false
                    }
                }
                if let advertised {
                    ForEach(advertised.directAddresses, id: \.self) { hint in
                        Button {
                            pin = advertised.endpointId; address = hint; enabled = true; saved = false
                        } label: {
                            HStack {
                                Text(hint).font(.body.monospaced())
                                Spacer()
                                if pin == advertised.endpointId && address == hint {
                                    Image(systemName: "checkmark")
                                }
                            }
                        }
                    }
                }
            } header: { Text("Your server") } footer: {
                Text("Load connection details from your signed-in HTTPS server, then save to use them for new streams. Direct addresses are connection hints; availability depends on your network.")
            }
            Section {
                Toggle("Use a pinned Iroh peer", isOn: $enabled)
                if enabled {
                    TextField("Endpoint public key", text: $pin).bluefinAddressInput()
                    TextField("Numeric IP address and port", text: $address).bluefinAddressInput()
                }
            } footer: {
                Text("Your server can provide a direct connection when its operator enables Iroh. You can also enter connection details from a server you trust; it receives your media authorization.")
            }.disabled(loading)
            Section {
                Button("Save transport") { save() }.disabled(loading)
                if saved { Label("Saved for new streams", systemImage: "checkmark.circle").foregroundStyle(.secondary) }
                if let failure { Text(failure).foregroundStyle(.secondary) }
            } footer: {
                Text("Metadata and downloads use your selected server's HTTPS API. Media streams use the pinned peer without silently falling back to HTTPS. Existing streams keep their captured route. This does not change your account's media permissions.")
            }
        }.navigationTitle("Direct Iroh transport")
        .onAppear { restore() }
        .onChange(of: store.accountContext) { _ in restore() }
        .onDisappear {
            loadTask?.cancel(); loadTask = nil; requestID = nil
            advertised = nil; loading = false; formContext = nil
        }
    }
    @MainActor private func restore() {
        loadTask?.cancel(); loadTask = nil; requestID = nil
        formContext = store.accountContext
        advertised = nil; loading = false; saved = false; failure = nil
        pin = store.session?.irohPeerId ?? ""
        address = store.session?.irohPeerAddress ?? ""
        enabled = !pin.isEmpty && !address.isEmpty
    }
    @MainActor private func loadServerConnection() async {
        guard !Task.isCancelled, !loading, formContext == store.accountContext else { return }
        let context = store.accountContext
        let request = UUID(); requestID = request; loading = true; failure = nil; saved = false
        defer { if requestID == request { loading = false; requestID = nil; loadTask = nil } }
        do {
            guard store.session?.server.lowercased().hasPrefix("https://") == true else {
                throw BluefinError.message("Use your server's HTTPS address to load its direct connection.")
            }
            let capability: BluefinCapabilities = try await store.authenticated("bluefinCapabilities")
            try Task.checkCancellation()
            guard requestID == request, formContext == context, store.accountContext == context else { return }
            guard let connection = capability.irohMediaTransport, connection.supported else {
                throw BluefinError.message("This server does not offer a supported direct Iroh connection for your account.")
            }
            advertised = connection
            pin = connection.endpointId; address = connection.directAddresses[0]; enabled = true
        } catch {
            guard requestID == request, formContext == context, store.accountContext == context, !Task.isCancelled else { return }
            advertised = nil
            failure = error.localizedDescription.contains("404") ? "Direct Iroh is not available on this server yet." : error.localizedDescription
        }
    }
    private func save() {
        failure = nil; saved = false
        do {
            guard formContext == store.accountContext else {
                throw BluefinError.message("Your account changed. Reopen transport settings before saving.")
            }
            if enabled {
                let pin = pin.trimmingCharacters(in: .whitespacesAndNewlines)
                let address = address.trimmingCharacters(in: .whitespacesAndNewlines)
                guard (32...128).contains(pin.count), pin.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }), ServerIrohTransport.validAddress(address) else {
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
