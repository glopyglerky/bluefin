#if os(iOS) && canImport(AppIntents)
import AppIntents
import LocalAuthentication
import Foundation

@available(iOS 16.0, *)
struct BluefinLibraryEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Bluefin library item"
    static var defaultQuery = BluefinLibraryQuery()
    var id: String
    var title: String
    var kind: String
    var displayRepresentation: DisplayRepresentation { .init(title: "\(title)", subtitle: "\(kind)") }
}

@available(iOS 16.0, *)
struct BluefinLibraryQuery: EntityStringQuery {
    func suggestedEntities() async throws -> [BluefinLibraryEntity] { [] }
    @MainActor func entities(matching string: String) async throws -> [BluefinLibraryEntity] {
        let (page, scope) = try await NativeLibraryAccess.search(string)
        return page.Items.map { .init(id: scope + ":" + $0.Id, title: $0.title, kind: $0.Type ?? "Library item") }
    }
    @MainActor func entities(for identifiers: [String]) async throws -> [BluefinLibraryEntity] {
        let session = try await NativeLibraryAccess.authorize()
        let scope = NativeLibraryAccess.scope(session)
        var result: [BluefinLibraryEntity] = []
        for id in identifiers.prefix(60) {
            let parts = id.split(separator: ":", maxSplits: 1).map(String.init)
            guard parts.count == 2, parts[0] == scope else { continue }
            let item: MediaItem = try await RustBridge.call("item", session: session, fields: ["itemId": parts[1]])
            try NativeLibraryAccess.check(session)
            result.append(.init(id: id, title: item.title, kind: item.Type ?? "Library item"))
        }
        return result
    }
}

@available(iOS 16.0, *)
@MainActor enum NativeLibraryAccess {
    static func scope(_ a: AccountSession) -> String { NativeIntegrations.digest(a.server + "\u{0}" + a.userId + "\u{0}" + a.token) }
    static func check(_ a: AccountSession) throws {
        guard let now = SessionVault.load(), scope(now) == scope(a) else { throw CancellationError() }
        try Task.checkCancellation()
    }
    static func authorize() async throws -> AccountSession {
        guard let a = SessionVault.load() else { throw BluefinError.message("Sign in to Bluefin first.") }
        let context = LAContext()
        guard try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: "Access your Bluefin library") else { throw CancellationError() }
        try check(a); return a
    }
    static func search(_ term: String) async throws -> (ItemPage, String) {
        let term = term.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !term.isEmpty, term.count <= 200 else { throw BluefinError.message("Enter a library search of up to 200 characters.") }
        let a = try await authorize()
        let page: ItemPage = try await RustBridge.call("items", session: a, fields: ["searchTerm": term, "startIndex": 0, "limit": 60])
        try check(a)
        return (page, scope(a))
    }
}

@available(iOS 16.0, *)
struct SearchBluefinLibraryIntent: AppIntent {
    static var title: LocalizedStringResource = "Search Bluefin library"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Search") var search: String
    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[BluefinLibraryEntity]> & ProvidesDialog {
        let (page, scope) = try await NativeLibraryAccess.search(search)
        let values = page.Items.map { BluefinLibraryEntity(id: scope + ":" + $0.Id, title: $0.title, kind: $0.Type ?? "Library item") }
        let message: String
        if let total = page.TotalRecordCount, total >= values.count { message = "Found \(total) library results. Returning \(values.count)." }
        else { message = "Returning \(values.count) library results. The total is unknown." }
        return .result(value: values, dialog: IntentDialog(stringLiteral: message))
    }
}

@available(iOS 16.0, *)
struct BluefinItemInformationIntent: AppIntent {
    static var title: LocalizedStringResource = "Get Bluefin item information"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Item") var item: BluefinLibraryEntity
    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        let a = try await NativeLibraryAccess.authorize()
        let prefix = NativeLibraryAccess.scope(a) + ":"
        guard item.id.hasPrefix(prefix) else { throw CancellationError() }
        let detail: MediaItem = try await RustBridge.call("item", session: a, fields: ["itemId": String(item.id.dropFirst(prefix.count))])
        try NativeLibraryAccess.check(a)
        let text = "\(detail.title). \(detail.Type ?? "Library item"). Available in your authorized library. Playback availability depends on your server and device. \(detail.Overview ?? "No description supplied by the server.")"
        return .result(dialog: IntentDialog(stringLiteral: String(text.prefix(2000))))
    }
}

