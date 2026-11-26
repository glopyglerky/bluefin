import XCTest
import SwiftUI
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

@MainActor final class AdaptiveLayoutTests: XCTestCase {

    func testCategoryRoutesPreserveWidgetMusicAndVideoFilters() {
        XCTAssertEqual(BluefinCategory.allCases.map(\.title),
                       ["Home", "Sports", "TV Shows", "Movies", "Live TV", "Music"])
        XCTAssertEqual(Set(BluefinCategory.allCases.map(\.rawValue)).count, 6)
        XCTAssertEqual(BluefinCategory.movies.videoItemType, "Movie")
        XCTAssertEqual(BluefinCategory.shows.videoItemType, "Series")
        XCTAssertNil(BluefinCategory.home.videoItemType)
        #if os(iOS)
        XCTAssertEqual(BluefinCategory(rawValue: WidgetRoute.music.tab), .music)
        XCTAssertEqual(BluefinCategory(rawValue: WidgetRoute.liveTV.tab), .liveTV)
        XCTAssertNil(BluefinCategory(rawValue: WidgetRoute.search.tab),
                     "Search stays a utility, not a media category")
        #endif
    }

    #if os(iOS)
    func testHomeAndSixCategoriesFitBothAppearances() async throws {
        guard #available(iOS 16, *) else { throw XCTSkip("Hosting size measurement requires iOS 16") }
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let oldWindow = scene.windows.first(where: \.isKeyWindow)
        for scheme in [ColorScheme.light, .dark] {
            for size in [CGSize(width: 320, height: 680), CGSize(width: 390, height: 844),
                         CGSize(width: 1024, height: 768)] {
                let store = LibraryStore()
                store.explore()
                let host = UIHostingController(rootView: LibraryTabs()
                    .environmentObject(store).preferredColorScheme(scheme))
                let window = UIWindow(windowScene: scene)
                window.frame = CGRect(origin: .zero, size: size)
                window.rootViewController = host
                window.makeKeyAndVisible()
                host.view.frame = window.bounds
                host.view.setNeedsLayout()
                host.view.layoutIfNeeded()
                await Task.yield()
                let fitting = host.sizeThatFits(in: size)
                XCTAssertLessThanOrEqual(fitting.width, size.width + 0.5)
                let format = UIGraphicsImageRendererFormat()
                format.scale = 1
                let captured = UIGraphicsImageRenderer(size: size, format: format).image { context in
                    host.view.layer.render(in: context.cgContext)
                }
                let attachment = XCTAttachment(image: captured)
                attachment.name = "Bluefin Home \(scheme == .light ? "light" : "dark") \(Int(size.width))x\(Int(size.height))"
                attachment.lifetime = .keepAlways
                add(attachment)
                window.isHidden = true
                oldWindow?.makeKey()
            }
        }
    }
    #endif

    func testGuideArtworkUsesOnlyTheProgramsAdvertisedPrimaryImage() throws {
        let payload = """
        {"Id":"programme","ChannelId":"channel","ImageTags":{"Primary":"version"}}
        """
        let program = try JSONDecoder().decode(LiveProgram.self, from: Data(payload.utf8))
        XCTAssertEqual(program.artwork?.Id, "programme")
        XCTAssertEqual(program.artwork?.ImageTags?["Primary"], "version")
        XCTAssertNil(LiveProgram(Id: "missing").artwork)
        XCTAssertNil(LiveProgram(Id: "other", ImageTags: ["Primary": ""]).artwork)
    }

    func testGuideTimelineKeepsChannelRowsAligned() {
        let midnight = Date(timeIntervalSince1970: 0)
        func program(_ id: String, _ start: String, _ end: String) -> LiveProgram {
            LiveProgram(Id: id, StartDate: start, EndDate: end)
        }
        let guide = GuideTimeline(start: midnight, end: midnight.addingTimeInterval(7200))
        let programmes = [
            program("before", "1969-12-31T23:00:00Z", "1970-01-01T00:00:00Z"),
            program("first", "1969-12-31T23:45:00Z", "1970-01-01T00:30:00Z"),
            program("overlap", "1970-01-01T00:20:00Z", "1970-01-01T00:45:00Z"),
            program("later", "1970-01-01T01:00:00Z", "1970-01-01T03:00:00Z"),
            program("invalid", "1970-01-01T01:15:00Z", "1970-01-01T01:00:00Z"),
        ]
        let cells = guide.segments(programmes)
        XCTAssertEqual(cells.map { $0.program?.Id }, ["first", "overlap", nil, "later"])
        XCTAssertEqual(cells.map { guide.fraction(at: $0.start) }, [0, 0.25, 0.375, 0.5])
        XCTAssertEqual(cells.map { guide.fraction(at: $0.end) }, [0.25, 0.375, 0.5, 1])
        XCTAssertEqual(guide.segments([]).first?.end, guide.end)
    }

    #if os(tvOS)
    func testTVSignInRendersAtWideAndCompactSizes() throws {
        guard #available(tvOS 16.0, *) else { throw XCTSkip("Hosting size measurement requires tvOS 16") }
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let store = LibraryStore()
        let oldWindow = scene.windows.first(where: \.isKeyWindow)
        for size in [CGSize(width: 1920, height: 1080), CGSize(width: 1280, height: 720)] {
            let host = UIHostingController(rootView: TVWelcomeView().environmentObject(store).preferredColorScheme(.dark))
            let window = UIWindow(windowScene: scene)
            window.frame = CGRect(origin: .zero, size: size)
            window.rootViewController = host
            window.makeKeyAndVisible()
            host.view.frame = window.bounds
            host.view.setNeedsLayout()
            host.view.layoutIfNeeded()
            defer { window.isHidden = true; oldWindow?.makeKey() }

            let fitting = host.sizeThatFits(in: size)
            XCTAssertLessThanOrEqual(fitting.width, size.width + 0.5)
            let attributes = UIGraphicsImageRendererFormat()
            attributes.scale = 1
            let captured = UIGraphicsImageRenderer(size: size, format: attributes).image { context in
                host.view.layer.render(in: context.cgContext)
            }
            let attachment = XCTAttachment(image: captured)
            attachment.name = "Bluefin sign-in \(Int(size.width))x\(Int(size.height))"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }
    #endif

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
