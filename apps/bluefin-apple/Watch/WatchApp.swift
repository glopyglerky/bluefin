import SwiftUI
import AVFoundation
import WatchKit
import MediaPlayer

private struct WatchBrowseKey: Equatable { let section: String; let search: String }
private struct WatchBrowseRequest { let key: WatchBrowseKey; let generation: UUID; let more: Bool }

@MainActor final class WatchStore: ObservableObject {
    @Published var account = WatchKeychain.load()
    @Published var tracks: [WatchTrack] = []
    @Published var current: WatchTrack?
    @Published var playing = false
    @Published var busy = false
    @Published var problem: String?
    @Published var hasMore = false
    private var revokePending = false
    private var ownershipUnavailable = false
    private var offset = 0
    private var accountEpoch = UUID()
    var accountContext: UUID { accountEpoch }
    @Published private var pendingAccount: WatchAccount? = WatchKeychain.load(slot: "pending-companion")
    @Published private var pendingChallenge = WatchKeychain.load(slot: "pending-challenge")
    var needsSetupCleanup: Bool { ownershipUnavailable || revokePending || pendingAccount != nil || pendingChallenge != nil }
    var visibleAccount: WatchAccount? { needsSetupCleanup ? nil : account }
    private var browseKey: WatchBrowseKey?
    private var browseGeneration = UUID()
    private var pendingBrowse: WatchBrowseRequest?
    private let player = AVPlayer()
    private let leases = WatchLeaseLifecycle()
    private var playbackGeneration = UUID()
    private var ended: NSObjectProtocol?
    private var failed: NSObjectProtocol?
    private var interrupted: NSObjectProtocol?
    private var commands: [(MPRemoteCommand, Any)] = []

