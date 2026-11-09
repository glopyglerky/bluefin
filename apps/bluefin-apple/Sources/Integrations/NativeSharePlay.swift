#if BLUEFIN_SHAREPLAY && canImport(GroupActivities)
import GroupActivities
import AVFoundation
import Combine
import SwiftUI

struct BluefinGroupActivity: GroupActivity {
    static let activityIdentifier = "org.glopyglerky.bluefin.watch"
    /// Hash of credential-free origin. Different users on the same server authenticate separately.
    let serverIdentity: String
    let itemID: String
    var mediaKind: String? = nil
    var metadata: GroupActivityMetadata {
        var value = GroupActivityMetadata()
        value.title = mediaKind == "Audio" ? "Listen together in Bluefin" : "Watch together in Bluefin"
        value.type = mediaKind == "Audio" ? .listenTogether : .watchTogether
        return value
    }
}

private final class BluefinPlaybackIdentity: NSObject, AVPlayerPlaybackCoordinatorDelegate {
    let identifier: String
    init(identifier: String) { self.identifier = identifier }
    func playbackCoordinator(_ coordinator: AVPlayerPlaybackCoordinator, identifierFor playerItem: AVPlayerItem) -> String { identifier }
}

@MainActor final class NativeSharePlay: ObservableObject {
    @Published private(set) var pending: GroupSession<BluefinGroupActivity>?
    @Published private(set) var joined = false
    private weak var store: LibraryStore?
    private var listener: Task<Void, Never>?
    private var joinTask: Task<Void, Never>?
    private var active: GroupSession<BluefinGroupActivity>?
    private var playbackIdentity: BluefinPlaybackIdentity?
    private var playbackSubscriptions: Set<AnyCancellable> = []
    private var subscriptions: Set<AnyCancellable> = []
    private var sessionSubscriptions: Set<AnyCancellable> = []

    private func serverKey(_ session: AccountSession) throws -> String {
        guard var parts = URLComponents(string: session.server), let host = parts.host,
              parts.user == nil, parts.password == nil, parts.query == nil, parts.fragment == nil,
              ["https", "http"].contains(parts.scheme ?? "") else { throw BluefinError.message("The server identity is invalid.") }
        parts.host = host.lowercased()
        return NativeIntegrations.digest(parts.string ?? session.server)
    }

    func attach(to store: LibraryStore) {
        detach()
        self.store = store
        store.$playback.sink { [weak self] playback in
            guard let self, let session = self.active else { return }
            guard let playback, playback.item.Id == session.activity.itemID else { self.leave(); return }
            self.playbackSubscriptions.removeAll()
            let identity = BluefinPlaybackIdentity(identifier: session.activity.serverIdentity + ":" + session.activity.itemID)
            self.playbackIdentity = identity
            playback.player.playbackCoordinator.delegate = identity
            playback.player.playbackCoordinator.coordinateWithSession(session)
            // A native queue can advance inside the same controller. Never label the
            // following song as the item that the group agreed to play together.
            playback.$item.dropFirst().sink { [weak self] item in
                guard item.Id == session.activity.itemID else { self?.leave(); return }
            }.store(in: &self.playbackSubscriptions)
        }.store(in: &subscriptions)
        listener = Task { [weak self] in
            for await session in BluefinGroupActivity.sessions() {
                guard !Task.isCancelled, let self else { return }
                self.pending?.leave()
                self.pending = session
            }
        }
    }

    func start(_ item: MediaItem) async throws {
        guard let store, let account = store.session, !store.preview, !item.isContainer else { throw BluefinError.message("Sign in and choose a playable library item.") }
        let activity = BluefinGroupActivity(serverIdentity: try serverKey(account), itemID: item.Id, mediaKind: item.Type)
        switch await activity.prepareForActivation() {
        case .activationPreferred:
            guard store.session.map(NativeIntegrations.identity) == NativeIntegrations.identity(account) else { throw CancellationError() }
            guard try await activity.activate() else { throw BluefinError.message("SharePlay was not activated.") }
        case .activationDisabled: throw BluefinError.message("SharePlay is unavailable for this build or session.")
        case .cancelled: return
        @unknown default: return
        }
    }

    /// Explicit local consent, including for incoming sessions. Never accepts a remote token or stream.
    func acceptPending() {
        joinTask?.cancel()
        guard let session = pending, let store, let account = store.session, !store.preview else { return }
        joinTask = Task { [weak self] in
            guard let self else { return }
            do {
                guard session.activity.serverIdentity == (try self.serverKey(account)),
                      !session.activity.itemID.isEmpty, session.activity.itemID.count <= 512 else {
                    throw BluefinError.message("Sign in to the same library server before joining SharePlay.")
                }
                let item = try await store.detail(MediaItem(Id: session.activity.itemID))
                try Task.checkCancellation()
                guard store.session.map(NativeIntegrations.identity) == NativeIntegrations.identity(account), self.pending === session,
                      !item.isContainer else { throw CancellationError() }
                self.leave()
                self.pending = nil
                self.active = session
                session.$state.sink { [weak self] state in
                    if case .invalidated = state { self?.leave() }
                }.store(in: &self.sessionSubscriptions)
                session.$activity.dropFirst().sink { [weak self] _ in self?.leave() }.store(in: &self.sessionSubscriptions)
                session.join()
                self.joined = true
                await store.play(item)
                if store.playback?.item.Id != item.Id { self.leave() }
            } catch is CancellationError {} catch { store.error = error.localizedDescription }
        }
    }

    func leave() {
        playbackSubscriptions.removeAll()
        sessionSubscriptions.removeAll()
        if active != nil { store?.playback?.player.pause() }
        active?.leave()
        store?.playback?.player.playbackCoordinator.delegate = nil
        playbackIdentity = nil
        active = nil
        joined = false
    }
    func accountChanged() {
        joinTask?.cancel()
        leave()
        pending?.leave()
        pending = nil
    }
    func detach() {
        listener?.cancel(); listener = nil
        accountChanged()
        subscriptions.removeAll()
        store = nil
    }
}

@MainActor struct NativeSharePlayControls: View {
    @ObservedObject private var sharing = NativeIntegrations.shared.sharing
    var body: some View {
        if sharing.pending != nil { Button("Join SharePlay with this library account") { sharing.acceptPending() } }
        if sharing.joined { Button("Leave SharePlay") { sharing.leave() } }
    }
}
#endif
