import Foundation

// Compiled with production Models.swift, ArtworkPlan.swift and CoreAdmission.swift.
// No network, account, Keychain, player, or generated stand-in model participates.
private actor AdmissionEvents {
    private var entered = false
    private var acquired = false
    func begin() { entered = true }
    func finish() { acquired = true }
    func didEnter() -> Bool { entered }
    func didAcquire() -> Bool { acquired }
}

@main struct ArtworkPerformanceChecks {
    static func main() async throws {
        let decoder = JSONDecoder()
        let episode = try decoder.decode(MediaItem.self, from: Data(#"""
        {"Id":"episode","Type":"Episode","SeriesId":"series","SeriesPrimaryImageTag":"poster-v2",
         "ImageTags":{"Primary":"still-v1"},"ParentBackdropItemId":"series","ParentBackdropImageTags":["wide-v3"]}
        """#.utf8))
        let posters = ArtworkPlan.sources(for: episode, backdrop: false)
        precondition(posters.first == ArtworkSource(id: "series", kind: "Primary", tag: "poster-v2"),
                     "Episode cards must use the series poster before an episode still")
        precondition(ArtworkPlan.sources(for: episode, backdrop: true).first ==
                     ArtworkSource(id: "series", kind: "Backdrop/0", tag: "wide-v3"))
        let song = try decoder.decode(MediaItem.self, from: Data(#"""
        {"Id":"song","Type":"Audio","AlbumId":"album","AlbumPrimaryImageTag":"cover",
         "ParentPrimaryImageItemId":"album","ParentPrimaryImageTag":"cover"}
        """#.utf8))
        precondition(ArtworkPlan.sources(for: song, backdrop: false) ==
                     [ArtworkSource(id: "album", kind: "Primary", tag: "cover")],
                     "Album art should be inherited once")
        let invalid = MediaItem(Id: "safe", Type: "Episode", SeriesId: "../other", SeriesPrimaryImageTag: "tag")
        precondition(ArtworkPlan.sources(for: invalid, backdrop: false) ==
                     [ArtworkSource(id: "safe", kind: "Primary", tag: nil)])
        precondition(ArtworkPlan.pixelWidth(-5, backdrop: false) == 96)
        precondition(ArtworkPlan.pixelWidth(9000, backdrop: true) == 1920)

        let admission = CoreAdmission()
        let first = try await admission.acquire(artwork: true)
        let second = try await admission.acquire(artwork: true)
        let events = AdmissionEvents()
        let queued = Task {
            await events.begin()
            let permit = try await admission.acquire(artwork: true)
            await events.finish()
            return permit
        }
        while !(await events.didEnter()) { await Task.yield() }
        let foreground = try await admission.acquire()
        let early = await events.didAcquire()
        precondition(!early, "Artwork occupied the foreground reserve")
        await admission.release(foreground)
        for _ in 0..<20 { await Task.yield() }
        let beforeRelease = await events.didAcquire()
        precondition(!beforeRelease, "A third image bypassed the two-image bound")
        await admission.release(first)
        let third = try await queued.value
        let afterRelease = await events.didAcquire()
        precondition(afterRelease)
        let cancelled = Task { try await admission.acquire(artwork: true) }
        cancelled.cancel()
        do {
            let unexpected = try await cancelled.value
            await admission.release(unexpected)
            preconditionFailure("Cancelled queued artwork acquired capacity")
        } catch is CancellationError {}
        await admission.release(second)
        await admission.release(third)
        let recovered = try await admission.acquire()
        await admission.release(recovered)
        print("PASS: episode and album artwork inheritance, deduplication, path and size bounds; foreground reserve, artwork capacity and cancellation recovery")
    }
}
