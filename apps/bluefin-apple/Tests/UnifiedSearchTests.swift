import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

@MainActor final class UnifiedSearchTests: XCTestCase {
    func testGlobalPreviewSearchIncludesMusicOutsideMovieLibrary() async throws {
        let store = LibraryStore(); store.explore()
        let model = UnifiedSearchModel()
        await model.load(query: "Original audio", store: store)
        XCTAssertEqual(model.items.map(\.Id), [MediaItem.previewAudio.Id])
        XCTAssertFalse(model.loading)
        await model.load(query: "Bluefin", store: store)
        XCTAssertTrue(model.items.contains { $0.Id == MediaItem.previewAlbum.Id })
        XCTAssertTrue(model.artists.contains { $0.Id == MediaItem.previewArtist.Id })
        await model.load(query: "Ocean", store: store)
        XCTAssertEqual(model.items.map(\.Id), MediaItem.preview.map(\.Id))
        XCTAssertTrue(model.artists.isEmpty)
    }
    func testSearchScopesKeepMusicAndLiveSeparateFromMovies() async throws {
        let store = LibraryStore(); store.explore()
        let model = UnifiedSearchModel()
        await model.load(query: "Bluefin", store: store, filters: .init(scope: .movies))
        XCTAssertTrue(model.items.isEmpty)
        XCTAssertTrue(model.artists.isEmpty)
        await model.load(query: "Ocean", store: store, filters: .init(scope: .tvShows))
        XCTAssertTrue(model.items.isEmpty)
        await model.load(query: "Original audio", store: store, filters: .init(scope: .music))
        XCTAssertEqual(model.items.map(\.Id), [MediaItem.previewAudio.Id])
        XCTAssertTrue(model.channels.isEmpty)
    }

    func testMetadataRefinementsRemainOnEveryServerPage() {
        let filters = UnifiedSearchFilters(scope: .tvShows, studioID: "studio-id",
            tag: "Service: Acme+", collectionID: "collection-id")
        let request = UnifiedSearchRequest(query: "ocean", filters: filters)
        XCTAssertEqual(request.lanes, [.library])
        let page = request.call(for: .library, start: 60)
        XCTAssertEqual(page.operation, "items")
        XCTAssertEqual(page.fields["searchScope"] as? String, "tvShows")
        XCTAssertEqual(page.fields["searchTerm"] as? String, "ocean")
        XCTAssertEqual(page.fields["startIndex"] as? Int, 60)
        XCTAssertEqual(page.fields["studioIds"] as? [String], ["studio-id"])
        XCTAssertEqual(page.fields["tags"] as? [String], ["Service: Acme+"])
        XCTAssertEqual(page.fields["parentId"] as? String, "collection-id")
        XCTAssertEqual(UnifiedSearchRequest(query: "live", filters: .init(scope: .liveTv)).lanes, [.programmes, .channels])
        XCTAssertEqual(UnifiedSearchRequest(query: "live", filters: .init(scope: .liveTv, tag: "Sport")).lanes, [.library])
    }

    func testPaginationCountsServerRowsAndStopsAtAnEmptyPage() {
        var results = UnifiedSearchPage()
        let first = MediaItem(Id: "first"), second = MediaItem(Id: "second")
        results.append(.init(Items: [first, second], TotalRecordCount: 6))
        results.append(.init(Items: [second, MediaItem(Id: "third")], TotalRecordCount: 6))
        XCTAssertEqual(results.items.map(\.Id), ["first", "second", "third"])
        XCTAssertEqual(results.nextIndex, 4)
        XCTAssertTrue(results.hasMore)
        results.append(.init(Items: [], TotalRecordCount: 6))
        XCTAssertFalse(results.hasMore)
    }

    func testSearchIdentityIncludesAccountGenerationAndEveryRefinement() {
        let account = UUID()
        let initial = UnifiedSearchIdentity(query: "ocean", filters: .init(), account: account)
        XCTAssertNotEqual(initial, .init(query: "ocean", filters: .init(tag: "Documentary"), account: account))
        XCTAssertNotEqual(initial, .init(query: "ocean", filters: .init(), account: UUID()))
    }

    private func waitUntil(_ condition: @MainActor () -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(2)
        while !condition() && Date() < deadline { await Task.yield() }
        if !condition() {
            XCTFail("Expected asynchronous search state", file: file, line: line)
            throw CancellationError()
        }
    }

    func testReturningAfterCanceledFirstPageResumesWithoutAcceptingItsLateResult() async throws {
        let store = LibraryStore(); store.explore()
        let filters = UnifiedSearchFilters(source: .library)
        var calls = 0
        var held: CheckedContinuation<ItemPage, Error>?
        let model = UnifiedSearchModel(fetchPage: { _, _, _, _, _, _ in
            calls += 1
            if calls == 1 {
                return try await withCheckedThrowingContinuation { held = $0 }
            }
            return .init(Items: [MediaItem(Id: "current")], TotalRecordCount: 1)
        })
        let first = Task { await model.load(query: "ocean", store: store, filters: filters) }
        try await waitUntil { held != nil }
        model.cancel()
        await model.load(query: "ocean", store: store, filters: filters)
        XCTAssertEqual(calls, 2)
        XCTAssertEqual(model.items.map(\.Id), ["current"])
        held?.resume(returning: .init(Items: [MediaItem(Id: "late-canceled")], TotalRecordCount: 1))
        await first.value
        XCTAssertEqual(model.items.map(\.Id), ["current"])
        XCTAssertFalse(model.loading)
    }

    func testReturningPreservesCompletedResultsAndCompletedEmptyLanes() async {
        let store = LibraryStore(); store.explore()
        var calls: [UnifiedSearchLane: Int] = [:]
        let model = UnifiedSearchModel(fetchPage: { _, lane, _, _, _, _ in
            calls[lane, default: 0] += 1
            return lane == .library
                ? .init(Items: [MediaItem(Id: "kept")], TotalRecordCount: 1)
                : .init(Items: [], TotalRecordCount: 0)
        })
        await model.load(query: "ocean", store: store)
        let library = model.laneModels[.library]
        model.cancel()
        await model.load(query: "ocean", store: store)
        XCTAssertTrue(model.laneModels[.library] === library)
        XCTAssertEqual(model.items.map(\.Id), ["kept"])
        XCTAssertEqual(Set(calls.keys), Set(UnifiedSearchLane.allCases))
        XCTAssertTrue(calls.values.allSatisfy { $0 == 1 })
        XCTAssertFalse(model.loading)
    }

}
