import Foundation

private final class MemoryIO: WidgetSnapshotIO, @unchecked Sendable {
    // Tests run sequentially; the writer actor has exclusive access during each await.
    var data = Data()
    var failRead = false
    var failWrite = false
    var failRemove = false
    func read() throws -> Data { if failRead { throw Fault.injected }; return data }
    func replace(_ value: Data) throws { if failWrite { throw Fault.injected }; data = value }
    func remove() throws { if failRemove { throw Fault.injected }; data = Data() }
    enum Fault: Error { case injected }
}

@main struct WidgetContractTests {
    static func main() async throws {
        let now = Date(timeIntervalSinceReferenceDate: 800_000_000)
        let owner = UUID(), next = UUID()
        let valid = try JSONEncoder().encode(WidgetSnapshot(generation: owner, status: .current, now: now))
        func check(_ condition: @autoclosure () -> Bool, _ name: String) {
            precondition(condition(), name)
            print("PASS " + name)
        }
        check(WidgetSnapshot.decode(valid, now: now)?.status == .current, "current snapshot")
        check(WidgetSnapshot.decode(valid, now: now, expectedGeneration: next) == nil, "owner isolation")
        check(WidgetSnapshot.decode(valid, now: now.addingTimeInterval(-1)) == nil, "future capture")
        check(WidgetSnapshot.decode(valid, now: now.addingTimeInterval(900)) == nil, "expiry inclusive")
        check(WidgetSnapshot.decode(Data(repeating: 32, count: 1025), now: now) == nil, "oversize")
        check(WidgetSnapshot.decode(Data("broken".utf8), now: now) == nil, "corrupt JSON")
        let object = try JSONSerialization.jsonObject(with: valid) as! [String: Any]
        for (key, value) in [("schema", 2 as Any), ("personalContentOptIn", true as Any),
                             ("title", "Private title" as Any), ("status", "playing" as Any),
                             ("expiresAt", now.addingTimeInterval(901).timeIntervalSinceReferenceDate as Any),
                             ("expiresAt", now.timeIntervalSinceReferenceDate as Any)] {
            var changed = object; changed[key] = value
            let changedData = try JSONSerialization.data(withJSONObject: changed)
            check(WidgetSnapshot.decode(changedData, now: now) == nil, "reject \(key)=\(value)")
        }
        let io = MemoryIO()
        io.data = valid
        check(WidgetSnapshotReader.status(io: io, now: now.addingTimeInterval(901)) == .stale, "expired presentation generic stale")
        check(WidgetSnapshotReader.presentation(io: io, now: now).expiresAt == now.addingTimeInterval(900), "timeline exact expiry")
        io.failRead = true
        check(WidgetSnapshotReader.status(io: io, now: now) == .unavailable, "read failure no cache fallback")
        io.failRead = false
        io.data = Data("broken".utf8)
        check(WidgetSnapshotReader.status(io: io, now: now) == .unavailable, "corrupt reader")
        io.data = Data(repeating: 32, count: 1025)
        check(WidgetSnapshotReader.status(io: io, now: now) == .unavailable, "oversize reader")
        check(WidgetSnapshotReader.status(io: nil, now: now) == .unavailable, "missing container")
        check(WidgetSnapshotFile(configuredGroup: nil) == nil, "no invented group")
        let writer = WidgetSnapshotWriter(io: io)
        try await writer.transition(to: owner, signedIn: true, now: now)
        check(WidgetSnapshotReader.status(io: io, now: now) == .unavailable, "transition replaces previous status")
        try await writer.publish(status: .current, generation: owner, now: now)
        try await writer.transition(to: next, signedIn: false, now: now)
        check(WidgetSnapshotReader.status(io: io, now: now) == .signedOut, "signed out replaces old generation")
        do { try await writer.publish(status: .current, generation: owner, now: now); preconditionFailure("old owner admitted") } catch {}
        do { try await writer.publish(status: .current, generation: next, now: now); preconditionFailure("signed out current admitted") } catch {}
        try await writer.transition(to: next, signedIn: true, now: now)
        io.failWrite = true
        do { try await writer.publish(status: .current, generation: next, now: now); preconditionFailure("write succeeded") } catch {}
        check(io.data.isEmpty, "failed publish removes persisted status")
        io.failWrite = false
        do { try await writer.publish(status: .current, generation: next, now: now); preconditionFailure("failed owner revived") } catch {}
        try await writer.transition(to: owner, signedIn: true, now: now)
        io.failRemove = true
        do { try await writer.transition(to: next, signedIn: true, now: now); preconditionFailure("remove succeeded") } catch {}
        io.failRemove = false
        do { try await writer.publish(status: .current, generation: owner, now: now); preconditionFailure("old generation retained authority") } catch {}
        io.failWrite = true
        do { try await writer.transition(to: next, signedIn: true, now: now); preconditionFailure("transition write succeeded") } catch {}
        io.failWrite = false
        do { try await writer.publish(status: .current, generation: next, now: now); preconditionFailure("failed transition admitted") } catch {}
        try await writer.transition(to: next, signedIn: true, now: now)
        for route in WidgetRoute.allCases {
            check(WidgetRoute.admitted(route.url, authenticated: true, preview: false) == route, "admit live account \(route.rawValue)")
            check(WidgetRoute.admitted(route.url, authenticated: false, preview: false) == nil, "reject signed out \(route.rawValue)")
            check(WidgetRoute.admitted(route.url, authenticated: true, preview: true) == nil, "reject preview \(route.rawValue)")
        }
        for url in ["bluefin-widget://open/library?token=x", "bluefin-widget://open/library#x", "bluefin-widget://user@open/library", "bluefin-widget://open:80/library", "bluefin-widget://open/%6cibrary", "bluefin-widget://open/library/", "bluefin-widget://open/play", "https://open/library", "bluefin-widget://open/search?q=x"] {
            check(WidgetRoute.admitted(URL(string: url)!, authenticated: true, preview: false) == nil, "reject route \(url)")
        }
        print("PASS lifecycle ownership and failure fencing; no playback AppIntent exposed")
    }
}