    init() {
        do {
            account = try WatchKeychain.loadChecked()
            pendingAccount = try WatchKeychain.loadChecked(slot: "pending-companion")
            pendingChallenge = try WatchKeychain.loadChecked(slot: "pending-challenge")
        } catch { ownershipUnavailable = true; problem = "Saved ownership is unavailable. Unlock and retry sign-out." }
        if needsSetupCleanup { account = nil }
        WatchIntentAccess.store = self
        WatchCompanion.shared.changed = { [weak self] in Task { @MainActor in await self?.reconcileCompanion() } }
        WatchCompanion.shared.start()
        let center = MPRemoteCommandCenter.shared()
        for command in [center.playCommand, center.pauseCommand, center.togglePlayPauseCommand] {
            let target = command.addTarget { [weak self] event in
                Task { @MainActor in
                    guard let self, self.current != nil, !self.busy, !self.needsSetupCleanup, self.account != nil else { return }
                    if event.command === center.playCommand { self.player.play(); self.playing = true }
                    else if event.command === center.pauseCommand { self.player.pause(); self.playing = false }
                    else { self.toggle() }
                }
                return .success
            }
            commands.append((command, target))
        }
        for (command, direction) in [(center.nextTrackCommand, 1), (center.previousTrackCommand, -1)] {
            let target = command.addTarget { [weak self] _ in
                Task { @MainActor in await self?.skip(direction) }; return .success
            }
            commands.append((command, target))
        }
        let seekTarget = center.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let event = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            Task { @MainActor in self?.seek(event.positionTime) }; return .success
        }
        commands.append((center.changePlaybackPositionCommand, seekTarget))
        interrupted = NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.player.pause(); self?.playing = false }
        }

    }
    func reconcileCompanion() async {
        if let saved = account?.companionGeneration ?? pendingAccount?.companionGeneration ?? UserDefaults.standard.string(forKey: "watch.companionGeneration"),
           let current = WatchCompanion.shared.remoteGeneration, saved != current {
            accountEpoch = UUID(); revokePending = true
            fenceCompanionPlayback()
        }
        if revokePending {
            guard !busy else { return }
            await signOut()
            if account == nil && pendingAccount == nil && pendingChallenge == nil { revokePending = false }
            return
        }
        if account == nil { await pairedSignIn() }
    }
    private func fenceCompanionPlayback() {
        player.pause(); playing = false
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
        tracks = []; hasMore = false; offset = 0
        pendingBrowse = nil; browseGeneration = UUID(); browseKey = nil
    }
    func pairedSignIn() async {
        guard account == nil else { return }
        await run {
            guard !needsSetupCleanup else { throw WatchFailure.message("Retry sign-out to clean up the previous setup.") }
            let epoch = accountEpoch; let nonce = UUID().uuidString
            let bridge = WatchCompanion.shared
            let bootstrap = try await bridge.ask(["action": "bootstrap", "nonce": nonce])
            guard bootstrap["nonce"] as? String == nonce, let address = bootstrap["server"] as? String,
                  let user = bootstrap["user"] as? String, let generation = bootstrap["generation"] as? String else { throw WatchFailure.message("Open Bluefin on your paired iPhone and sign in.") }
            let server = try WatchAPI.server(address)
            guard try await ManagedPairing.supported(server) else {
                throw WatchFailure.message("This server does not support managed Watch pairing. Use independent sign-in.")
            }
            guard accountEpoch == epoch, !Task.isCancelled else { throw CancellationError() }
            let start = try JSONDecoder().decode(ManagedPairingStart.self, from: await WatchAPI.request(server, "Bluefin/Pairing/Initiate", method: "POST"))
            try start.proof.validate()
            // Keep the returned authority even if validation or the Keychain write fails.
            let recovery = WatchAccount(server: server, token: "", user: user, name: generation, managedProof: start.proof)
            pendingChallenge = recovery
            try WatchKeychain.save(recovery, slot: "pending-challenge")
            guard start.device_id == WatchAPI.deviceID,
                  start.expires_at > Int64(Date().timeIntervalSince1970),
                  accountEpoch == epoch, !Task.isCancelled else { throw CancellationError() }
            let reply = try await bridge.ask(["action": "authorizeManaged", "nonce": nonce, "generation": generation,
                                              "challenge": start.proof.challenge, "secret": start.proof.secret, "device": start.device_id])
            guard reply["nonce"] as? String == nonce, reply["generation"] as? String == generation,
                  reply["authorized"] as? Bool == true, accountEpoch == epoch, !Task.isCancelled,
                  start.expires_at > Int64(Date().timeIntervalSince1970) else { throw CancellationError() }
            var a = try await ManagedPairing.exchange(server, proof: start.proof)
            a.companionGeneration = generation
            pendingAccount = a
            do {
                try WatchKeychain.save(a, slot: "pending-companion")
                let verify = try await bridge.ask(["action": "bootstrap", "nonce": UUID().uuidString])
                guard accountEpoch == epoch, account == nil, !Task.isCancelled,
                      verify["generation"] as? String == generation, verify["server"] as? String == address,
                      verify["user"] as? String == user,
                      a.user.replacingOccurrences(of: "-", with: "").lowercased() == user.replacingOccurrences(of: "-", with: "").lowercased() else { throw CancellationError() }
                // Published sessions keep their sensitive cancellation authority indefinitely.
                try WatchKeychain.save(a)
                UserDefaults.standard.set(generation, forKey: "watch.companionGeneration")
                try WatchKeychain.clear(slot: "pending-challenge"); pendingChallenge = nil
                try WatchKeychain.clear(slot: "pending-companion"); pendingAccount = nil; account = a
            } catch {
                // Both durable pending slots retain the proof; sign-out owns lease drain and cancellation.
                revokePending = true
                throw error
            }
        }
    }
    func seek(_ seconds: Double) {
        guard !busy, !revokePending, current != nil, seconds.isFinite, seconds >= 0 else { return }
        player.seek(to: CMTime(seconds: seconds, preferredTimescale: 600))
    }
    private func observe(_ item: AVPlayerItem, generation: UUID) {
        if let ended { NotificationCenter.default.removeObserver(ended) }
        if let failed { NotificationCenter.default.removeObserver(failed) }
        ended = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self, weak item] _ in
            Task { @MainActor in
                guard let self, let item, self.playbackGeneration == generation,
                      item === self.player.currentItem else { return }
                await self.stop()
            }
        }
        failed = NotificationCenter.default.addObserver(forName: .AVPlayerItemFailedToPlayToEndTime, object: item, queue: .main) { [weak self, weak item] _ in
            Task { @MainActor in
                guard let self, let item, self.playbackGeneration == generation,
                      item === self.player.currentItem else { return }
                self.problem = "Playback interrupted. Reconnect and try again."
                await self.stop()
            }
        }
    }
    deinit {
        if let ended { NotificationCenter.default.removeObserver(ended) }
        if let failed { NotificationCenter.default.removeObserver(failed) }
        if let interrupted { NotificationCenter.default.removeObserver(interrupted) }
        for (command, target) in commands { command.removeTarget(target) }
    }
    func run(_ action: () async throws -> Void) async {
        guard !busy else { return }
        busy = true; problem = nil
        defer {
            busy = false
            if revokePending { Task { await reconcileCompanion() } }
            else if pendingBrowse != nil { Task { await drainBrowse() } }
        }
        do { try await action() } catch { problem = (error as? WatchFailure)?.errorDescription ?? "The request could not complete. Check the server, account and audio route." }
    }
    func connect(server: String, name: String, password: String, create: Bool = false) async {
        await run {
            guard !needsSetupCleanup, account == nil else { throw WatchFailure.message("Finish signing out before connecting again.") }
            if create { try await WatchAPI.createAccount(WatchAPI.server(server), name: name, password: password) }
            let a = try await WatchAPI.signIn(server: server, username: name, password: password)
            try WatchKeychain.save(a); account = a
        }
    }
    func finishIndependentSignIn(_ request: WatchAPI.DeviceRequest) async {
        await run {
            guard !needsSetupCleanup, account == nil else { throw WatchFailure.message("Finish signing out before connecting again.") }
            guard let signedIn = try await WatchAPI.finishSignIn(request) else {
                throw WatchFailure.message("Finish approving the request first.")
            }
            do {
                try WatchKeychain.save(signedIn)
                account = signedIn
            } catch {
                try await WatchAPI.cancelSignIn(request)
                throw error
            }
        }
    }

    func beginBrowse(_ section: String, search: String) {
        let key = WatchBrowseKey(section: section, search: search)
        if browseKey != key {
            browseKey = key; browseGeneration = UUID(); pendingBrowse = nil
            tracks = []; offset = 0; hasMore = false
        }
    }
    func load(_ section: String, search: String = "", more: Bool = false) async {
        guard account != nil, !needsSetupCleanup else { return }
        let sameQuery = browseKey == WatchBrowseKey(section: section, search: search)
        beginBrowse(section, search: search)
        guard let key = browseKey else { return }
        pendingBrowse = WatchBrowseRequest(key: key, generation: browseGeneration, more: more && sameQuery)
        await drainBrowse()
    }
    private func drainBrowse() async {
        guard !busy, !needsSetupCleanup, let request = pendingBrowse else { return }
        pendingBrowse = nil
        await run {
            guard let a = account, !needsSetupCleanup, browseGeneration == request.generation else { return }
            let epoch = accountEpoch
            if request.key.section == "Queue" {
                let values = try await WatchAPI.queue(a)
                guard accountEpoch == epoch, account == a, !needsSetupCleanup, browseGeneration == request.generation else { return }
                tracks = values; hasMore = false
            } else {
                let page = try await WatchAPI.page(a, favorites: request.key.section == "Favorites", search: request.key.search, start: request.more ? offset : 0)
                guard accountEpoch == epoch, account == a, !needsSetupCleanup, browseGeneration == request.generation else { return }
                offset = (request.more ? offset : 0) + page.Items.count
                tracks = request.more ? tracks + page.Items.filter { item in !tracks.contains { $0.id == item.id } } : page.Items
                hasMore = page.TotalRecordCount.map { offset < $0 } ?? (page.Items.count == 50)
            }
        }
    }
    func play(_ track: WatchTrack) async {
        await run {
            guard !revokePending, track.Type == "Audio", let a = account else { return }
            let epoch = accountEpoch
            await stop()
            guard leases.owner == nil && !leases.isCleaning else { throw WatchFailure.message("Previous playback cleanup must succeed first.") }
            let raw = try await WatchAPI.request(a.server, "Bluefin/TV/v1/leases", account: a, method: "POST", body: ["version": 1, "itemId": track.Id.replacingOccurrences(of: "-", with: ""), "playback": ["kind": "audio", "maxBitrate": 320000]])
            let lease = try JSONDecoder().decode(WatchLease.self, from: raw)
            guard lease.version == 1 else { throw WatchFailure.message("Unsupported lease version.") }
            _ = try WatchAPI.capabilityURL(lease.releaseUrl, account: a, prefix: "/Bluefin/TV/v1/leases/")
            try leases.install(account: a, releasePath: String(lease.releaseUrl.dropFirst()))
            do {
                guard accountEpoch == epoch, account == a, !revokePending else { throw CancellationError() }
                let url = try WatchAPI.capabilityURL(lease.mediaUrl, account: a, prefix: "/Bluefin/TV/v1/media/")
                let audio = AVAudioSession.sharedInstance()
                try audio.setCategory(.playback, mode: .default, policy: .longFormAudio)
                try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
                    audio.activate(options: []) { success, error in
                        if let error { c.resume(throwing: error) }
                        else if success { c.resume() }
                        else { c.resume(throwing: WatchFailure.message("Connect an audio output.")) }
                    }
                }
                guard accountEpoch == epoch, account == a, !revokePending else { throw CancellationError() }
                let item = AVPlayerItem(url: url)
                let generation = UUID(); playbackGeneration = generation
                observe(item, generation: generation)
                player.replaceCurrentItem(with: item)
                current = track
                MPNowPlayingInfoCenter.default().nowPlayingInfo = [MPMediaItemPropertyTitle: track.Name]
                player.play(); playing = true
            } catch { await stop(); throw error }
        }
    }
    func toggle() {
        guard !busy, !revokePending, current != nil, player.currentItem != nil else { return }
        if playing { player.pause() } else { player.play() }; playing.toggle()
    }
    func skip(_ direction: Int) async {
        guard let current, let i = tracks.firstIndex(where: { $0.id == current.id }), tracks.indices.contains(i + direction) else { return }
        await play(tracks[i + direction])
    }
    func favorite() async {
        await run {
            guard !needsSetupCleanup, let a = account, let track = current else { return }
            let epoch = accountEpoch
            let value = track.UserData?.IsFavorite != true
            _ = try await WatchAPI.request(a.server, "Users/\(a.user)/FavoriteItems/\(track.Id)", account: a, method: value ? "POST" : "DELETE")
            guard account == a, accountEpoch == epoch, !needsSetupCleanup else { return }
            for index in tracks.indices where tracks[index].id == track.id {
                tracks[index].UserData = .init(IsFavorite: value)
            }
            if current?.id == track.id { current?.UserData = .init(IsFavorite: value) }
        }
    }
    func stop() async {
        if !leases.isCleaning {
            playbackGeneration = UUID()
            player.pause(); player.replaceCurrentItem(with: nil); playing = false; current = nil
            MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
            if let ended { NotificationCenter.default.removeObserver(ended); self.ended = nil }
            if let failed { NotificationCenter.default.removeObserver(failed); self.failed = nil }
        }
        let generation = playbackGeneration
        do {
            try await leases.drain(release: { owner in
                _ = try await WatchAPI.request(owner.account.server, owner.releasePath, account: owner.account, method: "DELETE")
            }, finished: { [weak self] in
                guard let self, self.playbackGeneration == generation,
                      self.current == nil && self.player.currentItem == nil else { return }
                try? AVAudioSession.sharedInstance().setActive(false)
            })
        } catch { problem = "Playback stopped. Server cleanup failed; try Stop again." }
    }
    private func cleanupPending(_ a: WatchAccount) async throws {
        if let proof = a.managedProof {
            try await ManagedPairing.cancel(a.server, proof: proof)
        } else {
            do { _ = try await WatchAPI.request(a.server, "Sessions/Logout", account: a, method: "POST") }
            catch WatchFailure.unauthorized { /* Legacy token already rejected. This says nothing about a challenge. */ }
        }
        if try WatchKeychain.loadChecked() == a { try WatchKeychain.clear() }
        try WatchKeychain.clear(slot: "pending-companion")
        pendingAccount = nil
    }
    func signOut() async {
        guard !busy else { return }
        busy = true; defer { busy = false }
        if ownershipUnavailable {
            do {
                account = try WatchKeychain.loadChecked()
                pendingAccount = try WatchKeychain.loadChecked(slot: "pending-companion")
                pendingChallenge = try WatchKeychain.loadChecked(slot: "pending-challenge")
                ownershipUnavailable = false
            } catch { problem = "Saved ownership remains unavailable. Unlock and retry."; return }
        }
        accountEpoch = UUID(); revokePending = true
        fenceCompanionPlayback()
        await stop()
        guard leases.owner == nil && !leases.isCleaning else { return }
        if let a = account {
            do {
                guard pendingAccount == nil || pendingAccount == a else { throw WatchFailure.message("Finish the previous sign-out first.") }
                try WatchKeychain.save(a, slot: "pending-companion"); pendingAccount = a
                try WatchKeychain.clear(); account = nil
            } catch { problem = "Could not preserve sign-out recovery. Retry signing out."; return }
        }
        if let a = pendingAccount {
            do { try await cleanupPending(a) }
            catch { problem = "Setup cleanup failed. Retry sign-out."; return }
        }
        if let challenge = pendingChallenge {
            do {
                if let proof = challenge.managedProof {
                    try await ManagedPairing.cancel(challenge.server, proof: proof)
                } else {
                    let a = try await WatchAPI.exchange(challenge.server, secret: challenge.token)
                    try WatchKeychain.save(a, slot: "pending-companion"); pendingAccount = a
                    try await cleanupPending(a)
                }
                try WatchKeychain.clear(slot: "pending-challenge"); pendingChallenge = nil
            } catch { problem = "Setup is still awaiting the server. Retry signing out, or revoke the Watch session in server settings."; return }
        }
        account = nil; tracks = []
        revokePending = false
        UserDefaults.standard.removeObject(forKey: "watch.companionGeneration")
    }
}

