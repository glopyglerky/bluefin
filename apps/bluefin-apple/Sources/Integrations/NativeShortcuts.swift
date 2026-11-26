#if os(iOS) && canImport(AppIntents)
import AppIntents

@available(iOS 16.0, *)
struct OpenBluefinLibraryIntent: AppIntent {
    static var title: LocalizedStringResource = "Open Bluefin library"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    static var openAppWhenRun: Bool = true
    @MainActor func perform() async throws -> some IntentResult {
        guard let open = NativeIntegrations.shared.openLibrary else { throw BluefinError.message("Open Bluefin's library from the app.") }
        open()
        return .result()
    }
}

@available(iOS 16.0, *)
struct PlayBluefinTitleIntent: AppIntent {
    static var title: LocalizedStringResource = "Play a Bluefin title"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    static var openAppWhenRun: Bool = true
    @Parameter(title: "Title") var name: String
    @MainActor func perform() async throws -> some IntentResult {
        try await NativeIntegrations.shared.playNamed(name)
        return .result()
    }
}

@available(iOS 16.0, *)
struct PauseBluefinIntent: AppIntent {
    static var title: LocalizedStringResource = "Pause Bluefin"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    static var openAppWhenRun: Bool = true
    @MainActor func perform() async throws -> some IntentResult {
        guard let playback = try NativeIntegrations.shared.requireStore().playback else { throw BluefinError.message("Nothing is playing in Bluefin.") }
        playback.player.pause()
        return .result()
    }
}

@available(iOS 16.0, *)
struct ResumeBluefinIntent: AppIntent {
    static var title: LocalizedStringResource = "Resume Bluefin"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    static var openAppWhenRun: Bool = true
    @MainActor func perform() async throws -> some IntentResult {
        guard let playback = try NativeIntegrations.shared.requireStore().playback else { throw BluefinError.message("Nothing is playing in Bluefin.") }
        playback.player.play()
        return .result()
    }
}

@available(iOS 16.0, *)
struct BluefinAppShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: RequestBluefinMediaIntent(), phrases: ["Request media in \(.applicationName)"], shortTitle: "Request media", systemImageName: "plus.circle")
        AppShortcut(intent: SearchBluefinLibraryIntent(), phrases: ["Search my library in \(.applicationName)"], shortTitle: "Search library", systemImageName: "magnifyingglass")
        AppShortcut(intent: BluefinItemInformationIntent(), phrases: ["Get item information in \(.applicationName)"], shortTitle: "Item information", systemImageName: "info.circle")
        AppShortcut(intent: PlayBluefinItemIntent(), phrases: ["Play a library item in \(.applicationName)"], shortTitle: "Play library item", systemImageName: "play")
        AppShortcut(intent: OpenBluefinLibraryIntent(), phrases: ["Open my library in \(.applicationName)"], shortTitle: "Open library", systemImageName: "rectangle.stack")
        AppShortcut(intent: PlayBluefinTitleIntent(), phrases: ["Play a title in \(.applicationName)"], shortTitle: "Play title", systemImageName: "play")
        AppShortcut(intent: PauseBluefinIntent(), phrases: ["Pause \(.applicationName)"], shortTitle: "Pause", systemImageName: "pause")
        AppShortcut(intent: ResumeBluefinIntent(), phrases: ["Resume \(.applicationName)"], shortTitle: "Resume", systemImageName: "play")
    }
}
#endif
