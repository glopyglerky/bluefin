import XCTest
import Foundation
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

@MainActor final class HomePersonalizationTests: XCTestCase {
    private var directory: URL!
    private var files: NativePlaybackFixtures.Files!
    private var server: NativePlaybackHTTP?

    override func setUp() async throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("BluefinHomeTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let placeholder = directory.appendingPathComponent("unused-media")
        try Data([0]).write(to: placeholder)
        files = .init(video: placeholder, audio: placeholder)
    }
    override func tearDown() async throws {
        await server?.stop()
        if let directory { try FileManager.default.removeItem(at: directory) }
    }
    private func session(_ server: NativePlaybackHTTP) -> AccountSession {
        .init(server: server.origin, token: server.token, userId: "home-fixture",
              username: "Synthetic Home", deviceId: "home-xctest", allowHttp: true)
    }

    func testHomeKeepsRankAndSkipsProgressWithoutLosingOverflowCandidates() throws {
        let items = (1...24).map { MediaItem(Id: String($0), Name: "Fixture \($0)", Type: "Movie") }
        var home = HomePage(resume: [], latest: [], nextUp: [], recommendations: [
            .init(id: "a", title: "Because you watched", items: items, reason: "watched"),
            .init(id: "b", title: "From your search", items: items, reason: "search")
        ])
        home.boundRecommendations()
        XCTAssertEqual(home.recommendations.map { $0.items.count }, [12, 12])
        XCTAssertEqual(home.recommendations.flatMap(\.items).map(\.Id), items.map(\.Id))
        home.resume = [items[0]]
        home.boundRecommendations()
        XCTAssertEqual(home.featured?.Id, items[0].Id)
        XCTAssertFalse(home.recommendations.flatMap(\.items).contains { $0.Id == items[0].Id })
        let id = "12345678-1234-1234-1234-123456789ABC"
        var normalized = HomePage(resume: [.init(Id: id)], latest: [], nextUp: [], recommendations: [
            .init(id: "a", title: "For you", items: [.init(Id: "12345678123412341234123456789abc")])
        ])
        normalized.boundRecommendations()
        XCTAssertTrue(normalized.recommendations.isEmpty)
    }