@available(iOS 16.0, *)
struct PlayBluefinItemIntent: AppIntent {
    static var title: LocalizedStringResource = "Play Bluefin library item"
    static var openAppWhenRun = true
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Item") var item: BluefinLibraryEntity
    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        let store = try NativeIntegrations.shared.requireStore()
        guard let a = store.session, item.id.hasPrefix(NativeLibraryAccess.scope(a) + ":") else { throw CancellationError() }
        let epoch = store.accountContext
        let id = String(item.id.dropFirst(NativeLibraryAccess.scope(a).count + 1))
        let detail: MediaItem = try await RustBridge.call("item", session: a, fields: ["itemId": id])
        guard store.accountContext == epoch, !detail.isContainer else { throw BluefinError.message("Choose a playable library item.") }
        await store.play(detail)
        guard store.accountContext == epoch, store.playback?.item.Id == id else { throw BluefinError.message("Playback did not start. Open Bluefin to check your server and audio route.") }
        return .result(dialog: "Playing \(detail.title).")
    }
}

@available(iOS 16.0, *)
struct BluefinRequestActionEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Bluefin server action"
    static var defaultQuery = BluefinRequestActionQuery()
    var id: String
    var title: String
    var displayRepresentation: DisplayRepresentation { .init(title: "\(title)") }
}
@available(iOS 16.0, *)
struct BluefinRequestActionQuery: EntityStringQuery {
    func suggestedEntities() async throws -> [BluefinRequestActionEntity] { [] }
    @MainActor func entities(matching string: String) async throws -> [BluefinRequestActionEntity] {
        guard !string.isEmpty, string.count <= 160 else { throw BluefinError.message("Enter a title of up to 160 characters.") }
        let a = try await NativeLibraryAccess.authorize()
        let caps: BluefinCapabilities = try await RustBridge.call("bluefinCapabilities", session: a)
        try NativeLibraryAccess.check(a)
        guard caps.protocolVersion == 1, caps.discovery?.search == true, caps.discovery?.requests == true else {
            throw BluefinError.message("Media requests are unavailable on this server or for your account.")
        }
        let page: ServerDiscoveryPage = try await RustBridge.call("serverDiscoverySearch", session: a, fields: ["query": string, "page": 1])
        try NativeLibraryAccess.check(a)
        return page.results.flatMap { item in item.actions.map { action in
            BluefinRequestActionEntity(id: NativeLibraryAccess.scope(a) + ":" + action.id, title: item.title + " · " + action.label)
        } }
    }
    @MainActor func entities(for identifiers: [String]) async throws -> [BluefinRequestActionEntity] {
        let a = try await NativeLibraryAccess.authorize()
        let prefix = NativeLibraryAccess.scope(a) + ":"
        var values: [BluefinRequestActionEntity] = []
        for id in identifiers.prefix(20) where id.hasPrefix(prefix) {
            let form: ServerActionForm = try await RustBridge.call("serverDiscoveryAction", session: a, fields: ["actionId": String(id.dropFirst(prefix.count))])
            try NativeLibraryAccess.check(a)
            values.append(.init(id: id, title: form.title))
        }
        return values
    }
}
@available(iOS 16.0, *)
struct RequestBluefinMediaIntent: AppIntent {
    static var title: LocalizedStringResource = "Request media through Bluefin server"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Server action") var action: BluefinRequestActionEntity
    @Parameter(title: "I am authorized to request this media") var authorizedUse: Bool
    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        let a = try await NativeLibraryAccess.authorize()
        let prefix = NativeLibraryAccess.scope(a) + ":"
        guard action.id.hasPrefix(prefix), authorizedUse else { throw BluefinError.message("Confirm you are authorized to request this media.") }
        let caps: BluefinCapabilities = try await RustBridge.call("bluefinCapabilities", session: a)
        try NativeLibraryAccess.check(a)
        guard caps.protocolVersion == 1, caps.discovery?.requests == true else { throw BluefinError.message("Media requests are unavailable on this server or for your account.") }
        let id = String(action.id.dropFirst(prefix.count))
        let form: ServerActionForm = try await RustBridge.call("serverDiscoveryAction", session: a, fields: ["actionId": id])
        try NativeLibraryAccess.check(a)
        guard !form.requiresSelection, form.choices.isEmpty else { throw BluefinError.message("This server action requires options. Select them in Bluefin's discovery results.") }
        try await requestConfirmation(result: .result(dialog: IntentDialog(stringLiteral: String((form.title + ". " + form.notice).prefix(2000)))))
        try NativeLibraryAccess.check(a)
        let receipt: ServerActionReceipt = try await RustBridge.call("submitServerAction", session: a, fields: ["actionId": id, "selections": [String](), "authorizedUseConfirmed": true])
        try NativeLibraryAccess.check(a)
        return .result(dialog: IntentDialog(stringLiteral: receipt.message))
    }
}
#endif
