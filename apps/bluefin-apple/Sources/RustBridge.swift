import Foundation
import Security

enum RustBridge {
    /// Rust owns protocol validation, device profiles, playback planning, and API requests.
    /// A detached task keeps synchronous FFI off the main actor.
    static func call<T: Decodable & Sendable>(_ operation: String, session: AccountSession? = nil, fields: [String: Any] = [:], as type: T.Type = T.self) async throws -> T {
        let cleanup = ["releasePlayback", "stopPlaybackProxy", "downloadCancel", "cancelKaraokePreparation", "meshCloseResources", "meshLocalSignOut", "meshRevoke"].contains(operation)
        let permit: UUID?
        if cleanup { permit = nil } else { permit = try await CoreAdmission.shared.acquire() }
        defer { if let permit { Task { await CoreAdmission.shared.release(permit) } } }
        if !cleanup { try Task.checkCancellation() }
        var request = fields
        request["operation"] = operation
        request["relayAllowlist"] = Bundle.main.object(forInfoDictionaryKey: "BluefinMeshRelayAllowlist") as? [String] ?? []
        if let session {
            request["server"] = session.server
            request["token"] = session.token
            request["userId"] = session.userId
            request["deviceId"] = session.deviceId
            request["allowLocalHttp"] = session.allowHttp
            if let reference = session.meshSessionId {
                if !cleanup, try MeshRevocationVault.contains(reference) {
                    throw BluefinError.message("This device connection is being signed out.")
                }
                let mesh = try MeshSessionVault.load(reference: reference)
                guard mesh.sessionId == reference, mesh.apiOrigin == session.server,
                      mesh.deviceId == session.deviceId, mesh.userId == session.userId else {
                    throw BluefinError.message("The saved device connection does not match this account.")
                }
                request["mesh"] = mesh.fields
                request.removeValue(forKey: "token")
            }
            if session.meshSessionId == nil, operation == "playback", let pin = session.irohPeerId, let address = session.irohPeerAddress {
                request["iroh"] = ["peerId": pin, "peerAddress": address]
            }
        }
        if let allowHttp = request.removeValue(forKey: "allowHttp") { request["allowLocalHttp"] = allowHttp }
        let data = try JSONSerialization.data(withJSONObject: request)
        let (response, decoded) = try await Task.detached(priority: .userInitiated) { () -> (Data, Result<T, Error>) in
            guard let json = String(data: data, encoding: .utf8) else { throw BluefinError.message("The request could not be encoded.") }
            return try json.withCString { input in
                let deadline = Date().addingTimeInterval(60)
                while true {
                func invoke() throws -> [String: Any] {
                guard let output = bluefin_call(input) else { throw BluefinError.message("The media engine did not return a response.") }
                defer { bluefin_free(output) }
                let bytes = Data(String(cString: output).utf8)
                guard let envelope = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else { throw BluefinError.message("The media engine returned an invalid response.") }
                return envelope
                }
                let envelope = try invoke()
                if operation == "releasePlayback", envelope["error"] as? String == "Client busy", Date() < deadline {
                    Thread.sleep(forTimeInterval: 0.05)
                    continue
                }
                guard envelope["ok"] as? Bool == true else {
                    throw BluefinError.message(envelope["error"] as? String ?? "The request failed.")
                }
                let response = try JSONSerialization.data(withJSONObject: envelope["value"] ?? NSNull(), options: [.fragmentsAllowed])
                return (response, Result { try JSONDecoder().decode(T.self, from: response) })
                }
            }
        }.value
        if Task.isCancelled && !["releasePlayback", "stopPlaybackProxy", "meshCloseResources", "meshLocalSignOut", "meshRevoke"].contains(operation) {
            if operation == "playback" { try await cleanupPlaybackResponse(response, session: session) }
            throw CancellationError()
        }
        do { return try decoded.get() }
        catch {
            if operation == "playback" { try await cleanupPlaybackResponse(response, session: session) }
            throw BluefinError.message("The server returned an unexpected response.")
        }
    }

    static func notify(_ operation: String, session: AccountSession, fields: [String: Any] = [:]) async throws {
        let _: EmptyValue = try await call(operation, session: session, fields: fields)
    }
    static func cleanupPlaybackPlan(_ plan: PlaybackPlan, session: AccountSession) async throws {
        var failure: Error?
        if let proxy = plan.proxyId {
            do { let _: EmptyValue = try await call("stopPlaybackProxy", fields: ["proxyId": proxy]) }
            catch { failure = error }
        }
        do { try await notify("releasePlayback", session: session, fields: PlaybackCleanup.fields(plan)) }
        catch { failure = failure ?? error }
        if let failure { throw failure }
    }
    private static func cleanupPlaybackResponse(_ data: Data, session: AccountSession?) async throws {
        guard let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
        var failure: Error?
        if let proxy = value["proxyId"] as? String {
            do { let _: EmptyValue = try await call("stopPlaybackProxy", fields: ["proxyId": proxy]) }
            catch { failure = error }
        }
        if let session, let playSession = value["playSessionId"] as? String {
            var fields: [String: Any] = ["playSessionId": playSession, "playMethod": value["playMethod"] as? String ?? "DirectPlay"]
            if let live = (value["source"] as? [String: Any])?["LiveStreamId"] as? String { fields["liveStreamId"] = live }
            do { try await notify("releasePlayback", session: session, fields: fields) }
            catch { failure = failure ?? error }
        }
        if let failure { throw failure }
    }
}

struct EmptyValue: Decodable { init(from decoder: Decoder) throws {} }

enum SessionVault {
    private static let service = "org.glopyglerky.bluefin.session"
    static func load() -> AccountSession? { try? loadChecked() }
    static func loadChecked() throws -> AccountSession? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw BluefinError.message("Saved sign-in is unavailable. Unlock and retry.") }
        return try JSONDecoder().decode(AccountSession.self, from: data)
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
    static func clear() throws {
        let status = SecItemDelete(baseQuery as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw BluefinError.message("Your saved sign-in could not be removed. Try signing out again.")
        }
    }
    private static var baseQuery: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: "current"]
    }
}

extension RustBridge {
    static func decodeMeshImage(_ bytes: Data) async throws -> MeshInvitePreview {
        guard !bytes.isEmpty, bytes.count <= MeshImageInput.maximumBytes else {
            throw BluefinError.message("Choose a PNG or JPEG image smaller than 8 MiB.")
        }
        let permit = try await CoreAdmission.shared.acquire()
        defer { Task { await CoreAdmission.shared.release(permit) } }
        try Task.checkCancellation()
        let value = try await Task.detached(priority: .userInitiated) {
            try bytes.withUnsafeBytes { buffer -> MeshInvitePreview in
                guard let pointer = buffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                      let output = bluefin_decode_qr_image(pointer, buffer.count) else {
                    throw BluefinError.message("The connection image could not be read.")
                }
                defer { bluefin_free(output) }
                let response = Data(String(cString: output).utf8)
                guard let envelope = try JSONSerialization.jsonObject(with: response) as? [String: Any],
                      envelope["ok"] as? Bool == true, let value = envelope["value"] else {
                    throw BluefinError.message("Choose an image containing exactly one valid Bluefin invitation.")
                }
                return try JSONDecoder().decode(MeshInvitePreview.self,
                    from: JSONSerialization.data(withJSONObject: value))
            }
        }.value
        try Task.checkCancellation()
        return value
    }
}
