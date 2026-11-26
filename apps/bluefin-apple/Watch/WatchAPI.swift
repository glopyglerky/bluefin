import Foundation
import Security

struct WatchAccount: Codable, Equatable, Sendable {
    var server: URL
    var token: String
    var user: String
    var name: String
    var managedProof: ManagedPairingProof? = nil
    var companionGeneration: String? = nil
}
struct WatchTrack: Decodable, Identifiable {
    var Id: String
    var Name: String
    var `Type`: String?
    var Album: String?
    var Artists: [String]?
    var RunTimeTicks: Int64?
    var UserData: UserDataValue?
    struct UserDataValue: Decodable { var IsFavorite: Bool? }
    var id: String { Id }
}
struct WatchPage: Decodable { var Items: [WatchTrack]; var TotalRecordCount: Int? }
struct WatchLease: Decodable {
    var version: Int
    var mediaUrl: String
    var releaseUrl: String
}
struct WatchQueue: Decodable {
    var protocolVersion: Int
    var queue: Queue
    struct Queue: Decodable { var itemIds: [String]; var cursor: Int? }
}
enum WatchFailure: LocalizedError {
    case unauthorized
    case message(String)
    var errorDescription: String? { if case let .message(text) = self { return text }; return "The server no longer authorizes this sign-in." }
}
// Refuse redirects so account credentials cannot cross origins.
final class WatchTransport: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    private let configuration: URLSessionConfiguration
    init(configuration: URLSessionConfiguration = .ephemeral, deadline: TimeInterval = 30) {
        precondition(deadline > 0 && deadline <= 30)
        let c = configuration.copy() as! URLSessionConfiguration
        c.urlCache = nil; c.httpCookieStorage = nil
        c.timeoutIntervalForRequest = min(15, deadline)
        c.timeoutIntervalForResource = deadline
        c.httpMaximumConnectionsPerHost = 4
        self.configuration = c
        super.init()
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
    lazy var session: URLSession = {
        URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }()
    func data(for request: URLRequest) async throws -> Data {
        let (bytes, response) = try await session.bytes(for: request)
        defer { bytes.task.cancel() }
        if (response as? HTTPURLResponse)?.statusCode == 401 { throw WatchFailure.unauthorized }
        guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode) else {
            throw WatchFailure.message("Server request failed. Check your account and server support.")
        }
        let limit = 2_000_000
        guard response.expectedContentLength <= Int64(limit) else {
            throw WatchFailure.message("Server response exceeds the Watch limit.")
        }
        var data = Data()
        data.reserveCapacity(Int(max(0, response.expectedContentLength)))
        for try await byte in bytes {
            guard data.count < limit else { throw WatchFailure.message("Server response exceeds the Watch limit.") }
            data.append(byte)
        }
        return data
    }
}
enum WatchKeychain {
    static var query: [String: Any] { [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "org.glopyglerky.bluefin.watch", kSecAttrAccount as String: "viewer"] }
    static func load(slot: String = "viewer") -> WatchAccount? { try? loadChecked(slot: slot) }
    static func loadChecked(slot: String = "viewer") throws -> WatchAccount? {
        var query = Self.query; query[kSecAttrAccount as String] = slot
        var q = query; q[kSecReturnData as String] = true; q[kSecMatchLimit as String] = kSecMatchLimitOne
        var data: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &data)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = data as? Data else { throw WatchFailure.message("Saved Watch ownership is unavailable. Unlock and retry.") }
        return try JSONDecoder().decode(WatchAccount.self, from: data)
    }
    static func save(_ account: WatchAccount, slot: String = "viewer") throws {
        var query = Self.query; query[kSecAttrAccount as String] = slot
        let data = try JSONEncoder().encode(account)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var q = query; q[kSecValueData as String] = data
            q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(q as CFDictionary, nil) == errSecSuccess else { throw WatchFailure.message("Could not save account.") }
        } else if status != errSecSuccess { throw WatchFailure.message("Could not update account.") }
    }
    static func clear(slot: String = "viewer") throws {
        var query = Self.query; query[kSecAttrAccount as String] = slot
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw WatchFailure.message("Could not remove the saved sign-in. Try signing out again.")
        }
    }
}
struct WatchAPI {
    static let transport = WatchTransport()
    static func server(_ value: String) throws -> URL {
        guard let c = URLComponents(string: value), c.scheme == "https", c.host != nil,
              c.user == nil, c.password == nil, c.query == nil, c.fragment == nil,
              let url = c.url else { throw WatchFailure.message("Enter an HTTPS server address without credentials or query parameters.") }
        return url
    }
    static func request(_ server: URL, _ path: String, account: WatchAccount? = nil, method: String = "GET", body: [String: Any]? = nil, query: [URLQueryItem] = []) async throws -> Data {
        _ = try self.server(server.absoluteString)
        guard account == nil || account?.server == server else { throw WatchFailure.message("Account server mismatch.") }
        var c = URLComponents(url: server.appendingPathComponent(path), resolvingAgainstBaseURL: false)!
        if !query.isEmpty { c.queryItems = query }
        var r = URLRequest(url: c.url!); r.httpMethod = method
        r.setValue("application/json", forHTTPHeaderField: "Content-Type")
        var authorization = "MediaBrowser Client=\"Bluefin Watch\", Device=\"Apple Watch\", DeviceId=\"\(deviceID)\", Version=\"0.1.0\""
        if let account {
            guard (1...4096).contains(account.token.utf8.count), account.token.utf8.allSatisfy({
                (33...126).contains($0) && $0 != 34 && $0 != 44 && $0 != 92
            }) else { throw WatchFailure.message("The server returned an unsupported session token.") }
            authorization += ", Token=\"\(account.token)\""
        }
        r.setValue(authorization, forHTTPHeaderField: "Authorization")
        if let body { r.httpBody = try JSONSerialization.data(withJSONObject: body) }
        return try await transport.data(for: r)
    }
    static var deviceID: String {
        if let id = UserDefaults.standard.string(forKey: "watch.device"), UUID(uuidString: id) != nil { return id }
        let id = UUID().uuidString; UserDefaults.standard.set(id, forKey: "watch.device"); return id
    }
    struct AccountOptions: Decodable {
        var allowSignups: Bool
        var passkeys: Bool
        var quickConnect: Bool
    }

