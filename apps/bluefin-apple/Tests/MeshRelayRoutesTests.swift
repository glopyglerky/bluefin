import XCTest
@testable import Bluefin

final class MeshRelayRoutesTests: XCTestCase {
    private let approved = [
        "https://use1-1.relay.n0.iroh.link./",
        "https://usw1-1.relay.n0.iroh.link./"
    ]
    private func saved(relays: [String] = []) -> [String: Any] {
        ["operation": "items", "relayAllowlist": approved,
         "mesh": [
            "serverAddress": ["endpoint_id": Array(repeating: 1, count: 32),
                              "server_certificate_sha256": Array(repeating: 2, count: 32),
                              "service": "bluefin/connect/1", "relay_urls": relays,
                              "direct_addresses": ["192.0.2.8:4433"]],
            "sessionId": "synthetic-session", "deviceSeed": "synthetic-key",
            "apiOrigin": "https://account.example/"
         ]]
    }
    private func encoded(_ value: [String: Any]) throws -> Data {
        try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    }

    func testOnlyRoutesChangeAndSavedRequestIsUnmodified() throws {
        let original = saved()
        let originalBytes = try encoded(original)
        var routed = MeshRelayRoutes.applyingApprovedFallback(to: original, approvedRelays: Array(approved.reversed()))
        var mesh = try XCTUnwrap(routed["mesh"] as? [String: Any])
        var address = try XCTUnwrap(mesh["serverAddress"] as? [String: Any])
        XCTAssertEqual(address["relay_urls"] as? [String], approved)
        address["relay_urls"] = [String]()
        mesh["serverAddress"] = address
        routed["mesh"] = mesh
        XCTAssertEqual(try encoded(routed), originalBytes)
        XCTAssertEqual(try encoded(original), originalBytes)
    }

    func testExistingUnapprovedHintsAreNeverReplaced() throws {
        let original = saved(relays: ["https://unapproved.example/"])
        XCTAssertEqual(try encoded(MeshRelayRoutes.applyingApprovedFallback(to: original, approvedRelays: approved)),
                       try encoded(original))
    }

    func testFreshPairingRequestIsUntouched() throws {
        let original: [String: Any] = [
            "operation": "meshFinish", "serverAddress": ["relay_urls": [String]()],
            "deviceSeed": "synthetic-key", "request": ["synthetic": true]
        ]
        XCTAssertEqual(try encoded(MeshRelayRoutes.applyingApprovedFallback(to: original, approvedRelays: approved)),
                       try encoded(original))
    }

    func testInvalidOrAbsentPolicyCannotCreateRoutes() throws {
        let original = saved()
        for policy in [[], ["http://relay.example/"], ["https://user@relay.example/"],
                       ["https://relay.example/?secret=bad"], ["https://relay.example/#bad"],
                       approved + ["https://extra.example/"]] {
            XCTAssertEqual(try encoded(MeshRelayRoutes.applyingApprovedFallback(to: original, approvedRelays: policy)),
                           try encoded(original))
        }
    }

    func testQueuedRevocationUsesTheSamePinnedFallback() throws {
        var original = saved()
        original["operation"] = "meshRevoke"
        original["requestId"] = "synthetic-request"
        let routed = MeshRelayRoutes.applyingApprovedFallback(to: original, approvedRelays: approved)
        let mesh = try XCTUnwrap(routed["mesh"] as? [String: Any])
        let address = try XCTUnwrap(mesh["serverAddress"] as? [String: Any])
        XCTAssertEqual(address["relay_urls"] as? [String], approved)
        XCTAssertEqual(routed["requestId"] as? String, "synthetic-request")
        XCTAssertEqual(mesh["sessionId"] as? String, "synthetic-session")
        XCTAssertEqual(mesh["deviceSeed"] as? String, "synthetic-key")
    }
}
