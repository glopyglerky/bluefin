import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

private struct MeshIssuedInvite: Decodable, Sendable {
    let code: String
    let link: String
    let pngBase64: String
    let expiresAtUnixMs: UInt64
    let intent: String
    let singleUse: Bool
    let handle: String
}
private struct MeshInviteState: Decodable, Sendable {
    let state: String
    let expiresAtUnixMs: UInt64
}

struct MeshConnectionsView: View {
    @EnvironmentObject private var store: LibraryStore
    @ObservedObject private var revocations = MeshRevocationQueue.shared
    @State private var invitation: MeshIssuedInvite?
    @State private var image: UIImage?
    @State private var busy = false
    @State private var problem: String?
    @State private var sharing = false
    @State private var forgetConfirmation = false
    private var identity: String { TrailersAccount.identity(store.session) }

    var body: some View {
        Form {
            Section("This device") {
                Text(store.session?.meshSessionId == nil ? "Signed in through the server address" : "Connected through the encrypted Bluefin mesh")
                Text("Your device's private connection key stays in this device's protected storage.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            Section("Connect another device to your account") {
                Text("Use this invitation on a device you own. Your account passkey is still required.")
                Button(invitation == nil ? "Create one-use invitation" : "Create a new invitation") {
                    Task { await create() }
                }.disabled(busy || store.preview)
                if let invitation, let image {
                    Image(uiImage: image).interpolation(.none).resizable().scaledToFit()
                        .frame(maxWidth: 320).accessibilityHidden(true)
                    Text("Expires \(Date(timeIntervalSince1970: Double(invitation.expiresAtUnixMs) / 1000), style: .relative)")
                        .font(.footnote)
                    #if !os(tvOS)
                    Button("Copy invitation") {
                        UIPasteboard.general.setItems([[UIPasteboard.typeAutomatic: invitation.link]],
                            options: [.localOnly: true, .expirationDate: Date(timeIntervalSince1970: Double(invitation.expiresAtUnixMs) / 1000)])
                    }
                    Button("Share QR image") { sharing = true }
                    #endif
                    Button("Hide invitation") { self.invitation = nil; self.image = nil }
                    Text("Every copy is the same invitation. Once claimed, no copy can connect another device.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                Text("Sharing a library with another person requires a separate grant to that person's own account.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            if revocations.count > 0 || revocations.problem != nil {
                Section("Pending sign-out confirmation") {
                    Text("\(revocations.count) device connection requests await confirmation.")
                    if let issue = revocations.problem { Text(issue).font(.footnote).foregroundStyle(.secondary) }
                    Button("Retry remote sign-out") { Task { await revocations.retry() } }.disabled(revocations.busy)
                    Button("Erase pending local keys", role: .destructive) { forgetConfirmation = true }.disabled(revocations.busy)
                }
            }
            if busy { ProgressView("Creating invitation") }
            if let problem { Text(problem).foregroundStyle(.secondary) }
        }
        .navigationTitle("Connections")
        .task(id: invitation?.handle) { await observe() }
        .onChange(of: identity) { _ in invitation = nil; image = nil; problem = nil }
        .onDisappear { invitation = nil; image = nil }
        .confirmationDialog("Erase pending sign-out keys?", isPresented: $forgetConfirmation, titleVisibility: .visible) {
            Button("Erase local keys", role: .destructive) {
                do { try revocations.forget() } catch { problem = error.localizedDescription }
            }
        } message: {
            Text("This erases the device keys needed to retry remote sign-out. It does not confirm that the server revoked those connections.")
        }
        #if !os(tvOS)
        .sheet(isPresented: $sharing) {
            if let image { MeshInviteShare(image: image) }
        }
        #endif
    }

    @MainActor private func create() async {
        guard !busy, let session = store.session else { return }
        let account = identity
        busy = true; problem = nil; invitation = nil; image = nil
        defer { busy = false }
        do {
            let value: MeshIssuedInvite = try await RustBridge.call("meshAccountInvite", session: session)
            try Task.checkCancellation()
            guard account == identity, value.singleUse, value.intent == "login",
                  let data = Data(base64Encoded: value.pngBase64), data.count <= 1024 * 1024,
                  let picture = UIImage(data: data) else {
                throw BluefinError.message("The server returned an invalid invitation.")
            }
            invitation = value; image = picture
        } catch is CancellationError {} catch { if account == identity { problem = error.localizedDescription } }
    }

    @MainActor private func observe() async {
        guard let saved = invitation, let session = store.session else { return }
        let account = identity
        do {
            while invitation?.handle == saved.handle, account == identity {
                try await Task.sleep(nanoseconds: 2_000_000_000)
                let status: MeshInviteState = try await RustBridge.call("meshAccountInviteStatus", session: session,
                    fields: ["handle": saved.handle])
                try Task.checkCancellation()
                guard account == identity, invitation?.handle == saved.handle else { return }
                if status.state != "issued" {
                    invitation = nil; image = nil
                    problem = status.state == "used" ? "This invitation was claimed. Create a new invitation for another device." : "This invitation is no longer valid."
                    return
                }
            }
        } catch is CancellationError {} catch {
            if account == identity, invitation?.handle == saved.handle {
                // If current authority cannot be checked, stop displaying the capability.
                invitation = nil; image = nil
                problem = "The invitation status could not be confirmed. Reconnect before creating another."
            }
        }
    }
}

#if !os(tvOS)
private struct MeshInviteShare: UIViewControllerRepresentable {
    let image: UIImage
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [image], applicationActivities: nil)
    }
    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
#endif
