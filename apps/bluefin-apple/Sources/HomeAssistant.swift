import SwiftUI
import Security

private struct HomeAssistantConnection: Codable {
    var server: String
    var token: String
    var allowHttp: Bool
    var fields: [String: Any] { ["server": server, "token": token, "allowLocalHttp": allowHttp] }
}

private struct HomeAssistantEntity: Decodable, Identifiable {
    var entityId: String
    var name: String
    var state: String
    var title: String?
    var volume: Double?
    var id: String { entityId }
}

struct HomeAssistantView: View {
    @State private var connection = HomeAssistantVault.load()
    @State private var server = ""
    @State private var token = ""
    @State private var allowHttp = false
    @State private var entities: [HomeAssistantEntity] = []
    @State private var busy = false
    @State private var problem: String?
    var body: some View {
        Form {
            if connection == nil {
                Section("Connect Home Assistant") {
                    TextField("https://home.example.com", text: $server).bluefinAddressInput()
                    SecureField("Home Assistant access token", text: $token)
                    if server.lowercased().hasPrefix("http:") { Toggle("Allow trusted local HTTP", isOn: $allowHttp) }
                    Text("Use a Home Assistant token for the account whose media players you want to control. Bluefin saves it in this device's Keychain.").font(.footnote).foregroundStyle(.secondary)
                    Button("Connect") { Task { await connect() } }.disabled(busy || server.isEmpty || token.isEmpty)
                }
            } else {
                Section("Media players") {
                    ForEach(entities) { entity in
                        VStack(alignment: .leading, spacing: 12) {
                            Text(entity.name).font(.headline)
                            Text(entity.title ?? entity.state).font(.subheadline).foregroundStyle(.secondary)
                            HStack(spacing: 20) {
                                Button { Task { await control(entity, service: "media_play_pause") } } label: { Image(systemName: "playpause.fill").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Play or pause \(entity.name)")
                                Button { Task { await control(entity, service: "media_next_track") } } label: { Image(systemName: "forward.end.fill").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Next track on \(entity.name)")
                                Button { Task { await control(entity, service: "media_stop") } } label: { Image(systemName: "stop.fill").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Stop \(entity.name)")
                            }.disabled(busy || ["unavailable", "unknown"].contains(entity.state))
                        }.padding(.vertical, 6)
                    }
                    if !busy && entities.isEmpty { Text("No media players are available to this Home Assistant account.").foregroundStyle(.secondary) }
                    Button("Refresh") { Task { await refresh() } }.disabled(busy)
                }
                Section {
                    Text("Controls apply only to the device you select. Bluefin does not publish your library token or a private stream URL to Home Assistant.").font(.footnote).foregroundStyle(.secondary)
                    Button("Disconnect Home Assistant", role: .destructive) { HomeAssistantVault.clear(); connection = nil; entities = [] }
                }
            }
            if busy { ProgressView() }
            if let problem { Section { Text(problem).foregroundColor(.red) } }
        }.navigationTitle("Home Assistant").task { if let connection { server = connection.server; await refresh() } }
    }
    private func connect() async {
        let candidate = HomeAssistantConnection(server: server.trimmingCharacters(in: .whitespacesAndNewlines), token: token, allowHttp: allowHttp)
        busy = true; problem = nil
        defer { busy = false; token = "" }
        do {
            let result: [HomeAssistantEntity] = try await RustBridge.call("homeAssistantStates", fields: candidate.fields)
            try HomeAssistantVault.save(candidate)
            connection = candidate
            entities = result
        } catch { problem = error.localizedDescription }
    }
    private func refresh() async {
        guard let connection else { return }
        busy = true; problem = nil
        defer { busy = false }
        do { entities = try await RustBridge.call("homeAssistantStates", fields: connection.fields) }
        catch { problem = error.localizedDescription }
    }
    private func control(_ entity: HomeAssistantEntity, service: String) async {
        guard !busy, let connection else { return }
        busy = true; problem = nil
        do {
            var fields = connection.fields
            fields["entityId"] = entity.entityId; fields["service"] = service
            let _: EmptyValue = try await RustBridge.call("homeAssistantControl", fields: fields)
            busy = false
            await refresh()
        } catch { busy = false; problem = error.localizedDescription }
    }
}

private enum HomeAssistantVault {
    private static var query: [String: Any] { [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "org.glopyglerky.bluefin.home-assistant", kSecAttrAccount as String: "connection"] }
    static func load() -> HomeAssistantConnection? {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(request as CFDictionary, &result) == errSecSuccess, let data = result as? Data else { return nil }
        return try? JSONDecoder().decode(HomeAssistantConnection.self, from: data)
    }
    static func save(_ connection: HomeAssistantConnection) throws {
        let data = try JSONEncoder().encode(connection)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var request = query
            request[kSecValueData as String] = data
            request[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(request as CFDictionary, nil) == errSecSuccess else { throw BluefinError.message("Home Assistant could not be saved in Keychain.") }
        } else if status != errSecSuccess { throw BluefinError.message("Keychain could not update this connection.") }
    }
    static func clear() { SecItemDelete(query as CFDictionary) }
}
