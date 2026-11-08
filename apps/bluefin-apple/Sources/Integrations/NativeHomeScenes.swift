#if os(iOS) && BLUEFIN_HOMEKIT
import HomeKit
import SwiftUI

/// Construct only from an explicit Home settings action, never during app attachment.
@MainActor final class NativeHomeScenes: NSObject, ObservableObject, HMHomeManagerDelegate {
    @Published private(set) var homes: [HMHome] = []
    @Published private(set) var ready = false
    @Published var error: String?
    private var manager: HMHomeManager?
    func requestHomes() {
        guard manager == nil else { return }
        let manager = HMHomeManager()
        self.manager = manager
        manager.delegate = self
    }
    nonisolated func homeManagerDidUpdateHomes(_ manager: HMHomeManager) {
        Task { @MainActor [weak self] in
            self?.homes = manager.homes
            self?.ready = true
        }
    }
    func execute(_ scene: HMActionSet, in home: HMHome) async throws {
        guard ready, homes.contains(where: { $0.uniqueIdentifier == home.uniqueIdentifier }),
              home.actionSets.contains(where: { $0.uniqueIdentifier == scene.uniqueIdentifier }) else {
            throw BluefinError.message("Choose an authorized Home scene first.")
        }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            home.executeActionSet(scene) { error in
                if let error { continuation.resume(throwing: error) } else { continuation.resume() }
            }
        }
    }
}
#endif