private enum WatchAppearance {
    static let background = Color(red: 6.0 / 255, green: 23.0 / 255, blue: 39.0 / 255)
    static let text = Color(red: 204.0 / 255, green: 208.0 / 255, blue: 211.0 / 255)
    static let accent = Color(red: 173.0 / 255, green: 181.0 / 255, blue: 209.0 / 255)
}

@main struct BluefinWatchApp: App {
    @StateObject private var store = WatchStore()
    var body: some Scene {
        WindowGroup {
            WatchRoot().environmentObject(store)
                .tint(WatchAppearance.accent)
                .foregroundStyle(WatchAppearance.text)
                .containerBackground(WatchAppearance.background, for: .navigation)
        }
    }
}
struct WatchRoot: View {
    @EnvironmentObject private var store: WatchStore
    @State private var server = ""
    @State private var name = ""
    @State private var password = ""
    @State private var connection: URL?
    @State private var serverName = ""
    @State private var options: WatchAPI.AccountOptions?
    @State private var device: WatchAPI.DeviceRequest?
    @State private var creating: Bool?
    @State private var connecting = false
    @State private var setupError: String?

    private func checkAddress() async {
        connecting = true; setupError = nil
        defer { connecting = false }
        do {
            let address = server.trimmingCharacters(in: .whitespacesAndNewlines)
            let url = try WatchAPI.server(address.contains("://") ? address : "https://" + address)
            let result = try await WatchAPI.accountOptions(url)
            connection = url; serverName = result.0; options = result.1
        } catch { setupError = error.localizedDescription }
    }

