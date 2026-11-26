import Foundation
import Security

/// Codable JSON at the Rust/native boundary. No protocol decisions live here.
enum MeshJSON: Codable, Sendable, Equatable {
    case object([String: MeshJSON]), array([MeshJSON]), string(String)
    case integer(Int64), unsigned(UInt64), number(Double), bool(Bool), null
    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let v = try? value.decode(Bool.self) { self = .bool(v) }
        else if let v = try? value.decode(Int64.self) { self = .integer(v) }
        else if let v = try? value.decode(UInt64.self) { self = .unsigned(v) }
        else if let v = try? value.decode(Double.self) { self = .number(v) }
        else if let v = try? value.decode(String.self) { self = .string(v) }
        else if let v = try? value.decode([String: MeshJSON].self) { self = .object(v) }
        else { self = .array(try value.decode([MeshJSON].self)) }
    }
    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .object(let v): try value.encode(v)
        case .array(let v): try value.encode(v)
        case .string(let v): try value.encode(v)
        case .integer(let v): try value.encode(v)
        case .unsigned(let v): try value.encode(v)
        case .number(let v): try value.encode(v)
        case .bool(let v): try value.encode(v)
        case .null: try value.encodeNil()
        }
    }
    subscript(_ key: String) -> MeshJSON {
        if case .object(let values) = self { return values[key] ?? .null }
        return .null
    }
    var string: String? { if case .string(let value) = self { return value }; return nil }
    var foundation: Any {
        switch self {
        case .object(let values): return values.mapValues(\.foundation)
        case .array(let values): return values.map(\.foundation)
        case .string(let value): return value
        case .integer(let value): return value
        case .unsigned(let value): return value
        case .number(let value): return value
        case .bool(let value): return value
        case .null: return NSNull()
        }
    }
    static func from(_ value: Any) throws -> MeshJSON {
        try JSONDecoder().decode(MeshJSON.self, from: JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]))
    }
}
struct MeshIdentity: Codable, Sendable {
    let deviceSeed: String
    let deviceId: String
    let beginRequestId: String
    let finishRequestId: String
}
struct MeshInvitePreview: Codable, Sendable {
    let code: String
    let link: String
    let expiresAtUnixMs: UInt64
    let intent: String
    let accountLinked: Bool
    let singleUse: Bool
    let service: String
}
struct MeshBeginResponse: Codable, Sendable {
    let challengeId: String
    let expiresAtUnixMs: UInt64
    let passkey: MeshJSON
    let deviceProofMessage: String
    var serverAddress: MeshJSON
    let apiOrigin: String
}
struct MeshPreparedFinish: Codable, Sendable { let request: MeshJSON }
struct MeshCompletion: Codable, Sendable {
    let kind: String
    var id: String?
    var userId: String?
    var username: String?
    var expiresAtUnixMs: UInt64?
    var handle: String?
    var state: String?
    var apiOrigin: String?
}
struct MeshSignupStatus: Codable, Sendable {
    let state: String
    var session: MeshCompletion?
}
struct MeshSavedSession: Codable, Sendable {
    var serverAddress: MeshJSON
    let sessionId: String
    let deviceSeed: String
    let apiOrigin: String
    let deviceId: String
    let userId: String
    var fields: [String: Any] {
        ["serverAddress": serverAddress.foundation, "sessionId": sessionId,
         "deviceSeed": deviceSeed, "apiOrigin": apiOrigin]
    }
}
struct MeshPending: Codable {
    let identity: MeshIdentity
    var code: String?
    var accountName: String?
    var begin: MeshBeginResponse?
    var prepared: MeshPreparedFinish?
    var signupHandle: String?
    var completion: MeshCompletion?
}
enum MeshPendingVault {
    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "org.glopyglerky.bluefin.mesh.pending",
         kSecAttrAccount as String: "current"]
    }
    static func load() throws -> MeshPending? {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else {
            throw BluefinError.message("Unlock your device to resume this connection.")
        }
        return try JSONDecoder().decode(MeshPending.self, from: data)
    }
    static func save(_ pending: MeshPending) throws {
        let data = try JSONEncoder().encode(pending)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var request = query
            request[kSecValueData as String] = data
            request[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(request as CFDictionary, nil) == errSecSuccess else {
                throw BluefinError.message("Your connection could not be saved securely.")
            }
        } else if status != errSecSuccess { throw BluefinError.message("Your saved connection could not be updated.") }
    }
    static func clear() throws {
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw BluefinError.message("Your pending connection could not be removed.")
        }
    }
}

enum MeshSessionVault {
    private static func query(_ reference: String) throws -> [String: Any] {
        guard reference.utf8.count == 64,
              reference.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
            throw BluefinError.message("The saved device connection is invalid.")
        }
        return [kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "org.glopyglerky.bluefin.mesh.device",
            kSecAttrAccount as String: reference,
            kSecAttrSynchronizable as String: false]
    }
    static func load(reference: String) throws -> MeshSavedSession {
        var request = try query(reference)
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(request as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data, data.count <= 65536 else {
            throw BluefinError.message("Unlock this device to use its saved Bluefin connection. If it was removed, connect again.")
        }
        let saved = try JSONDecoder().decode(MeshSavedSession.self, from: data)
        guard saved.sessionId == reference else { throw BluefinError.message("The saved device connection is invalid.") }
        return saved
    }
    static func save(_ saved: MeshSavedSession) throws {
        let data = try JSONEncoder().encode(saved)
        guard data.count <= 65536 else { throw BluefinError.message("The saved device connection is too large.") }
        let key = try query(saved.sessionId)
        let status = SecItemUpdate(key as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var request = key
            request[kSecValueData as String] = data
            request[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(request as CFDictionary, nil) == errSecSuccess else {
                throw BluefinError.message("This device's connection could not be saved securely.")
            }
        } else if status != errSecSuccess {
            throw BluefinError.message("This device's connection could not be updated.")
        }
    }
    static func clear(reference: String) throws {
        let status = SecItemDelete(try query(reference) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw BluefinError.message("This device's connection could not be removed.")
        }
    }
}
