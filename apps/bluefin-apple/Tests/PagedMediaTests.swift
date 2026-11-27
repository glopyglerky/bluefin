import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

@MainActor final class PagedMediaTests: XCTestCase {
    private func page(_ start: Int, count: Int = 60, total: Int = 720) -> ItemPage {
        .init(Items: (start..<(start + count)).map { MediaItem(Id: String($0)) }, TotalRecordCount: total)
    }

    private func waitUntil(_ condition: @MainActor () -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(2)
        while !condition() && Date() < deadline { await Task.yield() }
        if !condition() {
            XCTFail("Expected asynchronous page state", file: file, line: line)
            throw CancellationError()
        }
    }

    func testEvictionKeepsStablePositionsAndRehydratesEarlierRows() {
        var window = MediaPageWindow()
        for start in stride(from: 0, to: 720, by: 60) {
            window.append(page(start))
            window.trim(around: start)
            XCTAssertLessThanOrEqual(window.cachedPageCount, 5)
            XCTAssertLessThanOrEqual(window.cachedItemCount, 300)
        }
        XCTAssertEqual(window.nextIndex, 720)
        XCTAssertEqual(window.pages.map(\.start), Array(stride(from: 0, to: 720, by: 60)))
        XCTAssertNil(window.item(at: 0))
        window.install(page(0), at: 0)
        window.trim(around: 0, protecting: [0])
        XCTAssertEqual(window.item(at: 0)?.Id, "0")
        XCTAssertEqual(window.pages.count, 12)
        XCTAssertEqual(window.nextIndex, 720)
        XCTAssertLessThanOrEqual(window.cachedItemCount, 300)
    }

    func testDistinctPlaylistEntriesKeepRepeatedSongs() {
        var first = MediaItem(Id: "same-song")
        first.PlaylistItemId = "entry-one"
        var second = first
        second.PlaylistItemId = "entry-two"
        var window = MediaPageWindow()
        window.append(.init(Items: [first, second], TotalRecordCount: 2))
        XCTAssertEqual(window.items.count, 2)
        XCTAssertFalse(window.isDuplicate(at: 1))
        window.append(.init(Items: [second], TotalRecordCount: 3))
        XCTAssertTrue(window.isDuplicate(at: 2))
    }

    func testRehydratingRemovedTitlesPreservesSlotsWithoutRepeatedLoading() {
        var window = MediaPageWindow()
        window.append(page(0))
        window.install(.init(Items: [], TotalRecordCount: 0), at: 0)
        XCTAssertEqual(window.page(containing: 30)?.count, 60)
        XCTAssertNotNil(window.page(containing: 30)?.items)
        XCTAssertNil(window.item(at: 30))
    }

    func testLatePreviousQueryCannotReplaceNewerResults() async throws {
        let model = PagedMediaModel()
        var old: CheckedContinuation<ItemPage, Error>?
        let oldLoad = Task {
            await model.start(identity: "old") { _, _ in
                try await withCheckedThrowingContinuation { old = $0 }
            }
        }
        try await waitUntil { old != nil }
        await model.start(identity: "new") { _, _ in
            .init(Items: [MediaItem(Id: "new")], TotalRecordCount: 1)
        }
        old?.resume(returning: .init(Items: [MediaItem(Id: "old")], TotalRecordCount: 1))
        await oldLoad.value
        XCTAssertEqual(model.items.map(\.Id), ["new"])
    }

    func testReturningToSameCollectionRetainsAlreadyLoadedPositions() async {
        let model = PagedMediaModel()
        var requests = 0
        await model.start(identity: "library") { _, _ in
            requests += 1
            return self.page(0)
        }
        model.cancel()
        await model.start(identity: "library") { _, _ in
            requests += 1
            return self.page(0)
        }
        XCTAssertEqual(requests, 1)
        XCTAssertEqual(model.window.nextIndex, 60)
    }

    func testCanceledRehydrationRequeuesWhenItsRowsBecomeVisibleAgain() async throws {
        let model = PagedMediaModel()
        var requests: [Int: Int] = [:]
        var holdNextFirstPage = false
        var held: CheckedContinuation<ItemPage, Error>?
        await model.start(identity: "library") { start, _ in
            requests[start, default: 0] += 1
            if start == 0 && holdNextFirstPage {
                holdNextFirstPage = false
                return try await withCheckedThrowingContinuation { held = $0 }
            }
            return self.page(start)
        }
        var lastVisible: Int?
        while model.window.nextIndex < 720 {
            let previousCount = model.window.nextIndex
            if let lastVisible { model.disappear(lastVisible) }
            lastVisible = previousCount - 1
            model.appear(previousCount - 1)
            try await waitUntil { model.window.nextIndex > previousCount }
        }
        if let lastVisible { model.disappear(lastVisible) }
        model.appear(719); model.disappear(719)
        XCTAssertNil(model.window.item(at: 0))
        holdNextFirstPage = true
        model.appear(0)
        try await waitUntil { held != nil }
        model.disappear(0)
        model.appear(719)
        model.disappear(719)
        model.appear(0)
        held?.resume(returning: page(0))
        try await waitUntil { model.window.item(at: 0) != nil }
        XCTAssertGreaterThanOrEqual(requests[0] ?? 0, 3)
        XCTAssertEqual(model.window.item(at: 0)?.Id, "0")
        model.cancel()
    }

    func testSearchOrderAndAllRefinementsTravelWithEveryPage() {
        let filters = UnifiedSearchFilters(scope: .movies, source: .library, sort: .newestReleased,
            watched: .unwatched, favoritesOnly: true, genre: "Drama", year: 1997)
        let request = UnifiedSearchRequest(query: "ocean", filters: filters)
        XCTAssertEqual(request.lanes, [.library])
        for offset in [0, 60, 300] {
            let call = request.call(for: .library, start: offset)
            XCTAssertEqual(call.fields["startIndex"] as? Int, offset)
            XCTAssertEqual(call.fields["sortBy"] as? String, "PremiereDate")
            XCTAssertEqual(call.fields["sortOrder"] as? String, "Descending")
            XCTAssertEqual(call.fields["searchOrder"] as? String, "SortBy")
            XCTAssertEqual(call.fields["isPlayed"] as? Bool, false)
            XCTAssertEqual(call.fields["isFavorite"] as? Bool, true)
            XCTAssertEqual(call.fields["genres"] as? [String], ["Drama"])
            XCTAssertEqual(call.fields["years"] as? [Int], [1997])
        }
        let relevance = UnifiedSearchRequest(query: "ocean", filters: .init()).call(for: .library, start: 0)
        XCTAssertNil(relevance.fields["sortBy"])
        XCTAssertNil(relevance.fields["searchOrder"])
    }
}
