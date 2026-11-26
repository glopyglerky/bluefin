import SwiftUI
import AuthenticationServices
import CoreImage
import UniformTypeIdentifiers
#if canImport(UIKit)
import UIKit
#endif

/// Presentation state only. Rust validates the invitation, target, passkey
/// continuation, and device proof. Every network effect has saved retry data.
@MainActor final class MeshPairingModel: ObservableObject {
    // All windows and invitation entry points share the one protected pending
    // slot. A dismissed view cannot release ownership while its FFI is running.
    static let shared = MeshPairingModel { text in
        let preview: MeshInvitePreview = try await RustBridge.call("meshInviteDecode", fields: ["code": text])
        return preview
    }
    private let decodeInvitation: @MainActor (String) async throws -> MeshInvitePreview
    init(decodeInvitation: @escaping @MainActor (String) async throws -> MeshInvitePreview) {
        self.decodeInvitation = decodeInvitation
    }

    @Published private(set) var pending: MeshPending?
    @Published private(set) var preview: MeshInvitePreview?
    @Published private(set) var browserURL: URL?
    @Published private(set) var busy = false
    @Published private(set) var problem: String?
    @Published private(set) var presentationOwner: UUID?
    private var presentationAttached = false
    private var work: Task<Void, Never>?
    #if !os(tvOS)
    private let browser = MeshPasskeyBrowser()
    #endif

    func ownsPresentation(_ owner: UUID) -> Bool {
        presentationOwner == owner && presentationAttached
    }

    func claimPresentation(_ owner: UUID) -> Bool {
        if ownsPresentation(owner) { return true }
        guard presentationOwner == nil, !busy else { return false }
        presentationAttached = true
        presentationOwner = owner
        return true
    }

    func reportProblem(_ message: String, owner: UUID) {
        guard ownsPresentation(owner) else { return }
        problem = message
    }

    func restore(owner: UUID) {
        guard ownsPresentation(owner), !busy else { return }
        do { pending = try MeshPendingVault.load() }
        catch { problem = error.localizedDescription }
    }

    @discardableResult
    func decode(_ text: String, owner: UUID) -> Task<Void, Never>? {
        start(owner: owner) {
            let value = try await self.decodeInvitation(text)
            try Task.checkCancellation()
            self.preview = value
        }
    }

    func decodeImage(_ bytes: Data, owner: UUID) {
        start(owner: owner) {
            let value = try await RustBridge.decodeMeshImage(bytes)
            try Task.checkCancellation()
            self.preview = value
        }
    }

    func begin(accountName: String, owner: UUID) {
        guard ownsPresentation(owner), let preview, pending == nil else { return }
        start(owner: owner) {
            let identity: MeshIdentity = try await RustBridge.call("meshIdentityCreate")
            try Task.checkCancellation()
            let name = accountName.trimmingCharacters(in: .whitespacesAndNewlines)
            let saved = MeshPending(identity: identity, code: preview.code,
                accountName: name.isEmpty ? nil : name)
            try MeshPendingVault.save(saved)
            self.pending = saved
            try await self.obtainChallenge()
        }
    }

    func resume(owner: UUID) {
        start(owner: owner) {
            if self.pending?.discardRequested == true { try await self.discardSavedRequest(); return }
            if self.pending?.begin == nil { try await self.obtainChallenge() }
            else if self.pending?.completion == nil { try await self.prepareBrowser() }
        }
    }

    private func obtainChallenge() async throws {
        guard var saved = pending, let code = saved.code else {
            throw BluefinError.message("Paste or scan a new connection invitation.")
        }
        var fields: [String: Any] = [
            "code": code, "deviceSeed": saved.identity.deviceSeed,
            "requestId": saved.identity.beginRequestId, "deviceLabel": deviceLabel]
        if let name = saved.accountName { fields["accountName"] = name }
        let begin: MeshBeginResponse = try await RustBridge.call("meshBegin", fields: fields)
        saved.begin = begin
        // Begin has spent the invitation. Its secret is no longer needed.
        saved.code = nil
        try MeshPendingVault.save(saved)
        pending = saved
        preview = nil
        try Task.checkCancellation()
        try await prepareBrowser()
    }

    private var deviceLabel: String {
        #if os(tvOS)
        "Bluefin Apple TV"
        #elseif targetEnvironment(macCatalyst)
        "Bluefin Mac"
        #elseif os(visionOS)
        "Bluefin Vision"
        #else
        "Bluefin Apple"
        #endif
    }