    private func begin(_ server: URL, passkey: Bool) async {
        connecting = true; setupError = nil
        defer { connecting = false }
        do { device = try await WatchAPI.beginSignIn(server, passkey: passkey) }
        catch { setupError = error.localizedDescription }
    }

    var body: some View {
        NavigationStack {
            List {
                if let a = store.visibleAccount {
                    if let track = store.current {
                        Section("Now playing") {
                            Text(track.Name).font(.headline)
                            HStack {
                                Button { Task { await store.skip(-1) } } label: { Image(systemName: "backward.end.fill") }.accessibilityLabel("Previous track")
                                Button { store.toggle() } label: { Image(systemName: store.playing ? "pause.fill" : "play.fill") }.accessibilityLabel(store.playing ? "Pause" : "Play")
                                Button { Task { await store.skip(1) } } label: { Image(systemName: "forward.end.fill") }.accessibilityLabel("Next track")
                            }
                            Button(track.UserData?.IsFavorite == true ? "Remove favorite" : "Favorite") { Task { await store.favorite() } }
                        }
                    }
                    ForEach(["Music", "Queue", "Favorites"], id: \.self) { section in
                        NavigationLink(section) {
                            WatchTrackList(section: section)
                        }
                    }
                    NavigationLink("Account") {
                        List {
                            Text(a.name)
                            Text("Playback runs on this Watch. Your server controls access.")
                            Button("Sign out", role: .destructive) { Task { await store.signOut() } }.disabled(store.busy)
                        }
                    }
                    Button("Stop playback") { Task { await store.stop() } }.disabled(store.busy)
                    NowPlayingView()
                } else {
                    Button("Use paired iPhone") { Task { await store.pairedSignIn() } }.disabled(store.busy)
                    if store.needsSetupCleanup { Button("Finish signing out") { Task { await store.signOut() } }.disabled(store.busy) }
                    if let connection {
                        Text(serverName).font(.headline)
                        if let device {
                            Text(device.code).monospaced()
                            if let image = device.qr {
                                Text("Scan with your phone.")
                                AsyncImage(url: image) { value in value.resizable().interpolation(.none) }
                                    placeholder: { ProgressView() }
                                    .scaledToFit().frame(width: 120, height: 120)
                            } else { Text("Enter this code in a signed-in client.") }
                            Button("Finish sign-in") { Task { await store.finishIndependentSignIn(device) } }.disabled(store.busy)
                            Button("Cancel") {
                                Task {
                                    do { try await WatchAPI.cancelSignIn(device); self.device = nil }
                                    catch { setupError = error.localizedDescription }
                                }
                            }.disabled(store.busy)
                        } else if let creating {
                            TextField("Username", text: $name).textInputAutocapitalization(.never)
                            SecureField("Password", text: $password)
                            Button(creating ? "Create account" : "Sign in") {
                                let submitted = password
                                password = ""
                                Task {
                                    await store.connect(server: connection.absoluteString, name: name, password: submitted, create: creating)
                                    if creating && store.visibleAccount == nil {
                                        self.creating = nil
                                        await checkAddress()
                                    }
                                }
                            }.disabled(store.busy || name.isEmpty || (creating && password.count < 15))
                            Button("Back") { self.creating = nil; password = "" }
                        } else {
                            Button("Sign in") { creating = false }
                            if options?.passkeys == true {
                                Button("Sign in with a passkey") { Task { await begin(connection, passkey: true) } }
                                    .disabled(connecting)
                            }
                            if options?.quickConnect == true {
                                Button("Quick Connect") { Task { await begin(connection, passkey: false) } }
                                    .disabled(connecting)
                            }
                            if options?.allowSignups == true { Button("Create account") { creating = true } }
                            Button("Change server") { self.connection = nil; setupError = nil }
                        }
                    } else {
                        TextField("HTTPS server", text: $server).textInputAutocapitalization(.never).autocorrectionDisabled()
                        Button("Continue") { Task { await checkAddress() } }.disabled(connecting || server.isEmpty)
                    }
                    if connecting { ProgressView() }
                    if let setupError { Text(setupError).font(.footnote) }
                }
                NavigationLink("Software licenses", destination: SoftwareLicensesView())
                if store.busy { ProgressView() }
                if let problem = store.problem { Text(problem).font(.footnote) }
            }.scrollContentBackground(.hidden)
                .background(WatchAppearance.background)
                .navigationTitle("Bluefin")
        }
    }
}

struct WatchTrackList: View {
    let section: String
    @EnvironmentObject private var store: WatchStore
    @State private var search = ""
    var body: some View {
        List {
            ForEach(store.tracks) { track in
                Button { Task { await store.play(track) } } label: {
                    VStack(alignment: .leading) { Text(track.Name); if let artist = track.Artists?.joined(separator: ", ") { Text(artist).font(.caption).foregroundStyle(.secondary) } }
                }.disabled(store.busy)
            }
            if store.hasMore { Button("Load more") { Task { await store.load(section, search: search, more: true) } }.disabled(store.busy) }
            if !store.busy && store.tracks.isEmpty { Text("No music found") }
        }.scrollContentBackground(.hidden)
            .background(WatchAppearance.background)
            .navigationTitle(section).searchable(text: $search)
        .task(id: search) {
            store.beginBrowse(section, search: search)
            do { try await Task.sleep(nanoseconds: 350_000_000); try Task.checkCancellation(); await store.load(section, search: search) } catch {}
        }
    }
}
