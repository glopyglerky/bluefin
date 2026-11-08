#if os(iOS) && BLUEFIN_CARPLAY
import CarPlay
import Combine

/// Opt-in scene delegate. Requires an Apple-approved audio entitlement and scene manifest.
@MainActor final class NativeCarPlaySceneDelegate: UIResponder, CPTemplateApplicationSceneDelegate, CPSessionConfigurationDelegate {
    private var controller: CPInterfaceController?
    private var carSession: CPSessionConfiguration?
    private var subscriptions: Set<AnyCancellable> = []
    private var tasks: [UUID: Task<Void, Never>] = [:]
    private var generation = UUID()

    func templateApplicationScene(_ templateApplicationScene: CPTemplateApplicationScene, didConnect interfaceController: CPInterfaceController) {
        controller = interfaceController
        carSession = CPSessionConfiguration(delegate: self)
        guard let store = NativeIntegrations.shared.store else {
            interfaceController.setRootTemplate(CPListTemplate(title: "Open Bluefin and sign in", sections: []), animated: false, completion: nil)
            return
        }
        Publishers.CombineLatest3(store.$home, store.$libraries, store.$session).sink { [weak self] _, _, _ in
            // Published values are emitted before storage; read on the next main actor turn.
            Task { @MainActor [weak self] in self?.reload() }
        }.store(in: &subscriptions)
        reload()
    }

    private func reload() {
        guard let controller, let store = NativeIntegrations.shared.store else { return }
        generation = UUID()
        tasks.values.forEach { $0.cancel() }; tasks.removeAll()
        guard store.session != nil, !store.preview else {
            controller.setRootTemplate(CPListTemplate(title: "Sign in on iPhone", sections: []), animated: false, completion: nil)
            return
        }
        let audio = (store.home.resume + store.home.latest).filter { $0.Type == "Audio" }
        var seen = Set<String>()
        let items = audio.filter { seen.insert($0.Id).inserted }
        let recent = list("Recent audio", items: items)
        let favorites = list("Favorites", items: items.filter { $0.UserData?.IsFavorite == true })
        let libraries = store.libraries.filter { $0.CollectionType == "music" }.prefix(20).map { library -> CPListItem in
            let row = CPListItem(text: library.title, detailText: "Browse music")
            row.handler = { [weak self] _, completion in
                guard let self else { completion(); return }
                let id = UUID(), generation = self.generation
                self.tasks[id] = Task { @MainActor [weak self] in
                    defer { completion(); self?.tasks[id] = nil }
                    do {
                        let page = try await store.items(parent: library)
                        try Task.checkCancellation()
                        guard let self, self.generation == generation else { return }
                        let tracks = page.Items.filter { $0.Type == "Audio" }
                        // This first page intentionally omits folders and video; no recursive scan.
                        self.controller?.pushTemplate(self.list(library.title, items: tracks), animated: true, completion: nil)
                    } catch is CancellationError {} catch { store.error = error.localizedDescription }
                }
            }
            return row
        }
        let library = CPListTemplate(title: "Music libraries", sections: [CPListSection(items: Array(libraries))])
        recent.tabTitle = "Recent"; favorites.tabTitle = "Favorites"; library.tabTitle = "Library"
        var tabs: [CPTemplate] = [recent, favorites, library]
        if #available(iOS 26.4, *), let carSession, carSession.supportsVideoPlayback {
            let videos = (store.home.resume + store.home.latest).filter { ["Movie", "Episode", "Video", "TvChannel"].contains($0.Type ?? "") }
            let video = list("Video", items: videos, video: true)
            video.tabTitle = "Video"
            tabs.append(video)
        }
        controller.setRootTemplate(CPTabBarTemplate(templates: tabs), animated: false, completion: nil)
    }

    private func list(_ title: String, items: [MediaItem], video: Bool = false) -> CPListTemplate {
        let rows = items.prefix(60).map { item -> CPListItem in
            let row = CPListItem(text: item.title, detailText: nil)
            if video, #available(iOS 26.4, *), let carSession { BluefinCarPlayVideo.configure(row, item: item, session: carSession) }
            row.handler = { [weak self] _, completion in
                guard let self, let store = NativeIntegrations.shared.store, let account = store.session else { completion(); return }
                let id = UUID(), generation = self.generation
                self.tasks[id] = Task { @MainActor [weak self] in
                    defer { completion(); self?.tasks[id] = nil }
                    guard !Task.isCancelled, self?.generation == generation,
                          store.session.map(NativeIntegrations.identity) == NativeIntegrations.identity(account),
                          item.Type == "Audio" || (video && self?.videoSupported == true) else { return }
                    await store.play(item)
                    guard !Task.isCancelled, store.playback?.item.Id == item.Id else { return }
                    self?.controller?.pushTemplate(CPNowPlayingTemplate.shared, animated: true, completion: nil)
                }
            }
            return row
        }
        return CPListTemplate(title: title, sections: [CPListSection(items: rows)])
    }

    private var videoSupported: Bool {
        if #available(iOS 26.4, *) { return carSession?.supportsVideoPlayback == true }
        return false
    }

    func sessionConfiguration(_ sessionConfiguration: CPSessionConfiguration, limitedUserInterfacesChanged limitedUserInterfaces: CPLimitableUserInterface) { reload() }

    func templateApplicationScene(_ templateApplicationScene: CPTemplateApplicationScene, didDisconnectInterfaceController interfaceController: CPInterfaceController) {
        generation = UUID()
        tasks.values.forEach { $0.cancel() }; tasks.removeAll()
        subscriptions.removeAll()
        controller = nil
        carSession = nil
    }
}
#endif