    private func prepareBrowser() async throws {
        guard let begin = pending?.begin, pending?.signupHandle == nil,
              pending?.discardRequested != true else { return }
        let result: MeshBrowserLocation = try await RustBridge.call("meshBrowserValidate", fields: [
            "apiOrigin": begin.apiOrigin, "challengeId": begin.challengeId,
            "browserUrl": begin.passkey["browserUrl"].foundation])
        try Task.checkCancellation()
        browserURL = result.url
    }

    #if !os(tvOS)
    func authenticate(store: LibraryStore, owner: UUID) {
        start(owner: owner) {
            try await self.prepareBrowser()
            guard let url = self.browserURL, let begin = self.pending?.begin else {
                throw BluefinError.message("Resume the saved connection first.")
            }
            let callback = try await self.browser.authenticate(url)
            let _: EmptyValue = try await RustBridge.call("meshBrowserCallback", fields: [
                "challengeId": begin.challengeId, "callback": callback.absoluteString])
            try await self.finish(store: store)
        }
    }
    #endif

    /// This also recovers a browser response lost after successful verification.
    /// The callback is never authority; Finish requires the original device key.
    func finishAfterApproval(store: LibraryStore, owner: UUID) {
        start(owner: owner) { try await self.finish(store: store) }
    }

    private func finish(store: LibraryStore) async throws {
        guard var saved = pending, let begin = saved.begin else {
            throw BluefinError.message("Resume the saved connection first.")
        }
        guard saved.discardRequested != true else {
            try await discardSavedRequest(); return
        }
        if let completion = saved.completion {
            try await complete(completion, saved: saved, store: store)
            return
        }
        if let handle = saved.signupHandle {
            let status: MeshSignupStatus = try await RustBridge.call("meshStatus", fields: [
                "deviceSeed": saved.identity.deviceSeed,
                "serverAddress": begin.serverAddress.foundation, "handle": handle])
            guard let completion = status.session else {
                problem = status.state == "pending"
                    ? "Your request is waiting for approval. Check again after the administrator approves it."
                    : "This signup request is no longer available."
                return
            }
            try await complete(completion, saved: saved, store: store)
            return
        }
        if saved.prepared == nil {
            let prepared: MeshPreparedFinish = try await RustBridge.call("meshFinishPrepare", fields: [
                "deviceSeed": saved.identity.deviceSeed, "requestId": saved.identity.finishRequestId,
                "challengeId": begin.challengeId, "deviceProofMessage": begin.deviceProofMessage,
                "passkeyResponse": ["browser": true]])
            try Task.checkCancellation()
            saved.prepared = prepared
            // Persist the exact signature before sending. A retry must not re-sign.
            try MeshPendingVault.save(saved)
            pending = saved
        }
        guard let prepared = saved.prepared else { throw CancellationError() }
        let completion: MeshCompletion = try await RustBridge.call("meshFinish", fields: [
            "deviceSeed": saved.identity.deviceSeed, "serverAddress": begin.serverAddress.foundation,
            "request": prepared.request.foundation])
        if completion.kind == "signup", let handle = completion.handle {
            saved.signupHandle = handle
            saved.prepared = nil
            try MeshPendingVault.save(saved)
            pending = saved
            browserURL = nil
            try Task.checkCancellation()
            problem = "Your request is waiting for administrator approval."
            return
        }
        try await complete(completion, saved: saved, store: store)
    }

    private func complete(_ completion: MeshCompletion, saved: MeshPending, store: LibraryStore) async throws {
        guard completion.kind == "session", let id = completion.id,
              let userId = completion.userId, let begin = saved.begin else {
            throw BluefinError.message("The server returned an incomplete connection.")
        }
        var settled = saved
        settled.completion = completion
        try MeshPendingVault.save(settled)
        pending = settled
        try Task.checkCancellation()
        let connection = MeshSavedSession(serverAddress: begin.serverAddress, sessionId: id,
            deviceSeed: saved.identity.deviceSeed, apiOrigin: begin.apiOrigin,
            deviceId: saved.identity.deviceId, userId: userId)
        try await store.acceptMeshSession(connection, username: completion.username ?? "Your account")
        try MeshPendingVault.clear()
        pending = nil; preview = nil; browserURL = nil
    }

    func discard(owner: UUID) {
        start(owner: owner) { try await self.discardSavedRequest() }
    }

