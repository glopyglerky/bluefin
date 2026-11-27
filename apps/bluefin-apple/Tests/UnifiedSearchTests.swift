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

    func testPublicOffersPreserveUnknownRegionsFutureServicesAndExactQueryLinks() throws {
        let data = Data(#"""
        {"schemaVersion":1,"identity":{"kind":"recording","ids":[{"source":"musicbrainz","id":"a74b1b7f-71a5-4011-9441-d0b5e4122711"}]},"country":null,
         "offers":[{"id":"offer","identity":{"kind":"recording","ids":[{"source":"musicbrainz","id":"a74b1b7f-71a5-4011-9441-d0b5e4122711"}]},"country":null,
          "service":{"source":"domain","id":"future.example.org","name":"Future Music"},"accessType":"other","rawAccessType":"streaming","addon":null,
          "destinations":[{"kind":"title","url":"https://future.example.org/album/100?i=200"}],"price":null,"quality":null,
          "evidence":{"source":"musicbrainz","observedAt":42,"availableSince":null,"expiresAt":null,"expiresSoon":false,"regionVerified":false}}],
         "sources":[{"source":"musicbrainz","status":"partial","observedAt":42,"scope":"linkedAccess","coverage":"partial","catalogUrl":"https://musicbrainz.org/","attributions":[{"name":"MusicBrainz","url":"https://musicbrainz.org/"}]}]}
        """#.utf8)
        let value = try JSONDecoder().decode(PublicCatalogOffers.self, from: data)
        XCTAssertNil(value.country)
        XCTAssertNil(value.offers[0].country)
        XCTAssertFalse(value.offers[0].evidence.regionVerified)
        XCTAssertEqual(value.offers[0].service.name, "Future Music")
        XCTAssertEqual(value.offers[0].destinations[0].url.query, "i=200")
        XCTAssertEqual(value.offers[0].accessLabel, "streaming")
        XCTAssertEqual(value.sources[0].status, "partial")
    }

    func testPublicMusicFailuresRetainSourceStateWithoutInventingDetails() throws {
        let data = Data(#"""
        {"item":null,"description":null,"facts":[],"related":[],"sources":[
          {"source":"musicbrainz","status":"unavailable","observedAt":42,"scope":"linkedAccess","coverage":"partial","catalogUrl":null,"attributions":[]}]}
        """#.utf8)
        let value = try JSONDecoder().decode(PublicMusicDetail.self, from: data)
        XCTAssertNil(value.item)
        XCTAssertTrue(value.sources[0].interrupted)
        XCTAssertEqual(value.sources[0].observedAt, 42)
    }

    func testCatalogIdentityKeepsEveryAssertedLibraryIDAndIMDbOnlyFallback() {
        let item = MediaItem(Id: "movie", Type: "Movie",
                             ProviderIds: ["Tmdb": "603", "Imdb": "tt0133093"])
        let identity = PublicCatalogIdentity.library(item)
        XCTAssertEqual(identity?.ids.map(\.source), ["tmdb", "imdb"])
        XCTAssertEqual(identity?.ids.map(\.id), ["603", "tt0133093"])
        let imdb = MediaItem(Id: "imdb-only", Type: "Movie", ProviderIds: ["Imdb": "tt0133093"])
        XCTAssertEqual(PublicCatalogIdentity.library(imdb)?.ids.map(\.source), ["imdb"])
    }

    func testCatalogRequestIdentityIncludesTheAccountLifetimeAndPairedRoute() {
        let identity = PublicCatalogIdentity(kind: "movie", ids: [.init(source: "tmdb", id: "603")])
        let account = PublicCatalogAccount(generation: UUID(), paired: true)
        let request = PublicCatalogAccessRequest(identity: identity, account: account, country: "", attempt: 0)
        XCTAssertNotEqual(request, .init(identity: identity,
            account: .init(generation: UUID(), paired: true), country: "", attempt: 0))
        XCTAssertNotEqual(request, .init(identity: identity,
            account: .init(generation: account.generation, paired: false), country: "", attempt: 0))
        XCTAssertNotEqual(request, .init(identity: identity, account: account, country: "US", attempt: 0))
    }

}
