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
}
