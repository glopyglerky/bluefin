#if (os(iOS) && !targetEnvironment(macCatalyst)) || os(watchOS)
import Foundation
import Security

struct ManagedPairingProof: Codable, Equatable, Sendable {
    let challenge: String
    let secret: String
    var body: [String: Any] { ["challenge": challenge, "secret": secret] }
    func validate() throws {
        guard (1...128).contains(challenge.utf8.count), (1...128).contains(secret.utf8.count),
              challenge.utf8.allSatisfy({ (33...126).contains($0) }),
              secret.utf8.allSatisfy({ (33...126).contains($0) }) else { throw CancellationError() }
    }
}
struct ManagedPairingStart: Decodable {
    let proof: ManagedPairingProof
    let device_id: String
    let expires_at: Int64
}
enum ManagedPairing {
    static func supported(_ server: URL) async throws -> Bool {
        struct Capabilities: Decodable { let managed_pairing_v1: Bool? }
        return try JSONDecoder().decode(Capabilities.self, from: await WatchAPI.request(server, "Bluefin/Pairing/Capabilities")).managed_pairing_v1 == true
    }
    static func cancel(_ server: URL, proof: ManagedPairingProof) async throws {
        struct Receipt: Decodable { let state: String }
        let data = try await WatchAPI.request(server, "Bluefin/Pairing/Cancel", method: "POST", body: proof.body)
        guard try JSONDecoder().decode(Receipt.self, from: data).state == "cancelled" else { throw CancellationError() }
    }
    static func exchange(_ server: URL, proof: ManagedPairingProof) async throws -> WatchAccount {
        struct Login: Decodable {
            let AccessToken: String
            let User: Viewer
            struct Viewer: Decodable { let Id: String; let Name: String }
        }
        let data = try await WatchAPI.request(server, "Bluefin/Pairing/Exchange", method: "POST", body: proof.body)
        let login = try JSONDecoder().decode(Login.self, from: data)
        guard !login.AccessToken.isEmpty, !login.User.Id.isEmpty else { throw CancellationError() }
        return WatchAccount(server: server, token: login.AccessToken, user: login.User.Id, name: login.User.Name, managedProof: proof)
    }
}

#if os(iOS)
struct PhonePairingOwner: Codable, Equatable {
    let id: UUID
    let server: URL
    let generation: UUID
    let proof: ManagedPairingProof
    let device: String
}
// One MainActor writer; a receipt removes only its exact captured owner from a fresh ledger.
@MainActor final class PhonePairingLedger {
    let read: () throws -> [PhonePairingOwner]
    let write: ([PhonePairingOwner]) throws -> Void
    private var draining = false
    init(read: @escaping () throws -> [PhonePairingOwner], write: @escaping ([PhonePairingOwner]) throws -> Void) {
        self.read = read; self.write = write
    }
    func remember(_ owner: PhonePairingOwner) throws {
        guard !draining else { throw CancellationError() }
        var rows = try read()
        if rows.contains(where: { $0.server == owner.server && $0.proof == owner.proof }) { throw CancellationError() }
        guard rows.count < 32 else { throw WatchFailure.message("Finish Watch cleanup before pairing another Watch.") }
        rows.append(owner); try write(rows)
    }
    func drain(keeping generation: UUID?, cancel: (PhonePairingOwner) async throws -> Void) async throws {
        guard !draining else { throw CancellationError() }
        draining = true; defer { draining = false }
        for owner in try read() where owner.generation != generation {
            try await cancel(owner)
            var current = try read()
            current.removeAll { $0 == owner }
            try write(current)
        }
    }
}
enum PhonePairingVault {
    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "org.glopyglerky.bluefin.watch-owners", kSecAttrAccount as String: "ledger"]
    }
    static func load() throws -> [PhonePairingOwner] {
        var q = query; q[kSecReturnData as String] = true; q[kSecMatchLimit as String] = kSecMatchLimitOne
        var value: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &value)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let data = value as? Data else { throw CancellationError() }
        let rows = try JSONDecoder().decode([PhonePairingOwner].self, from: data)
        guard rows.count <= 32 else { throw CancellationError() }
        return rows
    }
    static func save(_ rows: [PhonePairingOwner]) throws {
        let data = try JSONEncoder().encode(rows)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var q = query; q[kSecValueData as String] = data
            q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(q as CFDictionary, nil) == errSecSuccess else { throw CancellationError() }
        } else if status != errSecSuccess { throw CancellationError() }
    }
}
#endif
#endif