    /// A sent Finish may have committed even when its reply was lost. Preserve
    /// the exact operation and key until reconciliation can retain a revocation.
    private func discardSavedRequest() async throws {
        guard var saved = pending else { return }
        saved.discardRequested = true
        try MeshPendingVault.save(saved)
        pending = saved
        browserURL = nil
        if saved.completion == nil, let begin = saved.begin {
            if let prepared = saved.prepared, saved.signupHandle == nil {
                let result: MeshCompletion = try await RustBridge.call("meshFinish", fields: [
                    "deviceSeed": saved.identity.deviceSeed,
                    "serverAddress": begin.serverAddress.foundation, "request": prepared.request.foundation])
                if result.kind == "signup", let handle = result.handle {
                    saved.signupHandle = handle
                    // Retain the exact Finish as recovery evidence as well.
                } else if result.kind == "session" {
                    saved.completion = result
                } else { throw BluefinError.message("The connection outcome is unconfirmed. Retry this saved request.") }
                try MeshPendingVault.save(saved)
                pending = saved
                try Task.checkCancellation()
            }
            if saved.completion == nil, let handle = saved.signupHandle {
                let status: MeshSignupStatus = try await RustBridge.call("meshStatus", fields: [
                    "deviceSeed": saved.identity.deviceSeed,
                    "serverAddress": begin.serverAddress.foundation, "handle": handle])
                if let completion = status.session {
                    saved.completion = completion
                    try MeshPendingVault.save(saved)
                    pending = saved
                    try Task.checkCancellation()
                } else if status.state != "denied" {
                    throw BluefinError.message("This request may still be approved. Its protected cleanup key is retained. Ask the administrator to deny it, or check again to finish disconnecting.")
                }
            }
        }
        if let result = saved.completion {
            guard result.kind == "session", let id = result.id, let user = result.userId,
                  let begin = saved.begin else {
                throw BluefinError.message("The connection outcome is unconfirmed. Retry this saved request.")
            }
            let connection = MeshSavedSession(serverAddress: begin.serverAddress, sessionId: id,
                deviceSeed: saved.identity.deviceSeed, apiOrigin: begin.apiOrigin,
                deviceId: saved.identity.deviceId, userId: user)
            try await MeshRevocationQueue.shared.prepareConnection(connection)
            try MeshSessionVault.clear(reference: id)
        }
        try MeshPendingVault.clear()
        pending = nil; preview = nil; browserURL = nil; problem = nil
        Task { await MeshRevocationQueue.shared.retry() }
    }

    @discardableResult
    private func start(owner: UUID, _ operation: @escaping @MainActor () async throws -> Void) -> Task<Void, Never>? {
        guard ownsPresentation(owner), !busy else { return nil }
        busy = true; problem = nil
        work = Task {
            defer {
                busy = false; work = nil
                if !presentationAttached {
                    preview = nil; browserURL = nil; problem = nil
                    presentationOwner = nil
                }
            }
            do { try Task.checkCancellation(); try await operation() }
            catch is CancellationError {}
            catch { if ownsPresentation(owner) { problem = error.localizedDescription } }
        }
        return work
    }

    func releasePresentation(_ owner: UUID) {
        guard ownsPresentation(owner) else { return }
        presentationAttached = false
        work?.cancel()
        // Only the presenting window may clear its input or cancel its browser.
        // Resumable device state stays in the protected pending slot.
        preview = nil; browserURL = nil; problem = nil
        #if !os(tvOS)
        browser.cancel()
        #endif
        // Cancellation cannot release an in-flight FFI operation. Its completion
        // returns the claim; a second window must wait before touching saved state.
        if !busy { presentationOwner = nil }
    }
}

struct MeshBrowserLocation: Decodable, Sendable { let url: URL }

#if !os(tvOS)
@MainActor private final class MeshPasskeyBrowser: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?
    private var completion: CheckedContinuation<URL, Error>?
    private var generation: UUID?

    func authenticate(_ url: URL) async throws -> URL {
        guard completion == nil else { throw BluefinError.message("A passkey request is already open.") }
        try Task.checkCancellation()
        let current = UUID()
        return try await withTaskCancellationHandler(operation: {
            try await withCheckedThrowingContinuation { continuation in
                guard !Task.isCancelled else { continuation.resume(throwing: CancellationError()); return }
                generation = current
                completion = continuation
                let browser = ASWebAuthenticationSession(url: url, callbackURLScheme: "bluefin") { [weak self] url, error in
                    Task { @MainActor in
                        if let url { self?.finish(current, .success(url)) }
                        else { self?.finish(current, .failure(error ?? CancellationError())) }
                    }
                }
                browser.presentationContextProvider = self
                browser.prefersEphemeralWebBrowserSession = true
                session = browser
                if !browser.start() {
                    finish(current, .failure(BluefinError.message("The passkey browser could not open. Try again.")))
                }
            }
        }, onCancel: { Task { @MainActor [weak self] in self?.cancel(current) } })
    }

    private func finish(_ current: UUID, _ result: Result<URL, Error>) {
        guard generation == current else { return }
        let pending = completion
        generation = nil; completion = nil; session = nil
        pending?.resume(with: result)
    }

    func cancel(_ expected: UUID? = nil) {
        guard let current = generation, expected == nil || expected == current else { return }
        let browser = session
        finish(current, .failure(CancellationError()))
        browser?.cancel()
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow) ?? ASPresentationAnchor()
    }
}
#endif

