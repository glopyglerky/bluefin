import AppIntents
import Foundation

@MainActor enum WatchIntentAccess {
    weak static var store: WatchStore?
    static func require() throws -> WatchStore {
        guard let store, store.visibleAccount != nil else { throw WatchFailure.message("Open Bluefin on this Watch and finish sign-in or cleanup first.") }
        return store
    }
}
struct SearchWatchMusicIntent: AppIntent {
    static var title: LocalizedStringResource = "Search Bluefin Watch music"
    static var openAppWhenRun = true
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Search") var search: String
    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        let store = try WatchIntentAccess.require()
        guard let account = store.account, !search.isEmpty, search.count <= 200 else { throw WatchFailure.message("Enter a music search of up to 200 characters.") }
        let epoch = store.accountContext
        let page = try await WatchAPI.page(account, favorites: false, search: search)
        guard store.accountContext == epoch, store.visibleAccount == account else { throw CancellationError() }
        let count = page.TotalRecordCount.map { "\($0) matches" } ?? "At least \(page.Items.count) matches"
        let titles = page.Items.prefix(5).map(\.Name).joined(separator: ", ")
        return .result(dialog: IntentDialog(stringLiteral: "\(count). \(titles). Open Music to browse all results."))
    }
}
struct PlayWatchMusicIntent: AppIntent {
    static var title: LocalizedStringResource = "Play Bluefin Watch music"
    static var openAppWhenRun = true
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Title") var title: String
    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        let store = try WatchIntentAccess.require()
        guard let account = store.account, !title.isEmpty, title.count <= 200 else { throw WatchFailure.message("Enter a music title.") }
        let epoch = store.accountContext
        let page = try await WatchAPI.page(account, favorites: false, search: title)
        guard store.accountContext == epoch, store.visibleAccount == account else { throw CancellationError() }
        guard let index = NativeTitleSelection.index(titles: page.Items.map(\.Name), containers: page.Items.map { $0.Type != "Audio" }, total: page.TotalRecordCount, query: title) else {
            throw WatchFailure.message("No unique exact title was found. Choose your track in Bluefin on Watch.")
        }
        let track = page.Items[index]
        await store.play(track)
        guard store.accountContext == epoch, store.visibleAccount == account, store.current?.id == track.id else { throw WatchFailure.message("Playback could not start. Check Bluefin on Watch.") }
        return .result(dialog: "Playing \(track.Name) on Watch.")
    }
}
enum WatchMusicControl: String, AppEnum {
    case pause, resume, next, previous, stop, favorite
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Watch music control"
    static var caseDisplayRepresentations: [Self: DisplayRepresentation] = [.pause: "Pause", .resume: "Resume", .next: "Next", .previous: "Previous", .stop: "Stop", .favorite: "Favorite"]
}
struct ControlWatchMusicIntent: AppIntent {
    static var title: LocalizedStringResource = "Control Bluefin Watch music"
    static var openAppWhenRun = true
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    @Parameter(title: "Control") var control: WatchMusicControl
    @MainActor func perform() async throws -> some IntentResult {
        let store = try WatchIntentAccess.require()
        guard !store.busy else { throw WatchFailure.message("Bluefin is busy. Try again shortly.") }
        switch control {
        case .pause: if store.playing { store.toggle() }
        case .resume: if !store.playing { store.toggle() }
        case .next: await store.skip(1)
        case .previous: await store.skip(-1)
        case .stop: await store.stop()
        case .favorite: await store.favorite()
        }
        if let problem = store.problem { throw WatchFailure.message(problem) }
        return .result()
    }
}
struct WatchLibraryShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: SearchWatchMusicIntent(), phrases: ["Search music in \(.applicationName)"], shortTitle: "Search music", systemImageName: "magnifyingglass")
        AppShortcut(intent: PlayWatchMusicIntent(), phrases: ["Play music in \(.applicationName)"], shortTitle: "Play music", systemImageName: "play")
        AppShortcut(intent: ControlWatchMusicIntent(), phrases: ["Control music in \(.applicationName)"], shortTitle: "Control music", systemImageName: "music.note")
    }
}
