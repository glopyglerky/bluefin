import Foundation
import Combine
import Security

struct MeshRevocation: Codable, Sendable {
    let connection: MeshSavedSession
    let requestId: String
}
private struct MeshRequestIdentity: Decodable, Sendable { let requestId: String }

/// This store is read only by sign-out reconciliation. Ordinary API requests
/// cannot recover a device identity from a pending revocation.
enum MeshRevocationVault {
    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "org.glopyglerky.bluefin.mesh.revocations",
         kSecAttrAccount as String: "pending", kSecAttrSynchronizable as String: false]
    }
    static func load() throws -> [MeshRevocation] {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let data = result as? Data, data.count <= 1024 * 1024 else {
            throw BluefinError.message("Unlock this device to reconcile its saved sign-out requests.")
        }
        let values = try JSONDecoder().decode([MeshRevocation].self, from: data)
        guard values.count <= 16, Set(values.map { $0.connection.sessionId }).count == values.count else {
            throw BluefinError.message("The saved sign-out requests are invalid.")
        }
        return values
    }
    static func contains(_ reference: String) throws -> Bool {
        try load().contains { $0.connection.sessionId == reference }
    }
    static func save(_ values: [MeshRevocation]) throws {
        guard values.count <= 16 else { throw BluefinError.message("Reconnect to finish earlier sign-out requests before adding another.") }
        let data = try JSONEncoder().encode(values)
        guard data.count <= 1024 * 1024 else { throw BluefinError.message("The saved sign-out requests are too large.") }
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var request = query
            request[kSecValueData as String] = data
            request[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(request as CFDictionary, nil) == errSecSuccess else {
                throw BluefinError.message("The sign-out request could not be saved securely.")
            }
        } else if status != errSecSuccess {
            throw BluefinError.message("The sign-out request could not be saved securely.")
        }
    }
}

@MainActor final class MeshRevocationQueue: ObservableObject {
    static let shared = MeshRevocationQueue()
    @Published private(set) var count = 0
    @Published private(set) var busy = false
    @Published private(set) var problem: String?

    func prepare(_ session: AccountSession) async throws {
        guard let reference = session.meshSessionId else { return }
        if try MeshRevocationVault.contains(reference) { return }
        let connection = try MeshSessionVault.load(reference: reference)
        try await prepareConnection(connection)
    }

    func prepareConnection(_ connection: MeshSavedSession) async throws {
        let reference = connection.sessionId
        if try MeshRevocationVault.contains(reference) { return }
        let id: MeshRequestIdentity = try await RustBridge.call("meshRequestIdCreate")
        var values = try MeshRevocationVault.load()
        if values.contains(where: { $0.connection.sessionId == reference }) { return }
        values.append(MeshRevocation(connection: connection, requestId: id.requestId))
        try MeshRevocationVault.save(values)
        count = values.count
    }

    func retry() async {
        guard !busy else { return }
        busy = true; problem = nil
        defer { busy = false }
        do {
            let values = try MeshRevocationVault.load()
            count = values.count
            for request in values {
                do {
                    let _: EmptyValue = try await RustBridge.call("meshRevoke", fields: [
                        "mesh": request.connection.fields, "requestId": request.requestId])
                    var current = try MeshRevocationVault.load()
                    current.removeAll {
                        $0.connection.sessionId == request.connection.sessionId && $0.requestId == request.requestId
                    }
                    try MeshRevocationVault.save(current)
                    count = current.count
                } catch {
                    problem = "Signed out on this device. Remote revocation is waiting for a secure connection."
                }
            }
        } catch { problem = error.localizedDescription }
    }

    /// Explicitly abandons remote reconciliation. This cannot claim the server
    /// revoked the session; it destroys the only locally retained cleanup key.
    func forget() throws {
        guard !busy else { throw BluefinError.message("Wait for the current sign-out check to finish.") }
        try MeshRevocationVault.save([])
        count = 0
        problem = "Local sign-out records were erased. Remote revocation was not confirmed."
    }
}
