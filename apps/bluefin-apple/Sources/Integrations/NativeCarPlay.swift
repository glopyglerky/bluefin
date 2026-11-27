#if os(iOS) && BLUEFIN_CARPLAY
import CarPlay
import Combine

/// The system owns layout, focus and driving restrictions. Requires the approved audio entitlement.
@MainActor final class NativeCarPlaySceneDelegate: UIResponder, CPTemplateApplicationSceneDelegate,
    CPSessionConfigurationDelegate, CPNowPlayingTemplateObserver {
    private var controller: CPInterfaceController?
    private var carSession: CPSessionConfiguration?
    private var subscriptions: Set<AnyCancellable> = []
    private var tasks: [UUID: Task<Void, Never>] = [:]
    private var generation = UUID()
    private var playingRows: [PlayingRow] = []
    private enum PlaybackKind { case music, radio, video }
    private final class PlayingRow {
        weak var row: CPListItem?
        let itemID: String
        init(_ row: CPListItem, itemID: String) { self.row = row; self.itemID = itemID }
    }

    func templateApplicationScene(_ templateApplicationScene: CPTemplateApplicationScene, didConnect interfaceController: CPInterfaceController) {
        controller = interfaceController
        carSession = CPSessionConfiguration(delegate: self)
        CPNowPlayingTemplate.shared.add(self)
        CPNowPlayingTemplate.shared.isUpNextButtonEnabled = true
        CPNowPlayingTemplate.shared.upNextTitle = "Playing Next"
        guard let store = NativeIntegrations.shared.store else {
            interfaceController.setRootTemplate(message("Open Bluefin", detail: "Sign in on your iPhone to connect your library."), animated: false, completion: nil)
            return
        }
        store.$session.sink { [weak self] _ in
            Task { @MainActor [weak self] in self?.reload() }
        }.store(in: &subscriptions)
        store.$playback.sink { [weak self] _ in
            Task { @MainActor [weak self] in self?.updatePlayingRows() }
        }.store(in: &subscriptions)
        reload()
    }

    private func reload() {
        guard let controller, let store = NativeIntegrations.shared.store else { return }
        generation = UUID()
        tasks.values.forEach { $0.cancel() }; tasks.removeAll(); playingRows.removeAll()
        guard store.session != nil, !store.preview else {
            controller.setRootTemplate(message("Sign in on iPhone", detail: "Your Bluefin library appears after you sign in."), animated: false, completion: nil)
            return
        }
        let home = message("Home", detail: "Loading recently added songs")
        let favorites = message("Favorites", detail: "Loading favorite songs")
        let library = libraryTemplate()
        let radio = message("Radio", detail: "Loading your radio channels")
        home.tabTitle = "Home"; home.tabImage = UIImage(systemName: "house.fill")
        favorites.tabTitle = "Favorites"; favorites.tabImage = UIImage(systemName: "star.fill")
        library.tabTitle = "Library"; library.tabImage = UIImage(systemName: "music.note.list")
        radio.tabTitle = "Radio"; radio.tabImage = UIImage(systemName: "dot.radiowaves.left.and.right")
        var tabs: [CPTemplate] = [home, favorites, library, radio]
        if videoSupported {
            let video = list("Video", items: (store.home.resume + store.home.latest).filter {
                ["Movie", "Episode", "Video", "TvChannel"].contains($0.Type ?? "")
            }, kind: .video)
            video.tabTitle = "Video"; video.tabImage = UIImage(systemName: "play.tv")
            tabs.append(video)
        }
        controller.setRootTemplate(CPTabBarTemplate(templates: tabs), animated: false, completion: nil)
        load(title: "Recently added", type: "Audio", sort: "DateCreated", descending: true, into: home)
        load(title: "Favorite songs", type: "Audio", favorites: true, into: favorites)
        loadRadio(into: radio)
    }

    private func libraryTemplate() -> CPListTemplate {
        let rows = [
            ("Playlists", "Playlist", "music.note.list"),
            ("Artists", "MusicArtist", "music.mic"),
            ("Albums", "MusicAlbum", "square.stack"),
            ("Songs", "Audio", "music.note"),
        ].map { title, type, symbol -> CPListItem in
            let row = CPListItem(text: title, detailText: nil, image: UIImage(systemName: symbol))
            row.accessoryType = .disclosureIndicator
            row.handler = { [weak self] _, complete in
                self?.open(title: title, type: type); complete()
            }
            return row
        }
        return CPListTemplate(title: "Library", sections: [CPListSection(items: rows)])
    }

    private func open(title: String, type: String, parent: MediaItem? = nil) {
        let template = message(title, detail: "Loading music")
        controller?.pushTemplate(template, animated: true, completion: nil)
        load(title: title, type: type, parent: parent, into: template)
    }

    private func load(title: String, type: String, parent: MediaItem? = nil, start: Int = 0,
                      sort: String = "SortName", descending: Bool = false, favorites: Bool = false,
                      into template: CPListTemplate) {
        perform { [weak self, weak template] store, context, generation in
            guard let self, let template else { return }
            do {
                let page = try await store.musicItems(type: type, parent: parent, start: start,
                    sort: parent?.Type == "MusicAlbum" ? "ParentIndexNumber,IndexNumber" : sort,
                    descending: descending, favorites: favorites, limit: 20)
                try Task.checkCancellation()
                guard self.current(store, context, generation) else { return }
                var rows = self.rows(page.Items, kind: .music)
                let next = start + page.Items.count
                if !page.Items.isEmpty && next < (page.TotalRecordCount ?? next) {
                    let more = CPListItem(text: "More " + title.lowercased(), detailText: "Continue browsing", image: UIImage(systemName: "chevron.right"))
                    more.handler = { [weak self, weak template] _, complete in
                        if let template {
                            self?.load(title: title, type: type, parent: parent, start: next,
                                sort: sort, descending: descending, favorites: favorites, into: template)
                        }
                        complete()
                    }
                    rows.append(more)
                }
                if rows.isEmpty { rows = [self.information("No music found", detail: "Add music to your Bluefin library.")] }
                template.updateSections([CPListSection(items: rows)])
                self.artwork(items: page.Items, rows: rows, store: store, context: context, generation: generation)
            } catch is CancellationError {} catch {
                guard self.current(store, context, generation) else { return }
                let retry = CPListItem(text: "Try again", detailText: error.localizedDescription, image: UIImage(systemName: "arrow.clockwise"))
                retry.handler = { [weak self, weak template] _, complete in
                    if let template { self?.load(title: title, type: type, parent: parent, start: start,
                        sort: sort, descending: descending, favorites: favorites, into: template) }
                    complete()
                }
                template.updateSections([CPListSection(items: [retry])])
            }
        }
    }

    private func loadRadio(into template: CPListTemplate) {
        perform { [weak self, weak template] store, context, generation in
            guard let self, let template else { return }
            do {
                let page: ItemPage = try await store.authenticated("liveChannels", fields: ["type": "Radio", "limit": 60, "startIndex": 0])
                try Task.checkCancellation()
                guard self.current(store, context, generation) else { return }
                let rows = self.rows(page.Items, kind: .radio)
                template.updateSections([CPListSection(items: rows.isEmpty ?
                    [self.information("No radio channels", detail: "Radio channels from your server appear here.")] : rows)])
                self.artwork(items: page.Items, rows: rows, store: store, context: context, generation: generation)
            } catch is CancellationError {} catch {
                guard self.current(store, context, generation) else { return }
                template.updateSections([CPListSection(items: [self.information("Radio unavailable", detail: error.localizedDescription)])])
            }
        }
    }

    private func list(_ title: String, items: [MediaItem], kind: PlaybackKind) -> CPListTemplate {
        CPListTemplate(title: title, sections: [CPListSection(items: rows(items, kind: kind))])
    }

    private func rows(_ items: [MediaItem], kind: PlaybackKind) -> [CPListItem] {
        items.prefix(60).map { item in
            let row = CPListItem(text: item.title, detailText: item.subtitle,
                image: UIImage(systemName: item.isContainer ? "square.stack" : kind == .radio ? "radio" : "music.note"))
            if item.isContainer && kind == .music {
                row.accessoryType = .disclosureIndicator
                row.handler = { [weak self] _, complete in
                    self?.open(title: item.title, type: item.Type == "MusicArtist" ? "MusicAlbum" : "Audio", parent: item)
                    complete()
                }
            } else {
                if kind == .video, #available(iOS 26.4, *), let carSession { BluefinCarPlayVideo.configure(row, item: item, session: carSession) }
                row.handler = { [weak self] _, complete in
                    guard let self else { complete(); return }
                    self.perform { [weak self] store, context, generation in
                        defer { complete() }
                        guard let self, self.current(store, context, generation),
                              kind != .video || self.videoSupported else { return }
                        if kind == .music {
                            let tracks = items.filter { $0.Type == "Audio" }
                            guard let index = tracks.firstIndex(where: { $0.Id == item.Id }) else { return }
                            await store.playMusic(tracks, at: index)
                        } else { await store.play(item) }
                        guard !Task.isCancelled, self.current(store, context, generation),
                              store.playback?.item.Id == item.Id else { return }
                        self.updatePlayingRows()
                        self.controller?.pushTemplate(CPNowPlayingTemplate.shared, animated: true, completion: nil)
                    }
                }
                playingRows.append(PlayingRow(row, itemID: item.Id))
                row.isPlaying = NativeIntegrations.shared.store?.playback?.item.Id == item.Id
            }
            return row
        }
    }

    private func artwork(items: [MediaItem], rows: [CPListItem], store: LibraryStore, context: UUID, generation: UUID) {
        guard let session = store.session else { return }
        let pairs = Array(zip(items, rows))
        perform { [weak self] store, _, _ in
            // Two requests at a time; decoded artwork shares the app's account-fenced cache.
            for index in stride(from: 0, to: pairs.count, by: 2) {
                guard let self, !Task.isCancelled, self.current(store, context, generation) else { return }
                let end = min(index + 2, pairs.count)
                await withTaskGroup(of: Void.self) { group in
                    for pair in pairs[index..<end] {
                        group.addTask { @MainActor [weak self] in
                            let image = await Artwork.nativeImage(for: pair.0, session: session, thumbnailWidth: 192)
                            guard let self, !Task.isCancelled, self.current(store, context, generation), let image else { return }
                            pair.1.setImage(image)
                        }
                    }
                }
            }
        }
    }

    private func perform(_ action: @escaping @MainActor (LibraryStore, UUID, UUID) async -> Void) {
        guard let store = NativeIntegrations.shared.store else { return }
        let id = UUID(), generation = generation, context = store.accountContext
        tasks[id] = Task { @MainActor [weak self] in
            defer { self?.tasks[id] = nil }
            guard let self, self.current(store, context, generation) else { return }
            await action(store, context, generation)
        }
    }
    private func current(_ store: LibraryStore, _ context: UUID, _ expected: UUID) -> Bool {
        !Task.isCancelled && generation == expected && store.accountContext == context && store.session != nil && controller != nil
    }
    private func updatePlayingRows() {
        let id = NativeIntegrations.shared.store?.playback?.item.Id
        playingRows.removeAll { $0.row == nil }
        for entry in playingRows { entry.row?.isPlaying = entry.itemID == id }
    }
    private func information(_ text: String, detail: String) -> CPListItem {
        let row = CPListItem(text: text, detailText: detail); row.isEnabled = false; return row
    }
    private func message(_ title: String, detail: String) -> CPListTemplate {
        CPListTemplate(title: title, sections: [CPListSection(items: [information(title, detail: detail)])])
    }
    private var videoSupported: Bool {
        if #available(iOS 26.4, *) { return carSession?.supportsVideoPlayback == true }
        return false
    }
    func nowPlayingTemplateUpNextButtonTapped(_ nowPlayingTemplate: CPNowPlayingTemplate) {
        guard let store = NativeIntegrations.shared.store, store.session != nil else { return }
        let remaining = Array(store.musicQueue.dropFirst(max(0, store.musicIndex)))
        controller?.pushTemplate(list("Playing Next", items: remaining, kind: .music), animated: true, completion: nil)
    }
    func sessionConfiguration(_ sessionConfiguration: CPSessionConfiguration, limitedUserInterfacesChanged limitedUserInterfaces: CPLimitableUserInterface) { reload() }
    func templateApplicationScene(_ templateApplicationScene: CPTemplateApplicationScene, didDisconnectInterfaceController interfaceController: CPInterfaceController) {
        generation = UUID()
        tasks.values.forEach { $0.cancel() }; tasks.removeAll(); playingRows.removeAll()
        subscriptions.removeAll()
        CPNowPlayingTemplate.shared.remove(self)
        controller = nil; carSession = nil
    }
}
#endif
