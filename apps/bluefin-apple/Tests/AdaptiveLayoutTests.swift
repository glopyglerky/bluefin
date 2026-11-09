import XCTest
import SwiftUI
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

@MainActor final class AdaptiveLayoutTests: XCTestCase {
    #if os(iOS)
    func testNowPlayingFitsNarrowWindow() async throws {
        guard #available(iOS 16, tvOS 16, *) else { throw XCTSkip("Hosting size measurement requires OS 16") }
        let store = LibraryStore()
        store.explore()
        let url = try XCTUnwrap(MediaItem.previewAudio.previewURL)
        let playback = try PlaybackController(item: .previewAudio, url: url, plan: nil, session: nil)
        defer { playback.stop() }
        let host = UIHostingController(rootView: MusicPlayer(playback: playback).environmentObject(store))
        let size = host.sizeThatFits(in: CGSize(width: 320, height: 2_000))
        XCTAssertLessThanOrEqual(size.width, 320.5, "Now Playing must not overflow a narrow app window")
    }
    func testResizingPreservesPlaybackAndDuplicateQueue() async throws {
        guard #available(iOS 16, *) else { throw XCTSkip("Hosting size measurement requires OS 16") }
        let store = LibraryStore(); store.explore()
        let playback = try PlaybackController(item: .previewAudio, url: XCTUnwrap(MediaItem.previewAudio.previewURL), plan: nil, session: nil)
        defer { playback.stop() }
        store.playback = playback; store.musicQueue = [.previewAudio, .previewAudio]; store.musicIndex = 1
        let host = UIHostingController(rootView: LibraryTabs().environmentObject(store))
        // These are viewport fixtures, not a claim of actual Duo hardware/pose acceptance.
        for size in [CGSize(width: 1024, height: 768), CGSize(width: 320, height: 680),
                     CGSize(width: 626, height: 890), CGSize(width: 890, height: 626),
                     CGSize(width: 768, height: 1024)] {
            host.view.frame = CGRect(origin: .zero, size: size)
            host.view.setNeedsLayout(); host.view.layoutIfNeeded()
            let fitted = host.sizeThatFits(in: size)
            XCTAssertLessThanOrEqual(fitted.width, size.width + 0.5)
            XCTAssertTrue(store.playback === playback)
            XCTAssertEqual(store.musicQueue.map(\.Id), [MediaItem.previewAudio.Id, MediaItem.previewAudio.Id])
            XCTAssertEqual(store.musicIndex, 1)
        }
    }
    #endif
}