struct MeshPairingView: View {
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var model = MeshPairingModel.shared
    @EnvironmentObject private var nearby: MeshNearbyBrowser
    @State private var code = ""
    @State private var accountName = ""
    @State private var importing = false
    @State private var camera = false
    @State private var photos = false
    @State private var presentationID = UUID()
    @State private var visible = false
    var initialCode: String? = nil

    var body: some View {
        BluefinNavigation {
            Form {
                if !model.ownsPresentation(presentationID) {
                    Section {
                        Text("A connection is open in another Bluefin window. Finish or close it to continue here.")
                    }
                } else {
                    if let pending = model.pending {
                        Section("Saved connection") {
                            Text("This invitation is reserved for this device. You can resume without using the invitation again.")
                            if pending.discardRequested == true {
                                Button("Finish discarding connection") { model.discard(owner: presentationID) }.disabled(model.busy)
                                Text("The protected request is retained until its server outcome can be checked.")
                                    .font(.footnote).foregroundStyle(.secondary)
                            } else if pending.begin == nil {
                                Button("Resume connection") { model.resume(owner: presentationID) }.disabled(model.busy)
                            } else if pending.completion != nil {
                                Button("Open connected library") { model.finishAfterApproval(store: store, owner: presentationID) }.disabled(model.busy)
                            } else if pending.signupHandle != nil {
                                Button("Check approval") { model.finishAfterApproval(store: store, owner: presentationID) }.disabled(model.busy)
                            } else {
                                if let url = model.browserURL {
                                    MeshPasskeyCode(url: url)
                                    #if !os(tvOS)
                                    Button("Continue with passkey") { model.authenticate(store: store, owner: presentationID) }.disabled(model.busy)
                                    #endif
                                    Button("I've completed the passkey") { model.finishAfterApproval(store: store, owner: presentationID) }
                                        .disabled(model.busy)
                                    Text("The passkey page approves only this device's pending connection.")
                                        .font(.footnote).foregroundStyle(.secondary)
                                } else {
                                    Button("Resume passkey") { model.resume(owner: presentationID) }.disabled(model.busy)
                                }
                            }
                            if pending.discardRequested != true {
                                Button("Discard saved request", role: .destructive) { model.discard(owner: presentationID) }.disabled(model.busy)
                            }
                            Text("Discarding does not make the old invitation reusable.").font(.footnote).foregroundStyle(.secondary)
                        }
                    } else {
                        Section("Connect with an invitation") {
                            Text("Scan a Bluefin QR code, open an invitation image, or paste the full connection code.")
                            TextField("Connection code", text: $code).bluefinAddressInput()
                                .accessibilityIdentifier("meshConnectionCode")
                            Button("Check code") { model.decode(code, owner: presentationID) }.disabled(model.busy || code.isEmpty)
                            #if !os(tvOS)
                            Button("Paste code") {
                                if let value = UIPasteboard.general.string { code = value; model.decode(value, owner: presentationID) }
                            }.disabled(model.busy)
                            Button("Open QR image") { importing = true }.disabled(model.busy)
                            #endif
                            #if os(iOS) && !targetEnvironment(macCatalyst)
                            Button("Scan QR code") { camera = true }.disabled(model.busy)
                            Button("Choose photo or screenshot") { photos = true }.disabled(model.busy)
                            #endif
                        }
                        if !nearby.servers.isEmpty {
                            Section("Nearby Bluefin installations") {
                                ForEach(nearby.servers) { server in Label(server.name, systemImage: "network") }
                                Text("Use an invitation from the installation you want to connect. A nearby name alone does not verify its owner.")
                                    .font(.footnote).foregroundStyle(.secondary)
                            }
                        }
                        if let preview = model.preview {
                            Section("Ready to connect") {
                                Text(preview.intent == "signup" ? "Request your own account" : "Connect to your existing account")
                                Text("The invitation is used once. A passkey is required before this device receives access.")
                                if preview.intent == "signup" {
                                    TextField("Your account name", text: $accountName).bluefinAddressInput()
                                }
                                Button("Connect securely") { code = ""; model.begin(accountName: accountName, owner: presentationID) }
                                    .disabled(model.busy || (preview.intent == "signup" && accountName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
                                    .accessibilityIdentifier("meshBegin")
                            }
                        }
                    }
                    if model.busy { ProgressView("Connecting securely") }
                    if let problem = model.problem { Text(problem).foregroundStyle(.secondary).accessibilityIdentifier("meshPairingError") }
                }
            }
            .navigationTitle("Connect Bluefin")
            .toolbar { ToolbarItem(placement: .primaryAction) { Button("Done") { close() } } }
        }
        .task { visible = true; claimPresentation() }
        .onChange(of: model.presentationOwner) { if $0 == nil { claimPresentation() } }
        .onChange(of: store.signedIn) { if $0 { close() } }
        .onDisappear {
            visible = false; code = ""
            model.releasePresentation(presentationID)
        }
        #if !os(tvOS)
        .fileImporter(isPresented: $importing, allowedContentTypes: [.png, .jpeg]) { result in
            guard model.ownsPresentation(presentationID) else { return }
            do {
                let url = try result.get()
                let access = url.startAccessingSecurityScopedResource()
                defer { if access { url.stopAccessingSecurityScopedResource() } }
                let bytes = try MeshImageInput.read(url)
                model.decodeImage(bytes, owner: presentationID)
            } catch { model.reportProblem(error.localizedDescription, owner: presentationID) }
        }
        #endif
        #if os(iOS) && !targetEnvironment(macCatalyst)
        .sheet(isPresented: $camera) {
            MeshCameraScanner { value in camera = false; code = value; model.decode(value, owner: presentationID) }
                failure: { message in camera = false; model.reportProblem(message, owner: presentationID) }
        }
        .sheet(isPresented: $photos) {
            MeshPhotoPicker { result in
                photos = false
                switch result {
                case .success(let bytes): model.decodeImage(bytes, owner: presentationID)
                case .failure(let error): if !(error is CancellationError) { model.reportProblem(error.localizedDescription, owner: presentationID) }
                }
            }
        }
        #endif
    }

    private func claimPresentation() {
        guard visible, model.claimPresentation(presentationID) else { return }
        model.restore(owner: presentationID)
        if model.pending != nil { model.resume(owner: presentationID) }
        else if let initialCode {
            code = initialCode
            model.decode(initialCode, owner: presentationID)
        }
    }

    private func close() {
        visible = false; code = ""
        model.releasePresentation(presentationID)
        dismiss()
    }
}

private struct MeshPasskeyCode: View {
    let url: URL
    private var qr: CGImage? {
        let filter = CIFilter(name: "CIQRCodeGenerator", parameters: [
            "inputMessage": Data(url.absoluteString.utf8), "inputCorrectionLevel": "M"])
        guard let image = filter?.outputImage else { return nil }
        return CIContext().createCGImage(image, from: image.extent)
    }
    var body: some View {
        VStack(spacing: 12) {
            Text("Verify with your passkey").font(.headline)
            if let qr {
                Image(decorative: qr, scale: 1).interpolation(.none).resizable().scaledToFit()
                    .frame(width: 240, height: 240).padding(20).background(.white)
                    .accessibilityHidden(true)
            }
            Text("Scan with your phone if your passkey is on another device.")
                .font(.footnote).foregroundStyle(.secondary)
            if let host = url.host { Text(host).font(.footnote) }
        }
    }
}

enum MeshImageInput {
    static let maximumBytes = 8 * 1024 * 1024
    static func read(_ url: URL) throws -> Data {
        let values = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
        guard values.isRegularFile == true, let size = values.fileSize, size > 0, size <= maximumBytes else {
            throw BluefinError.message("Choose a PNG or JPEG image smaller than 8 MiB.")
        }
        let bytes = try Data(contentsOf: url, options: .mappedIfSafe)
        guard !bytes.isEmpty, bytes.count <= maximumBytes else {
            throw BluefinError.message("Choose a PNG or JPEG image smaller than 8 MiB.")
        }
        return bytes
    }
}
