#if os(iOS) && !targetEnvironment(macCatalyst)
import XCTest
@testable import Bluefin

final class ManagedPairingTests: XCTestCase {
    @MainActor func testLedgerRetainsDeniedAndFailedWritesAcrossRestart() async throws {
        var rows: [PhonePairingOwner] = []
        var failWrite = false
        let read = { rows }
        let write: ([PhonePairingOwner]) throws -> Void = { value in
            if failWrite { throw CancellationError() }
            rows = value
        }
        let ledger = PhonePairingLedger(read: read, write: write)
        let owner = PhonePairingOwner(id: UUID(), server: URL(string: "https://example.invalid")!, generation: UUID(), proof: .init(challenge: "synthetic", secret: "synthetic"), device: "synthetic")
        try ledger.remember(owner)
        do { try await ledger.drain(keeping: nil) { _ in throw CancellationError() }; XCTFail("Denied cancellation removed ownership") } catch {}
        XCTAssertEqual(rows, [owner])
        let restored = PhonePairingLedger(read: read, write: write)
        failWrite = true
        do { try await restored.drain(keeping: nil) { _ in }; XCTFail("Failed persistence removed ownership") } catch {}
        XCTAssertEqual(rows, [owner])
        failWrite = false
        try await restored.drain(keeping: owner.generation) { _ in XCTFail("Cancelled current account") }
        XCTAssertEqual(rows, [owner])
        try await restored.drain(keeping: nil) { captured in XCTAssertEqual(captured, owner) }
        XCTAssertTrue(rows.isEmpty)
    }

    @MainActor func testLedgerRefusesOverflowWithoutEviction() throws {
        var rows: [PhonePairingOwner] = []
        let ledger = PhonePairingLedger(read: { rows }, write: { rows = $0 })
        for _ in 0..<32 {
            try ledger.remember(.init(id: UUID(), server: URL(string: "https://example.invalid")!, generation: UUID(), proof: .init(challenge: UUID().uuidString, secret: "synthetic"), device: "synthetic"))
        }
        let before = rows
        XCTAssertThrowsError(try ledger.remember(.init(id: UUID(), server: URL(string: "https://example.invalid")!, generation: UUID(), proof: .init(challenge: "overflow", secret: "synthetic"), device: "synthetic")))
        XCTAssertEqual(rows, before)
    }
}
#endif