    func testLegacyHomeDecodesAndColdStartUsesActualLatest() throws {
        let page = try JSONDecoder().decode(HomePage.self, from: Data(
            #"{"resume":[],"latest":[{"Id":"latest","Name":"A real addition"}],"nextUp":[]}"#.utf8))
        XCTAssertTrue(page.recommendations.isEmpty)
        XCTAssertEqual(page.featured?.Id, "latest")
    }

    func testHomeLoadsRealShelvesAndOnlyExplicitSearchWritesOwnerHistory() async throws {
        let api = try NativePlaybackHTTP(files: files); server = api
        let fixture = HomeFixture()
        api.intercept = { request, respond in fixture.respond(request, complete: respond) }
        try await api.start()
        let store = LibraryStore(); store.session = session(api)
        await store.refresh()
        XCTAssertNil(store.libraryProblem)
        XCTAssertEqual(store.home.recommendations.first?.items.first?.Id, "recommended")
        XCTAssertEqual(store.searchHistoryGeneration, 7)
        let initial = await api.requests()
        XCTAssertFalse(initial.contains { $0.method == "POST" }, "Loading Home must not invent search events")

        await store.recordHomeSearch(query: "  space   documentaries ")
        await store.recordHomeSearch(query: "space documentaries")
        let requests = await api.requests()
        let searches = requests.filter { $0.method == "POST" && $0.path == "/Bluefin/Home/Searches" }
        XCTAssertEqual(searches.count, 1, "A repeated submit must not add duplicate immediate signals")
        let event = try XCTUnwrap(try JSONSerialization.jsonObject(with: searches[0].body) as? [String: Any])
        XCTAssertEqual(event["query"] as? String, "space documentaries")
        XCTAssertEqual(event["generation"] as? Int, 7)
        XCTAssertNotNil(UUID(uuidString: try XCTUnwrap(event["eventId"] as? String)))
        XCTAssertNil(event["userId"], "The owner comes from authentication, never a request body")

        await store.clearHomeSearchHistory()
        XCTAssertEqual(store.searchHistoryGeneration, 8)
        XCTAssertFalse(store.home.recommendations.contains { $0.reason == "search" })
        XCTAssertNil(store.homeHistoryProblem)
    }

    func testOlderHomeFailureCannotEraseNewerRecommendationsOrReportStaleError() async throws {
        let api = try NativePlaybackHTTP(files: files); server = api
        let began = expectation(description: "Initial Home recommendation request received")
        let fixture = HomeFixture(holdFirstRecommendations: began)
        api.intercept = { request, respond in fixture.respond(request, complete: respond) }
        try await api.start()
        let store = LibraryStore(); store.session = session(api)
        let initial = Task { await store.refresh() }
        await fulfillment(of: [began], timeout: 5)

        await store.refreshHomeRecommendations()
        XCTAssertEqual(store.home.recommendations.first?.items.first?.Id, "recommended")
        XCTAssertEqual(store.searchHistoryGeneration, 7)
        api.queue.async { fixture.failPendingRecommendation() }
        await initial.value

        XCTAssertEqual(store.home.recommendations.first?.items.first?.Id, "recommended")
        XCTAssertEqual(store.searchHistoryGeneration, 7)
        XCTAssertNil(store.libraryProblem, "A superseded request cannot report a current Home failure")
        XCTAssertNil(store.homeHistoryProblem)
    }

    func testDelayedRecommendationsCannotCrossAccountContext() async throws {
        let api = try NativePlaybackHTTP(files: files); server = api
        let began = expectation(description: "Owner recommendation request received")
        let response = Data(#"{"recommendations":[{"id":"private","title":"Old account","items":[{"Id":"private-item"}]}],"searchHistorySupported":true,"searchHistoryGeneration":3}"#.utf8)
        let queue = api.queue
        api.intercept = { request, respond in
            guard request.path == "/Bluefin/Home/Recommendations" else { return false }
            began.fulfill()
            queue.asyncAfter(deadline: .now() + 0.15) { respond(.init(bytes: response)) }
            return true
        }
        try await api.start()
        let store = LibraryStore(); store.session = session(api)
        let context = store.accountContext
        let load = Task { await store.refreshHomeRecommendations() }
        await fulfillment(of: [began], timeout: 5)
        store.explore()
        XCTAssertNotEqual(context, store.accountContext)
        await load.value
        XCTAssertTrue(store.home.recommendations.isEmpty)
        XCTAssertNil(store.searchHistoryGeneration)
        XCTAssertFalse(store.home.latest.contains { $0.Id == "private-item" })
    }
}

/// Mutations occur only on NativePlaybackHTTP.queue.
private final class HomeFixture: @unchecked Sendable {
    private var generation = 7
    private var initialRecommendations: XCTestExpectation?
    private var pendingRecommendation: ((NativePlaybackHTTP.Response) -> Void)?
    init(holdFirstRecommendations: XCTestExpectation? = nil) {
        initialRecommendations = holdFirstRecommendations
    }
    func failPendingRecommendation() {
        let complete = pendingRecommendation
        pendingRecommendation = nil
        complete?(.init(bytes: Data(), status: "503 Service Unavailable"))
    }
    func respond(_ request: NativePlaybackHTTP.Request, complete: @escaping (NativePlaybackHTTP.Response) -> Void) -> Bool {
        let path = request.path.components(separatedBy: "?")[0]
        let body: Any
        if path == "/Bluefin/Home/Recommendations" {
            if let started = initialRecommendations {
                initialRecommendations = nil
                pendingRecommendation = complete
                started.fulfill()
                return true
            }
            body = ["searchHistorySupported": true, "searchHistoryGeneration": generation,
                    "recommendations": generation == 7 ?
                    [["id": "for-you", "title": "Because you like a film", "reason": "liked",
                      "items": [["Id": "recommended", "Name": "Recommendation", "Type": "Movie"]]],
                     ["id": "from-search", "title": "From your search", "reason": "search",
                      "items": [["Id": "search-result", "Name": "Search result", "Type": "Series"]]]] :
                    [["id": "for-you", "title": "Because you like a film", "reason": "liked",
                      "items": [["Id": "recommended", "Name": "Recommendation", "Type": "Movie"]]]]]
        } else if path == "/Bluefin/Home/Searches" {
            if request.method == "DELETE" { generation += 1 }
            body = ["generation": generation]
        } else if path.hasPrefix("/Users/") || path == "/Shows/NextUp" {
            body = ["Items": [], "TotalRecordCount": 0]
        } else { return false }
        do { complete(.init(bytes: try JSONSerialization.data(withJSONObject: body))) }
        catch { complete(.init(bytes: Data(), status: "500 Fixture encoding failed")) }
        return true
    }
}
