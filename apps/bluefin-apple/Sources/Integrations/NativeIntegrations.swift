import Foundation
import Combine
import CryptoKit
#if os(iOS)
import CoreSpotlight
import UniformTypeIdentifiers
#endif

@MainActor final class NativeIntegrations {
    static let shared = NativeIntegrations()
    weak var store: LibraryStore?
    /// Root sets this to select its library tab. Opening the app alone cannot select a tab.
    var openLibrary: (() -> Void)?
    private var subscriptions: Set<AnyCancellable> = []
    private var accountKey: String?
    #if os(iOS)
    private var indexWork: Task<Void, Never>?
    #endif
    #if BLUEFIN_SHAREPLAY
    let sharing = NativeSharePlay()
    #endif

    static func attach(to store: LibraryStore) { shared.attach(store) }
    private func attach(_ store: LibraryStore) {
        guard self.store !== store else { return }
        detach()
        self.store = store
        store.$session.sink { [weak self] session in self?.accountChanged(session) }.store(in: &subscriptions)
        #if BLUEFIN_SHAREPLAY
        sharing.attach(to: store)
        #endif
    }

    func detach() {
        subscriptions.removeAll()
        #if os(iOS)
        enqueueIndexDeletion()
        #endif
        #if BLUEFIN_SHAREPLAY
        sharing.detach()
        #endif
        store = nil
        accountKey = nil
    }

    static func digest(_ value: String) -> String { SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined() }
    static func identity(_ session: AccountSession) -> String { digest(session.server + "\u{0}" + session.userId) }

    private func accountChanged(_ session: AccountSession?) {
        let next = session.map(Self.identity)
        guard next != accountKey else { return }
        accountKey = next
        #if os(iOS)
        // Serialized native index operations: deletion precedes subsequent donations.
        enqueueIndexDeletion()
        #endif
        #if BLUEFIN_SHAREPLAY
        sharing.accountChanged()
        #endif
    }

    func requireStore() throws -> LibraryStore {
        guard let store, store.session != nil, !store.preview else { throw BluefinError.message("Open Bluefin and sign in first.") }
        return store
    }

    func playNamed(_ name: String) async throws {
        let store = try requireStore()
        guard let session = store.session else { return }
        let key = Self.identity(session)
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, name.count <= 200 else { throw BluefinError.message("Enter a library title of up to 200 characters.") }
        let page = try await store.items(query: name)
        try Task.checkCancellation()
        guard store.session.map(Self.identity) == key else { throw CancellationError() }
        let matches = page.Items.filter { !$0.isContainer && $0.title.compare(name, options: [.caseInsensitive, .diacriticInsensitive]) == .orderedSame }
        guard matches.count == 1, let total = page.TotalRecordCount, total >= 1, total <= page.Items.count else {
            throw BluefinError.message("No unique exact title was found. Choose the item in Bluefin.")
        }
        await store.play(matches[0])
        guard store.playback?.item.Id == matches[0].Id else { throw BluefinError.message(store.error ?? "Playback could not start.") }
    }

    #if os(iOS)
    private func enqueueIndexDeletion() {
        let previous = indexWork
        indexWork = Task {
            await previous?.value
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                CSSearchableIndex.default().deleteAllSearchableItems { _ in continuation.resume() }
            }
        }
    }

    /// Opt-in only: call after explicit user consent to expose this account's titles in device search.
    func donateToSpotlight(_ items: [MediaItem]) throws {
        let store = try requireStore()
        guard let session = store.session else { return }
        let domain = "bluefin." + Self.identity(session)
        let entries = items.prefix(100).filter { !$0.isContainer }.map { item -> CSSearchableItem in
            let attributes = CSSearchableItemAttributeSet(contentType: .content)
            attributes.title = item.title
            let encoded = Data(item.Id.utf8).base64EncodedString()
            let entry = CSSearchableItem(uniqueIdentifier: domain + ":" + encoded, domainIdentifier: domain, attributeSet: attributes)
            entry.expirationDate = Date().addingTimeInterval(7 * 86400)
            return entry
        }
        let previous = indexWork
        let key = accountKey
        indexWork = Task { [weak self] in
            await previous?.value
            guard let self, self.accountKey == key else { return }
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                CSSearchableIndex.default().indexSearchableItems(entries) { _ in continuation.resume() }
            }
        }
    }

    /// Root routes CSSearchableItemActionType here. Never plays an item from another account.
    func continueSpotlight(_ activity: NSUserActivity) async throws {
        guard activity.activityType == CSSearchableItemActionType,
              let identifier = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String else { return }
        let store = try requireStore()
        guard let session = store.session else { return }
        let prefix = "bluefin." + Self.identity(session) + ":"
        guard identifier.hasPrefix(prefix), let data = Data(base64Encoded: String(identifier.dropFirst(prefix.count))),
              let id = String(data: data, encoding: .utf8), !id.isEmpty, id.count <= 512 else { return }
        // Fetch current permissions and metadata through the authenticated server before playback.
        let key = Self.identity(session)
        let item = try await store.detail(MediaItem(Id: id))
        try Task.checkCancellation()
        guard store.session.map(Self.identity) == key else { throw CancellationError() }
        await store.play(item)
    }
    #endif

    #if BLUEFIN_SHAREPLAY
    func startSharing(_ item: MediaItem) async throws { try await sharing.start(item) }
    #endif
}
