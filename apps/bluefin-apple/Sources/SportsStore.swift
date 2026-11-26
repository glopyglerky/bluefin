import Foundation
import Combine

/// Injectable typed operations keep auth and HTTP semantics in the existing Rust bridge.
struct SportsAPI {
    var preferences: (AccountSession) async throws -> SportsSaved
    var home: (AccountSession) async throws -> SportsFeed
    var save: (SportsPreferences, String, AccountSession) async throws -> SportsSaved
    static let live = SportsAPI(
        preferences: { try await RustBridge.call("sportsPreferences", session: $0) },
        home: { try await RustBridge.call("sportsHome", session: $0) },
        save: { preferences, etag, session in
            let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(preferences))
            return try await RustBridge.call("saveSportsPreferences", session: session, fields: ["etag": etag, "preferences": value])
        })
}

@MainActor final class SportsStore: ObservableObject {
    @Published private(set) var feed: SportsFeed?
    @Published private(set) var saved: SportsSaved?
    @Published private(set) var loading = false
    @Published private(set) var saving = false
    @Published private(set) var available = false
    @Published private(set) var lastUpdated: Date?
    @Published private(set) var accountGeneration = UUID()
    @Published var failure: String?
    private var identity: String?
    private let api: SportsAPI
    init(api: SportsAPI = .live) { self.api = api }
    func disconnect() {
        accountGeneration = UUID(); identity = nil
        feed = nil; saved = nil; failure = nil; available = false
        loading = false; saving = false; lastUpdated = nil
    }
    func connect(_ session: AccountSession?) async {
        let next = session.map(OfflineLibrary.identity)
        if identity != next { disconnect(); identity = next }
        guard let session, !loading, !saving else { return }
        let current = accountGeneration
        loading = true
        defer { if accountGeneration == current { loading = false } }
        do {
            let preferences = try await api.preferences(session)
            try Task.checkCancellation()
            guard accountGeneration == current else { return }
            guard preferences.etag == "\"\(preferences.version)\"" else { throw BluefinError.message("Reload the preference revision before saving.") }
            saved = preferences; available = true
            let home = try await api.home(session)
            try Task.checkCancellation()
            guard accountGeneration == current else { return }
            feed = home; failure = nil; lastUpdated = Date()
        } catch is CancellationError {} catch {
            guard accountGeneration == current else { return }
            if error.localizedDescription.contains("HTTP 404") { available = false; feed = nil; saved = nil }
            else { failure = "Scores could not refresh. Check your connection and try again." }
        }
    }
    /// The editor supplies the revision it opened. A later refresh cannot silently
    /// promote an old draft to a new revision and overwrite another device's changes.
    func save(_ preferences: SportsPreferences, session: AccountSession, expectedETag: String? = nil) async throws {
        guard let saved, identity == OfflineLibrary.identity(session), !saving, !loading else {
            throw BluefinError.message("Reload your sports preferences before saving.")
        }
        let revision = expectedETag ?? saved.etag
        guard revision == saved.etag else { throw SportsPreferenceError.conflict }
        let current = accountGeneration
        saving = true
        defer { if accountGeneration == current { saving = false } }
        do {
            let updated = try await api.save(preferences, revision, session)
            try Task.checkCancellation()
            guard accountGeneration == current else { throw CancellationError() }
            guard updated.etag == "\"\(updated.version)\"" else { throw BluefinError.message("Reload the preference revision before saving.") }
            self.saved = updated
        } catch {
            guard accountGeneration == current else { throw CancellationError() }
            if error.localizedDescription.contains("412") { throw SportsPreferenceError.conflict }
            if error.localizedDescription.contains("428") { throw SportsPreferenceError.revisionRequired }
            throw error
        }
        // Saving succeeded even if refreshing scores fails. Keep the accepted revision.
        do {
            let home = try await api.home(session)
            try Task.checkCancellation()
            guard accountGeneration == current else { throw CancellationError() }
            feed = home; lastUpdated = Date(); failure = nil
        } catch is CancellationError { throw CancellationError() } catch {
            guard accountGeneration == current else { throw CancellationError() }
            failure = "Preferences saved. Scores could not refresh; try again when connected."
        }
    }
}

enum SportsPreferenceError: LocalizedError {
    case conflict, revisionRequired
    var errorDescription: String? {
        switch self {
        case .conflict: return "Preferences changed on another device. Reload before making further changes. Your draft has not been saved."
        case .revisionRequired: return "The server requires a current preference revision. Reload before saving."
        }
    }
}
