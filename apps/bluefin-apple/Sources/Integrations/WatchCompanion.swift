#if (os(iOS) && !targetEnvironment(macCatalyst)) || os(watchOS)
import Foundation
import WatchConnectivity

struct WatchCompanionPermit {
    let generation: UUID
    let pairing: UUID
    let expires: Date
    func accepts(generation: UUID, pairing: UUID, now: Date) -> Bool {
        self.generation == generation && self.pairing == pairing && now < expires
    }
}
// Only interactive messages carry the short-lived Watch challenge. Context contains no credentials.
@MainActor final class WatchCompanion: NSObject, WCSessionDelegate {
    static let shared = WatchCompanion()
    private var session: WCSession?
    private var pending: [UUID: CheckedContinuation<[String: Any], Error>] = [:]
    private var pairingEpoch = UUID()
    var remoteGeneration: String? { session?.receivedApplicationContext["generation"] as? String }
    var changed: (() -> Void)?
    #if os(iOS)
    private var grants: [String: WatchCompanionPermit] = [:]
    private let owners = PhonePairingLedger(read: PhonePairingVault.load, write: PhonePairingVault.save)
    private var changingAccount = false
    private var retrying = false
    func beginAccountChange() async throws {
        guard !changingAccount else { throw CancellationError() }
        changingAccount = true; pairingEpoch = UUID(); grants.removeAll()
        do { try await owners.drain(keeping: nil) { try await ManagedPairing.cancel($0.server, proof: $0.proof) } }
        catch { changingAccount = false; throw error }
    }
    func endAccountChange() { changingAccount = false; publish() }
    private func savedAccount() throws -> AccountSession? {
        guard !changingAccount else { throw CancellationError() }
        var saved = try SessionVault.loadChecked()
        if var legacy = saved, legacy.companionGeneration == nil {
            // Migration must persist before any grant or signed-in context escapes.
            guard NativeIntegrations.shared.store == nil else { throw CancellationError() }
            legacy.companionGeneration = UUID(); try SessionVault.save(legacy); saved = legacy
        }
        if let store = NativeIntegrations.shared.store {
            guard !store.preview, store.session?.token == saved?.token,
                  store.session?.server == saved?.server, store.session?.userId == saved?.userId,
                  store.companionContext == saved?.companionGeneration else { throw CancellationError() }
        }
        return saved
    }
    private func retryOwners() async {
        guard !retrying, !changingAccount else { return }
        retrying = true; defer { retrying = false }
        do {
            let generation = try savedAccount()?.companionGeneration
            try await owners.drain(keeping: generation) { try await ManagedPairing.cancel($0.server, proof: $0.proof) }
        } catch { /* Durable rows remain for the next activation or explicit account change. */ }
    }
    #endif
    func start() {
        guard session == nil, WCSession.isSupported() else { return }
        session = .default; session?.delegate = self; session?.activate()
    }
    func ask(_ message: [String: Any]) async throws -> [String: Any] {
        guard let session, session.activationState == .activated, session.isReachable else {
            throw WatchFailure.message("Your iPhone is unreachable. Open Bluefin on your paired iPhone or use independent sign-in.")
        }
        let id = UUID()
        return try await withTaskCancellationHandler(operation: {
            try await withCheckedThrowingContinuation { continuation in
                pending[id] = continuation
                session.sendMessage(message, replyHandler: { reply in
                    Task { @MainActor in self.finish(id, .success(reply)) }
                }, errorHandler: { _ in
                    Task { @MainActor in self.finish(id, .failure(WatchFailure.message("The paired iPhone did not reply. Try again."))) }
                })
                Task { try? await Task.sleep(nanoseconds: 12_000_000_000); self.finish(id, .failure(WatchFailure.message("The paired iPhone timed out. Try again."))) }
            }
        }, onCancel: { Task { @MainActor in self.finish(id, .failure(CancellationError())) } })
    }
    private func finish(_ id: UUID, _ result: Result<[String: Any], Error>) {
        pending.removeValue(forKey: id)?.resume(with: result)
    }
    func publish() {
        #if os(iOS)
        grants.removeAll()
        guard let session, session.activationState == .activated else { return }
        do {
            let saved = try savedAccount()
            let generation = saved?.companionGeneration?.uuidString ?? "signed-out"
            try session.updateApplicationContext(["generation": generation])
            Task { await retryOwners() }
        } catch { /* Unknown Keychain state must never publish signed-out. */ }
        #endif
    }
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: Error?) {
        Task { @MainActor in self.publish(); self.changed?() }
    }
    nonisolated func session(_ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]) {
        Task { @MainActor in self.changed?() }
    }
    nonisolated func session(_ session: WCSession, didReceiveMessage message: [String: Any], replyHandler: @escaping ([String: Any]) -> Void) {
        Task { @MainActor in
            #if os(iOS)
            do { replyHandler(try await self.respond(message)) }
            catch { replyHandler(["error": "Open Bluefin on your iPhone and check sign-in, HTTPS and Quick Connect support."]) }
            #else
            replyHandler(["error": "Unsupported request"])
            #endif
        }
    }
    #if os(iOS)
    nonisolated func sessionDidBecomeInactive(_ session: WCSession) { Task { @MainActor in self.pairingEpoch = UUID(); self.grants.removeAll() } }
    nonisolated func sessionDidDeactivate(_ session: WCSession) { Task { @MainActor in self.pairingEpoch = UUID(); self.grants.removeAll(); session.activate() } }
    private func respond(_ m: [String: Any]) async throws -> [String: Any] {
        guard m.count <= 6, let nonce = m["nonce"] as? String, UUID(uuidString: nonce) != nil,
              let a = try savedAccount(), let companion = a.companionGeneration else { throw CancellationError() }
        let epoch = pairingEpoch
        let attached = NativeIntegrations.shared.store
        let process = attached?.accountContext
        let server = try WatchAPI.server(a.server)
        func check() throws {
            let current = try savedAccount()
            guard current?.token == a.token, current?.server == a.server, current?.userId == a.userId,
                  current?.companionGeneration == companion, pairingEpoch == epoch,
                  attached == nil || (NativeIntegrations.shared.store === attached && attached?.accountContext == process) else { throw CancellationError() }
        }
        if m["action"] as? String == "bootstrap" {
            grants = grants.filter { $0.value.expires > Date() }
            guard grants.count < 8 else { throw CancellationError() }
            grants[nonce] = WatchCompanionPermit(generation: companion, pairing: epoch, expires: Date().addingTimeInterval(60))
            return ["nonce": nonce, "server": server.absoluteString, "user": a.userId, "generation": companion.uuidString]
        }
        guard m["action"] as? String == "authorizeManaged", let grant = grants.removeValue(forKey: nonce),
              grant.accepts(generation: companion, pairing: epoch, now: Date()),
              m["generation"] as? String == companion.uuidString,
              let challenge = m["challenge"] as? String, let secret = m["secret"] as? String,
              let device = m["device"] as? String, UUID(uuidString: device) != nil else { throw CancellationError() }
        let proof = ManagedPairingProof(challenge: challenge, secret: secret)
        try proof.validate()
        guard try await ManagedPairing.supported(server) else { throw CancellationError() }
        try check()
        try await owners.drain(keeping: companion) { try await ManagedPairing.cancel($0.server, proof: $0.proof) }
        try check()
        // This durable write precedes any request capable of minting a Watch session.
        try owners.remember(PhonePairingOwner(id: UUID(), server: server, generation: companion, proof: proof, device: device))
        let phone = WatchAccount(server: server, token: a.token, user: a.userId, name: a.username)
        _ = try await WatchAPI.request(server, "Bluefin/Pairing/Authorize", account: phone, method: "POST", body: ["proof": proof.body, "device_id": device])
        try check()
        guard grant.accepts(generation: companion, pairing: epoch, now: Date()) else { throw CancellationError() }
        return ["nonce": nonce, "generation": companion.uuidString, "authorized": true]
    }
    #endif
}
struct WatchChallenge: Decodable {
    var Code: String; var Secret: String; var Authenticated: Bool
    var AppName: String; var DeviceName: String
}
#endif
