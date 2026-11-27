import Foundation

/// Host checks for the wire contract. Device tests separately exercise the real AVPlayer.
@main enum KaraokePreparationChecks {
    static func main() throws {
        var count = 0
        let timeline: [String: Any] = ["offsetSeconds": 0.0, "lines": [
            ["id": "line", "voice": "lead", "tokens": [["text": "Original tone", "start": 0.0, "end": 1.0]]]]]
        let base: [String: Any] = [
            "itemId": "track", "revision": String(repeating: "a", count: 64),
            "durationSeconds": 2.0, "processing": "modelSeparatedStems",
            "processingReference": "test-model", "mediaSourceId": "selected-source",
            "accompaniment": ["sha256": String(repeating: "b", count: 64), "bytes": 16044],
            "leadVocal": ["sha256": String(repeating: "c", count: 64), "bytes": 16044],
            "timeline": timeline]
        func check(_ name: String, accepts: Bool, changes: [String: Any] = [:], omitted: [String] = [],
                   item: String = "track", source: String = "selected-source") throws {
            var value = base
            value.merge(changes) { _, new in new }
            for key in omitted { value.removeValue(forKey: key) }
            do {
                let manifest = try JSONDecoder().decode(KaraokePreparationManifest.self, from: JSONSerialization.data(withJSONObject: value))
                try manifest.validate(itemID: item, mediaSourceID: source)
                guard accepts else { throw CheckFailure.failed(name) }
            } catch is CheckFailure { throw CheckFailure.failed(name) }
            catch { guard !accepts else { throw CheckFailure.failed(name + ": " + error.localizedDescription) } }
            count += 1
        }
        try check("Legacy valid timeline", accepts: true)
        try check("Legacy omitted timeline", accepts: false, omitted: ["timeline"])
        try check("Legacy null timeline", accepts: false, changes: ["timeline": NSNull()])
        try check("Explicit none omitted timeline", accepts: true, changes: ["timingKind": "none"], omitted: ["timeline"])
        try check("Explicit none null timeline", accepts: true, changes: ["timingKind": "none", "timeline": NSNull()])
        try check("Explicit none conflicting timeline", accepts: false, changes: ["timingKind": "none"])
        for kind in ["word", "syllable"] {
            try check(kind + " valid", accepts: true, changes: ["timingKind": kind])
            try check(kind + " absent", accepts: false, changes: ["timingKind": kind], omitted: ["timeline"])
            try check(kind + " null", accepts: false, changes: ["timingKind": kind, "timeline": NSNull()])
            try check(kind + " malformed", accepts: false, changes: ["timingKind": kind, "timeline": ["offsetSeconds": 0, "lines": []]])
        }
        try check("Unknown timing kind", accepts: false, changes: ["timingKind": "invented"])
        let noTiming: [String: Any] = ["timingKind": "none", "timeline": NSNull()]
        try check("Stem-only wrong item", accepts: false, changes: noTiming, item: "another-track")
        try check("Stem-only wrong source", accepts: false, changes: noTiming, source: "another-source")
        try check("Legacy supplied response must qualify playing source", accepts: false,
                  changes: ["processing": "suppliedStems"], omitted: ["mediaSourceId"])
        try check("Supplied response rejects another edition", accepts: false,
                  changes: ["processing": "suppliedStems", "mediaSourceId": "other-source"])
        try check("Qualified legacy supplied response remains valid", accepts: true,
                  changes: ["processing": "suppliedStems"])
        try check("Stem-only missing processor", accepts: false, changes: noTiming, omitted: ["processingReference"])
        try check("Stem-only invalid duration", accepts: false, changes: noTiming.merging(["durationSeconds": 0]) { _, new in new })
        try check("Stem-only malformed digest", accepts: false, changes: noTiming.merging(["revision": "bad"]) { _, new in new })
        print("PASS \(count) native Sing contract checks")
    }
    enum CheckFailure: Error { case failed(String) }
}
