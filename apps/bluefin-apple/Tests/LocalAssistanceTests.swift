import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

@MainActor final class LocalAssistanceTests: XCTestCase {
    func testAvailabilityRequiresAllCurrentConditions() {
        for supported in [false, true] {
            for ready in [false, true] {
                for locale in [false, true] {
                    XCTAssertEqual(
                        LocalAssistanceAvailability.admitted(
                            systemSupported: supported, modelReady: ready, localeSupported: locale
                        ),
                        supported && ready && locale
                    )
                }
            }
        }
        #if os(tvOS) || os(watchOS)
        XCTAssertFalse(LocalAssistanceAvailability.isAvailable)
        #endif
    }

    func testCancellationRetainsRealInFlightPermitUntilFrameworkCompletion() {
        let permit = LocalAssistancePermit()
        let first = LocalAssistanceTicket(id: 1, generation: 1)
        let next = LocalAssistanceTicket(id: 2, generation: 2)
        XCTAssertTrue(permit.acquire(first))
        permit.requestCancellation(first)
        XCTAssertTrue(permit.cancellationRequested)
        XCTAssertFalse(permit.acquire(next))
        permit.finished(next)
        XCTAssertEqual(permit.active, first)
        XCTAssertFalse(permit.acquire(next))
        permit.finished(first)
        XCTAssertNil(permit.active)
        XCTAssertTrue(permit.acquire(next))
        // A late completion for the old model cannot free the newer lease.
        permit.finished(first)
        XCTAssertEqual(permit.active, next)
    }

    func testCatalogAdapterDoesNotCopyProviderTransportOrUserData() throws {
        var item = MediaItem(Id: "catalog-01", Name: "Quiet film", Type: "Movie",
                             Overview: "A walk on the coast.", ProductionYear: 2024,
                             RunTimeTicks: 540_000_000_00)
        item.ProviderIds = ["private-provider": "must-not-enter-model"]
        item.ExternalUrls = [.init(Name: "Private location", Url: "https://private.example/token")]
        let input = LocalAssistanceInput.catalog(query: "Something quiet", items: [item, item])
        let data = try JSONSerialization.data(withJSONObject: input)
        let text = String(decoding: data, as: UTF8.self)
        XCTAssertFalse(text.contains("private-provider"))
        XCTAssertFalse(text.contains("private.example"))
        XCTAssertFalse(text.contains("ExternalUrls"))
        XCTAssertFalse(text.contains("UserData"))
        XCTAssertEqual((input["items"] as? [[String: Any]])?.count, 1)
    }

    func testPlaybackAdapterAcceptsOnlyTypedStatusAndNoErrorDetails() throws {
        let input = LocalAssistanceInput.playback(status: .failed)
        XCTAssertEqual(Set(input.keys), ["task", "status"])
        XCTAssertEqual(input["status"] as? String, "failed")
    }

    func testSummaryPassesExactDescriptionForRustGrounding() {
        let item = MediaItem(Id: "catalog-01", Name: "Quiet film", Type: "Movie")
        let supplied = "First supplied sentence. Second supplied sentence."
        let input = LocalAssistanceInput.summary(item: item, description: supplied)
        XCTAssertEqual((input["item"] as? [String: Any])?["description"] as? String, supplied)
    }
}
