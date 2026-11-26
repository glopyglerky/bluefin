import XCTest
#if !SPORTS_STANDALONE
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif
#endif

@MainActor final class SportsStoreTests: XCTestCase {
    func testConflictKeepsDraftAndAcceptedRevision() async throws {
        var observedTag: String?
        let api = SportsAPI(preferences: { _ in SportsFixtures.saved() }, home: { _ in SportsFixtures.feed }, save: { _, tag, _ in
            observedTag = tag; throw BluefinError.message("HTTP 412")
        })
        let store = SportsStore(api: api); let session = SportsFixtures.session("one")
        await store.connect(session)
        do { try await store.save(SportsFixtures.preferences, session: session, expectedETag: "\"7\""); XCTFail("Expected conflict") }
        catch { XCTAssertTrue(error is SportsPreferenceError) }
        XCTAssertEqual(observedTag, "\"7\""); XCTAssertEqual(store.saved?.version, 7)
        XCTAssertFalse(store.saving)
    }
    func testMissingRevisionRequiresReloadWithoutRetry() async {
        var writes = 0
        let api = SportsAPI(preferences: { _ in SportsFixtures.saved() }, home: { _ in SportsFixtures.feed }, save: { _, _, _ in
            writes += 1; throw BluefinError.message("HTTP 428")
        })
        let store = SportsStore(api: api); let session = SportsFixtures.session("one")
        await store.connect(session)
        do { try await store.save(SportsFixtures.preferences, session: session); XCTFail("Expected reload requirement") }
        catch { XCTAssertEqual(error as? SportsPreferenceError, .revisionRequired) }
        XCTAssertEqual(writes, 1); XCTAssertEqual(store.saved?.version, 7)
    }
    func testStaleEditorDoesNotAdoptRefreshedRevision() async throws {
        var revision: UInt64 = 7; var writes = 0
        let api = SportsAPI(preferences: { _ in SportsFixtures.saved(revision) }, home: { _ in SportsFixtures.feed }, save: { _, _, _ in writes += 1; return SportsFixtures.saved(9) })
        let store = SportsStore(api: api); let session = SportsFixtures.session("one")
        await store.connect(session); revision = 8; await store.connect(session)
        do { try await store.save(SportsFixtures.preferences, session: session, expectedETag: "\"7\""); XCTFail("Expected conflict") }
        catch { XCTAssertTrue(error is SportsPreferenceError) }
        XCTAssertEqual(writes, 0); XCTAssertEqual(store.saved?.version, 8)
    }
    func testAccountSwitchDiscardsLateResponse() async {
        var pending: CheckedContinuation<SportsFeed, Error>?
        let api = SportsAPI(preferences: { _ in SportsFixtures.saved() }, home: { session in
            if session.userId == "one" { return try await withCheckedThrowingContinuation { pending = $0 } }
            return SportsFeed(schema: 1, sports_only: false, games: [], local_broadcasts: [], presentation_only: true)
        }, save: { _, _, _ in SportsFixtures.saved(8) })
        let store = SportsStore(api: api)
        let first = Task { await store.connect(SportsFixtures.session("one")) }
        // Yield only to reach the controlled suspension, never wait on external work.
        while pending == nil { await Task.yield() }
        await store.connect(SportsFixtures.session("two"))
        pending?.resume(returning: SportsFixtures.feed); await first.value
        XCTAssertEqual(store.feed?.games.count, 0)
        store.disconnect(); XCTAssertNil(store.feed); XCTAssertNil(store.saved); XCTAssertNil(store.lastUpdated)
    }
    func testCrossAccountSaveIsRejectedBeforeTransport() async {
        var writes = 0
        let api = SportsAPI(preferences: { _ in SportsFixtures.saved() }, home: { _ in SportsFixtures.feed }, save: { _, _, _ in writes += 1; return SportsFixtures.saved(8) })
        let store = SportsStore(api: api); await store.connect(SportsFixtures.session("one"))
        do { try await store.save(SportsFixtures.preferences, session: SportsFixtures.session("two")); XCTFail("Expected rejection") } catch {}
        XCTAssertEqual(writes, 0)
    }
    func testSavedRevisionSurvivesOfflineHomeRefresh() async throws {
        var offline = false
        let api = SportsAPI(preferences: { _ in SportsFixtures.saved() }, home: { _ in
            if offline { throw BluefinError.message("offline") }; return SportsFixtures.feed
        }, save: { _, _, _ in offline = true; return SportsFixtures.saved(8) })
        let store = SportsStore(api: api); let session = SportsFixtures.session("one")
        await store.connect(session)
        try await store.save(SportsFixtures.preferences, session: session)
        XCTAssertEqual(store.saved?.version, 8); XCTAssertNotNil(store.feed); XCTAssertNotNil(store.failure)
    }
}
