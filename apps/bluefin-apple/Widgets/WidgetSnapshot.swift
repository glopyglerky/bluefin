import Foundation

/// Public-safe status only. Personal content is intentionally not representable in this schema.
struct WidgetSnapshot: Codable, Equatable, Sendable {
    enum Status: String, Codable, Sendable { case signedOut, unavailable, stale, current }
    static let maximumBytes = 1024
    static let maximumLifetime: TimeInterval = 900
    let schema: Int
    let accountGeneration: UUID
    let capturedAt: Date
    let expiresAt: Date
    let status: Status
    let personalContentOptIn: Bool

    init(generation: UUID, status: Status, now: Date) {
        schema = 1
        accountGeneration = generation
        capturedAt = now
        expiresAt = now.addingTimeInterval(Self.maximumLifetime)
        self.status = status
        personalContentOptIn = false
    }

    static func decode(_ data: Data, now: Date, expectedGeneration: UUID? = nil) -> WidgetSnapshot? {
        guard data.count <= maximumBytes,
              // Reject extra keys, including attempts to add personal fields to this generic schema.
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(object.keys) == Set(["schema", "accountGeneration", "capturedAt", "expiresAt", "status", "personalContentOptIn"]),
              let value = try? JSONDecoder().decode(Self.self, from: data),
              value.schema == 1, !value.personalContentOptIn,
              expectedGeneration == nil || expectedGeneration == value.accountGeneration,
              value.capturedAt.timeIntervalSinceReferenceDate.isFinite,
              value.expiresAt.timeIntervalSinceReferenceDate.isFinite,
              value.capturedAt <= now, value.expiresAt > now,
              value.expiresAt > value.capturedAt,
              value.expiresAt.timeIntervalSince(value.capturedAt) <= maximumLifetime else { return nil }
        return value
    }
}

protocol WidgetSnapshotIO: Sendable {
    func read() throws -> Data
    func replace(_ data: Data) throws
    func remove() throws
}

/// Only constructed from a real, entitled App Group. No default group, documents-directory fallback or networking.
struct WidgetSnapshotFile: WidgetSnapshotIO {
    private let url: URL
    init?(configuredGroup: String?) {
        #if os(iOS) && !targetEnvironment(macCatalyst)
        guard let configuredGroup, configuredGroup.hasPrefix("group."),
              let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: configuredGroup) else { return nil }
        url = container.appendingPathComponent("bluefin-widget-status-v1.json", isDirectory: false)
        #else
        // This lane does not admit macOS's non-validating App Group URL behavior.
        return nil
        #endif
    }
    func read() throws -> Data {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        return try handle.read(upToCount: WidgetSnapshot.maximumBytes + 1) ?? Data()
    }
    func replace(_ data: Data) throws {
        #if os(iOS) || os(watchOS) || os(visionOS)
        try data.write(to: url, options: [.atomic, .completeFileProtection])
        #else
        try data.write(to: url, options: .atomic)
        #endif
    }
    func remove() throws {
        if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
    }
}

enum WidgetSnapshotReader {
    struct Presentation {
        let status: WidgetSnapshot.Status
        let expiresAt: Date?
    }
    static func presentation(io: (any WidgetSnapshotIO)?, now: Date) -> Presentation {
        guard let io, let data = try? io.read(), data.count <= WidgetSnapshot.maximumBytes else { return .init(status: .unavailable, expiresAt: nil) }
        if let value = WidgetSnapshot.decode(data, now: now) {
            return .init(status: value.status, expiresAt: value.expiresAt)
        }
        // Expired data never becomes a usable snapshot. Only a generic stale label survives.
        if let raw = try? JSONDecoder().decode(WidgetSnapshot.self, from: data),
           let value = WidgetSnapshot.decode(data, now: raw.capturedAt), value.expiresAt <= now {
            return .init(status: .stale, expiresAt: nil)
        }
        return .init(status: .unavailable, expiresAt: nil)
    }
    static func status(io: (any WidgetSnapshotIO)?, now: Date) -> WidgetSnapshot.Status {
        presentation(io: io, now: now).status
    }
}

/// One app-process writer. The extension is read-only. A generation token is an ownership fence, not account authority.
actor WidgetSnapshotWriter {
    private let io: (any WidgetSnapshotIO)?
    private var generation: UUID?
    private var healthy = false
    private var signedIn = false
    init(io: (any WidgetSnapshotIO)?) { self.io = io }

    /// Invoke before an account transition. Invalidate old work before touching disk.
    /// Failure prohibits publishing for the new owner. No personal data is ever persisted.
    func transition(to next: UUID, signedIn: Bool, now: Date) throws {
        generation = nil
        healthy = false
        guard let io else { throw Failure.unavailable }
        do {
            try io.remove()
            try io.replace(JSONEncoder().encode(WidgetSnapshot(generation: next, status: signedIn ? .unavailable : .signedOut, now: now)))
            generation = next
            self.signedIn = signedIn
            healthy = true
        } catch { try? io.remove(); throw error }
    }

    func publish(status: WidgetSnapshot.Status, generation owner: UUID, now: Date) throws {
        guard healthy, generation == owner, signedIn || status == .signedOut, let io else { throw Failure.unavailable }
        do { try io.replace(JSONEncoder().encode(WidgetSnapshot(generation: owner, status: status, now: now))) }
        catch {
            healthy = false
            generation = nil
            try? io.remove()
            throw error
        }
    }
    enum Failure: Error { case unavailable }
}
