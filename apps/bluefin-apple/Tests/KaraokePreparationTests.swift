import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

final class KaraokePreparationTests: XCTestCase {
    private func manifest(_ changes: [String: Any] = [:]) throws -> KaraokePreparationManifest {
        var value: [String: Any] = ["itemId":"synthetic-item", "revision": String(repeating: "a", count: 64), "durationSeconds":1.0, "processing":"suppliedStems", "accompaniment":["sha256":String(repeating:"b",count:64),"bytes":16044], "leadVocal":["sha256":String(repeating:"c",count:64),"bytes":16044], "timeline":["offsetSeconds":0.0,"lines":[["id":"one","voice":"lead","tokens":[["text":"La","start":0.0,"end":1.0]]]]]]
        value.merge(changes) { _,new in new }
        return try JSONDecoder().decode(KaraokePreparationManifest.self, from: JSONSerialization.data(withJSONObject:value))
    }
    func testBoundManifestAndSourceTruth() throws {
        try manifest().validate(itemID:"synthetic-item")
        XCTAssertThrowsError(try manifest().validate(itemID:"other"))
        XCTAssertThrowsError(try manifest(["processing":"modelSeparated"]).validate(itemID:"synthetic-item"))
        XCTAssertThrowsError(try manifest(["revision":"bad"]).validate(itemID:"synthetic-item"))
        XCTAssertThrowsError(try manifest(["durationSeconds":0]).validate(itemID:"synthetic-item"))
    }
    func testStemOnlyManifestRequiresExplicitMarkerAndKeepsSourceBinding() throws {
        let changes: [String: Any] = ["timingKind": "none", "timeline": NSNull(),
            "processing": "modelSeparatedStems", "processingReference": "test-model",
            "mediaSourceId": "selected-source"]
        let audio = try manifest(changes)
        try audio.validate(itemID: "synthetic-item", mediaSourceID: "selected-source")
        XCTAssertNil(audio.timeline)
        XCTAssertEqual(audio.timingKind, KaraokeTimingKind.none)
        XCTAssertThrowsError(try audio.validate(itemID: "other-item", mediaSourceID: "selected-source"))
        XCTAssertThrowsError(try audio.validate(itemID: "synthetic-item", mediaSourceID: "other-source"))
        XCTAssertThrowsError(try manifest(["timeline": NSNull()]).validate(itemID: "synthetic-item"))
        XCTAssertThrowsError(try manifest(["timingKind": "none"]).validate(itemID: "synthetic-item"))
        for kind in ["word", "syllable"] {
            try manifest(["timingKind": kind]).validate(itemID: "synthetic-item")
            XCTAssertThrowsError(try manifest(["timingKind": kind, "timeline": NSNull()]).validate(itemID: "synthetic-item"))
        }
        XCTAssertThrowsError(try manifest(["timingKind": "guessed"]))
    }

    func testLeaseDeletesOnlyAfterLastConsumer() throws {
        var lease: KaraokePreparationLease? = try KaraokePreparationLease()
        let directory = try XCTUnwrap(lease?.directory)
        var retained = lease; lease = nil
        XCTAssertTrue(FileManager.default.fileExists(atPath:directory.path))
        XCTAssertNotNil(retained); retained = nil
        XCTAssertFalse(FileManager.default.fileExists(atPath:directory.path))
    }
    func testProcessedManifestIsDistinctFromSuppliedStems() throws {
        let prepared = try manifest(["processing":"modelSeparatedStems",
            "processingReference":"deezer/spleeter/2stems@v1.4.0", "mediaSourceId":"selected-source"])
        XCTAssertNoThrow(try prepared.validate(itemID:"synthetic-item"))
        XCTAssertEqual(prepared.processingKind, .modelSeparated)
        XCTAssertThrowsError(try prepared.validate(itemID:"synthetic-item", mediaSourceID:"other-source"))
        XCTAssertThrowsError(try manifest(["processing":"modelSeparatedStems", "mediaSourceId":"selected-source"]).validate(itemID:"synthetic-item"))
        XCTAssertThrowsError(try manifest(["revision":String(repeating:"Ａ",count:64)]).validate(itemID:"synthetic-item"))
    }
    func testPreparationStatusRejectsAnotherJobOrSelectedSource() throws {
        let id = UUID().uuidString
        func status(_ changes:[String:Any] = [:]) throws -> KaraokePreparationStatus {
            var value:[String:Any] = ["itemId":"synthetic-item","jobId":id,"mediaSourceId":"selected-source","state":"ready","retryAfterMilliseconds":1000]
            value.merge(changes) { _,new in new }
            return try JSONDecoder().decode(KaraokePreparationStatus.self,from:JSONSerialization.data(withJSONObject:value))
        }
        try status().validate(itemID:"synthetic-item",requestID:id,sourceID:"selected-source")
        XCTAssertThrowsError(try status(["jobId":UUID().uuidString]).validate(itemID:"synthetic-item",requestID:id,sourceID:"selected-source"))
        XCTAssertThrowsError(try status(["mediaSourceId":"other"]).validate(itemID:"synthetic-item",requestID:id,sourceID:"selected-source"))
        XCTAssertThrowsError(try status(["mediaSourceId":NSNull()]).validate(itemID:"synthetic-item",requestID:id,sourceID:nil))
        XCTAssertThrowsError(try status(["retryAfterMilliseconds":0]).validate(itemID:"synthetic-item",requestID:id,sourceID:nil))
        XCTAssertThrowsError(try status(["state":"inventedReady"]))
    }
}