    struct DeviceRequest {
        var server: URL
        var code: String
        var qr: URL?
        var ticket: String?
        var pairing: ManagedPairingProof?
        var secret: String?

        var body: [String: Any]? {
            guard let ticket, let pairing else { return nil }
            return ["ticket": ticket, "pairing": pairing.body]
        }
    }

    static func accountOptions(_ address: URL) async throws -> (String, AccountOptions) {
        struct Info: Decodable { var ServerName: String }
        let info = try JSONDecoder().decode(Info.self, from: await request(address, "System/Info/Public"))
        let options: AccountOptions
        do {
            options = try JSONDecoder().decode(AccountOptions.self, from: await request(address, "Bluefin/Accounts/Options"))
        } catch is CancellationError { throw CancellationError() }
        catch {
            let legacy = try? await request(address, "QuickConnect/Enabled")
            options = AccountOptions(allowSignups: false, passkeys: false,
                                     quickConnect: legacy.flatMap { try? JSONDecoder().decode(Bool.self, from: $0) } == true)
        }
        return (info.ServerName, options)
    }

    static func createAccount(_ address: URL, name: String, password: String) async throws {
        _ = try await request(address, "Bluefin/Accounts/Create", method: "POST",
                              body: ["username": name, "password": password])
    }

    static func beginSignIn(_ address: URL, passkey: Bool) async throws -> DeviceRequest {
        if passkey {
            struct Start: Decodable {
                struct Proof: Decodable { var ticket: String; var pairing: ManagedPairingProof }
                var code: String; var qr_url: URL; var proof: Proof
            }
            let response = try JSONDecoder().decode(Start.self, from: await request(address, "Bluefin/Accounts/Passkey/Initiate", method: "POST", body: [:]))
            let ticket = response.proof.ticket.replacingOccurrences(of: "-", with: "").lowercased()
            let expected = address.appendingPathComponent("Bluefin/Accounts/Passkey/QR").appendingPathComponent(ticket)
            let device = DeviceRequest(server: address, code: response.code, qr: response.qr_url,
                                       ticket: response.proof.ticket, pairing: response.proof.pairing)
            guard expected == response.qr_url else {
                try? await cancelSignIn(device)
                throw WatchFailure.message("The server returned an invalid passkey link.")
            }
            return device
        }
        struct Code: Decodable { var Secret: String; var Code: String }
        let reply = try JSONDecoder().decode(Code.self, from: await request(address, "QuickConnect/Initiate", method: "POST", body: [:]))
        return DeviceRequest(server: address, code: reply.Code, secret: reply.Secret)
    }

