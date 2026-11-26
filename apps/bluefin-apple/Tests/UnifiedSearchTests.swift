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

}
