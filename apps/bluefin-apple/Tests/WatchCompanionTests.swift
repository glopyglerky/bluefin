#if os(iOS) && !targetEnvironment(macCatalyst)
import XCTest
import AppIntents
@testable import Bluefin
@available(iOS 16.0, *)
final class WatchCompanionTests: XCTestCase {
    func testPermitFencesAccountPairingAndExpiry() {
        let account = UUID(), pairing = UUID(), now = Date()
        let permit = WatchCompanionPermit(generation: account, pairing: pairing, expires: now.addingTimeInterval(60))
        XCTAssertTrue(permit.accepts(generation: account, pairing: pairing, now: now))
        XCTAssertFalse(permit.accepts(generation: UUID(), pairing: pairing, now: now))
        XCTAssertFalse(permit.accepts(generation: account, pairing: UUID(), now: now))
        XCTAssertFalse(permit.accepts(generation: account, pairing: pairing, now: now.addingTimeInterval(60)))
    }
    func testExactTitleRefusesAmbiguousFuzzyUnknownOrIncompleteResults() {
        XCTAssertEqual(NativeTitleSelection.index(titles: ["Song"], containers: [false], total: 1, query: "song"), 0)
        XCTAssertNil(NativeTitleSelection.index(titles: ["Song", "Song"], containers: [false, false], total: 2, query: "song"))
        XCTAssertNil(NativeTitleSelection.index(titles: ["Songs"], containers: [false], total: 1, query: "song"))
        XCTAssertNil(NativeTitleSelection.index(titles: [], containers: [], total: 0, query: "song"))
        XCTAssertNil(NativeTitleSelection.index(titles: ["Song"], containers: [false], total: nil, query: "song"))
        XCTAssertNil(NativeTitleSelection.index(titles: ["Song"], containers: [false], total: 61, query: "song"))
        XCTAssertNil(NativeTitleSelection.index(titles: ["Song"], containers: [true], total: 1, query: "song"))
    }
    @MainActor func testPostAwaitTokenRotationRejectsCapturedSession() async throws {
        let original = SessionVault.load()
        defer { if let original { try? SessionVault.save(original) } else { try? SessionVault.clear() } }
        let a = AccountSession(server: "https://example.invalid", token: "synthetic", userId: "fixture", username: "Fixture", deviceId: "fixture", allowHttp: false)
        try SessionVault.save(a)
        try NativeLibraryAccess.check(a)
        await Task.yield()
        var replacement = a; replacement.token = "rotated-synthetic"
        try SessionVault.save(replacement)
        XCTAssertThrowsError(try NativeLibraryAccess.check(a))
    }
    func testWatchRequiresHTTPSAndRejectsCredentialURLs() throws {
        XCTAssertEqual(try WatchAPI.server("https://example.com/base").path, "/base")
        for value in ["http://example.com", "https://u:p@example.com", "https://example.com/?token=x", "https://example.com/#x"] { XCTAssertThrowsError(try WatchAPI.server(value)) }
    }
    func testWatchPagePreservesUnknownTotalAndMetadata() throws {
        let page = try JSONDecoder().decode(WatchPage.self, from: Data(#"{"Items":[{"Id":"a","Name":"Song","Type":"Audio","Artists":["Artist"],"Album":"Album"}]}"#.utf8))
        XCTAssertNil(page.TotalRecordCount); XCTAssertEqual(page.Items.first?.Artists, ["Artist"])
    }
    @MainActor func testIntentEntitiesFenceServerUserAndTokenRotation() {
        let a = AccountSession(server: "https://example.com", token: "one", userId: "a", username: "A", deviceId: "phone", allowHttp: false)
        var b = a; b.token = "two"; XCTAssertNotEqual(NativeLibraryAccess.scope(a), NativeLibraryAccess.scope(b))
        b = a; b.userId = "b"; XCTAssertNotEqual(NativeLibraryAccess.scope(a), NativeLibraryAccess.scope(b))
        b = a; b.server = "https://other.example.com"; XCTAssertNotEqual(NativeLibraryAccess.scope(a), NativeLibraryAccess.scope(b))
    }
    @MainActor func testNoAutomaticLibrarySuggestionsOrNilForegroundPlayback() async throws {
        let suggestions = try await BluefinLibraryQuery().suggestedEntities()
        XCTAssertTrue(suggestions.isEmpty)
        NativeIntegrations.shared.detach()
        XCTAssertThrowsError(try NativeIntegrations.shared.requireStore())
    }
}
#endif
