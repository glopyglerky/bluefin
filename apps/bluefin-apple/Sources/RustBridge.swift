import Foundation
import Security

enum RustBridge {
    /// Rust owns protocol validation, device profiles, playback planning, and API requests.
    /// A detached task keeps synchronous FFI off the main actor.
    static func call<T: Decodable>(_ operation: String, session: AccountSession? = nil, fields: [String: Any] = [:], as type: T.Type = T.self) async throws -> T {
        var request = fields
        request["operation"] = operation
        if let session {
            request["server"] = session.server
            request["token"] = session.token
            request["userId"] = session.userId
            request["deviceId"] = session.deviceId
            request["allowLocalHttp"] = session.allowHttp
        }
        if let allowHttp = request.removeValue(forKey: "allowHttp") { request["allowLocalHttp"] = allowHttp }
        let data = try JSONSerialization.data(withJSONObject: request)
        let response = try await Task.detached(priority: .userInitiated) {
            guard let json = String(data: data, encoding: .utf8) else { throw BluefinError.message("The request could not be encoded.") }
            return try json.withCString { input in
                guard let output = bluefin_call(input) else { throw BluefinError.message("The media engine did not return a response.") }
                defer { bluefin_free(output) }
                let bytes = Data(String(cString: output).utf8)
                guard let envelope = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else { throw BluefinError.message("The media engine returned an invalid response.") }
                guard envelope["ok"] as? Bool == true else {
                    throw BluefinError.message(envelope["error"] as? String ?? "The request failed.")
                }
                return try JSONSerialization.data(withJSONObject: envelope["value"] ?? NSNull(), options: [.fragmentsAllowed])
            }
        }.value
        if Task.isCancelled {
            if operation == "playback", let plan = try? JSONDecoder().decode(PlaybackPlan.self, from: response), let proxy = plan.proxyId {
                let _: EmptyValue? = try? await call("stopPlaybackProxy", fields: ["proxyId": proxy])
            }
            throw CancellationError()
        }
        do { return try JSONDecoder().decode(T.self, from: response) }
        catch {
            if operation == "playback", let value = try? JSONSerialization.jsonObject(with: response) as? [String: Any], let proxy = value["proxyId"] as? String {
                let _: EmptyValue? = try? await call("stopPlaybackProxy", fields: ["proxyId": proxy])
            }
            throw BluefinError.message("The server returned an unexpected response.")
        }
    }

    static func notify(_ operation: String, session: AccountSession, fields: [String: Any] = [:]) async throws {
        let _: EmptyValue = try await call(operation, session: session, fields: fields)
    }
}

struct EmptyValue: Decodable { init(from decoder: Decoder) throws {} }

enum SessionVault {
    private static let service = "org.glopyglerky.bluefin.session"
    static func load() -> AccountSession? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let data = result as? Data else { return nil }
        return try? JSONDecoder().decode(AccountSession.self, from: data)
    }
    static func save(_ session: AccountSession) throws {
        let data = try JSONEncoder().encode(session)
        let update = [kSecValueData as String: data]
        let status = SecItemUpdate(baseQuery as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            var query = baseQuery
            query[kSecValueData as String] = data
            query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(query as CFDictionary, nil) == errSecSuccess else { throw BluefinError.message("Your sign-in could not be saved in Keychain.") }
        } else if status != errSecSuccess { throw BluefinError.message("Keychain could not update your sign-in.") }
    }
    static func clear() { SecItemDelete(baseQuery as CFDictionary) }
    private static var baseQuery: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: "current"]
    }
}