    static func finishSignIn(_ device: DeviceRequest) async throws -> WatchAccount? {
        if let proof = device.body {
            struct Login: Decodable {
                struct User: Decodable { var Id: String; var Name: String }
                var pending: Bool?; var AccessToken: String?; var User: User?
            }
            let reply = try JSONDecoder().decode(Login.self, from: await request(device.server,
                "Bluefin/Accounts/Passkey/Exchange", method: "POST", body: proof))
            if reply.pending == true { return nil }
            guard let user = reply.User, let token = reply.AccessToken, !token.isEmpty else { throw WatchFailure.unauthorized }
            return WatchAccount(server: device.server, token: token, user: user.Id, name: user.Name, managedProof: device.pairing)
        }
        guard let secret = device.secret else { throw WatchFailure.unauthorized }
        struct Status: Decodable { var Authenticated: Bool }
        let status = try JSONDecoder().decode(Status.self, from: await request(device.server, "QuickConnect/Connect", query: [.init(name: "secret", value: secret)]))
        return status.Authenticated ? try await exchange(device.server, secret: secret) : nil
    }

    static func cancelSignIn(_ request: DeviceRequest) async throws {
        if let proof = request.body {
            _ = try await self.request(request.server, "Bluefin/Accounts/Passkey/Cancel", method: "POST", body: proof)
        }
    }

    static func signIn(server: String, username: String, password: String) async throws -> WatchAccount {
        let url = try self.server(server)
        struct Login: Decodable { var AccessToken: String; var User: User; struct User: Decodable { var Id: String; var Name: String } }
        let data = try await request(url, "Users/AuthenticateByName", method: "POST", body: ["Username": username, "Pw": password])
        let login = try JSONDecoder().decode(Login.self, from: data)
        guard !login.AccessToken.isEmpty else { throw WatchFailure.message("Server returned no session.") }
        return WatchAccount(server: url, token: login.AccessToken, user: login.User.Id, name: login.User.Name)
    }
    static func page(_ account: WatchAccount, favorites: Bool, search: String = "", start: Int = 0) async throws -> WatchPage {
        var q = [URLQueryItem(name: "IncludeItemTypes", value: "Audio"), .init(name: "Recursive", value: "true"), .init(name: "Limit", value: "50"), .init(name: "Fields", value: "UserData")]
        q.append(.init(name: "StartIndex", value: String(start)))
        q.append(.init(name: "SearchTerm", value: String(search.prefix(200))))
        if favorites { q.append(.init(name: "Filters", value: "IsFavorite")) }
        return try JSONDecoder().decode(WatchPage.self, from: await request(account.server, "Users/\(account.user)/Items", account: account, query: q))
    }
    static func items(_ account: WatchAccount, favorites: Bool) async throws -> [WatchTrack] { try await page(account, favorites: favorites).Items }
    static func exchange(_ server: URL, secret: String) async throws -> WatchAccount {
        struct Login: Decodable { var AccessToken: String; var User: User; struct User: Decodable { var Id: String; var Name: String } }
        let login = try JSONDecoder().decode(Login.self, from: await request(server, "Users/AuthenticateWithQuickConnect", method: "POST", body: ["Secret": secret]))
        guard !login.AccessToken.isEmpty, !login.User.Id.isEmpty else { throw WatchFailure.message("Server returned no session.") }
        return WatchAccount(server: server, token: login.AccessToken, user: login.User.Id, name: login.User.Name)
    }
    static func queue(_ account: WatchAccount) async throws -> [WatchTrack] {
        let saved = try JSONDecoder().decode(WatchQueue.self, from: await request(account.server, "Bluefin/Music/Queue", account: account))
        guard saved.protocolVersion == 1, saved.queue.itemIds.count <= 500 else { throw WatchFailure.message("Unsupported server queue.") }
        let ids = Array(saved.queue.itemIds.prefix(50))
        guard !ids.isEmpty else { return [] }
        let page = try JSONDecoder().decode(WatchPage.self, from: await request(account.server, "Users/\(account.user)/Items", account: account, query: [.init(name: "Ids", value: ids.joined(separator: ",")), .init(name: "IncludeItemTypes", value: "Audio"), .init(name: "Limit", value: "50")]))
        return ids.compactMap { id in page.Items.first { $0.Id == id && $0.Type == "Audio" } }
    }
    static func capabilityURL(_ path: String, account: WatchAccount, prefix: String) throws -> URL {
        // Only the documented opaque Bluefin capability routes, under the chosen base path.
        let parts = path.split(separator: "/")
        let expected = prefix.split(separator: "/")
        guard path.hasPrefix(prefix), !path.contains("?"), !path.contains("#"), !path.contains("%"),
              !path.contains("\\"), parts.count == expected.count + (prefix.contains("/media/") ? 2 : 1),
              Array(parts.prefix(expected.count)) == expected,
              parts[expected.count].count == 64,
              parts[expected.count].utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              !prefix.contains("/media/") || parts.last == "0" else { throw WatchFailure.message("Invalid playback lease.") }
        return account.server.appendingPathComponent(String(path.dropFirst()))
    }
}
