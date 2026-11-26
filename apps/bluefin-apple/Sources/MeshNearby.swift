import Foundation
import Combine

struct MeshNearbyServer: Decodable, Identifiable, Sendable {
    let name: String
    let endpointId: String
    let txt: MeshJSON
    let addresses: MeshJSON
    let trusted: Bool
    var id: String { endpointId }
    var refreshFields: [String: Any] {
        ["txt": txt.foundation, "addresses": addresses.foundation]
    }
}

/// Bonjour supplies route hints. The Rust matcher requires the saved endpoint,
/// service and certificate pin before a sighting can update a paired connection.
@MainActor final class MeshNearbyBrowser: NSObject, ObservableObject, NetServiceBrowserDelegate, NetServiceDelegate {
    @Published private(set) var servers: [MeshNearbyServer] = []
    @Published private(set) var problem: String?
    @Published private(set) var revision = UUID()
    private var browser: NetServiceBrowser?
    private var services: [String: NetService] = [:]
    private var decoded: [String: MeshNearbyServer] = [:]
    private var tasks: [String: Task<Void, Never>] = [:]
    private var generation = UUID()

    func start() {
        guard browser == nil else { return }
        generation = UUID(); problem = nil
        let browser = NetServiceBrowser()
        browser.delegate = self
        browser.includesPeerToPeer = false
        self.browser = browser
        browser.searchForServices(ofType: "_bluefin-mesh._udp.", inDomain: "local.")
    }
    func stop() {
        generation = UUID()
        browser?.stop(); browser?.delegate = nil; browser = nil
        for service in services.values { service.stop(); service.delegate = nil }
        services.removeAll()
        for task in tasks.values { task.cancel() }
        tasks.removeAll(); decoded.removeAll(); servers = []
    }
    private func key(_ service: NetService) -> String {
        service.domain + "/" + service.type + "/" + service.name
    }
    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        guard self.browser === browser, services.count < 32 else { return }
        let key = key(service)
        guard services[key] == nil else { return }
        services[key] = service
        service.includesPeerToPeer = false
        service.delegate = self
        service.resolve(withTimeout: 6)
    }
    func netServiceBrowser(_ browser: NetServiceBrowser, didRemove service: NetService, moreComing: Bool) {
        let key = key(service)
        services.removeValue(forKey: key)?.stop()
        tasks.removeValue(forKey: key)?.cancel()
        decoded.removeValue(forKey: key)
        publish()
    }
    func netServiceBrowser(_ browser: NetServiceBrowser, didNotSearch errorDict: [String: NSNumber]) {
        guard self.browser === browser else { return }
        problem = "Nearby discovery is unavailable. Check local network permission, or use an invitation."
    }
    func netServiceDidResolveAddress(_ sender: NetService) {
        let key = key(sender)
        guard services[key] === sender, let txt = sender.txtRecordData(), txt.count <= 4096 else { return }
        let values = NetService.dictionary(fromTXTRecord: txt)
        guard values.count <= 16, values.allSatisfy({ $0.key.utf8.count <= 64 && $0.value.count <= 1024 }) else { return }
        let addresses = sender.addresses ?? []
        guard addresses.count <= 32, addresses.allSatisfy({ $0.count <= 128 }) else { return }
        let fields: [String: Any] = [
            "txt": values.sorted(by: { $0.key < $1.key }).map { [$0.key, Array($0.value)] as [Any] },
            "addresses": addresses.map { Array($0) }]
        let epoch = generation
        tasks[key]?.cancel()
        tasks[key] = Task {
            defer { if generation == epoch { tasks[key] = nil } }
            do {
                let server: MeshNearbyServer = try await RustBridge.call("meshLanDecode", fields: fields)
                try Task.checkCancellation()
                guard generation == epoch, services[key] === sender else { return }
                decoded[key] = server
                publish()
            } catch { /* An untrusted or malformed advertisement is not an app error. */ }
        }
    }
    func netService(_ sender: NetService, didNotResolve errorDict: [String: NSNumber]) {
        let key = key(sender)
        tasks.removeValue(forKey: key)?.cancel()
        decoded.removeValue(forKey: key)
        publish()
    }
    private func publish() {
        revision = UUID()
        // Conflicting advertisements for an endpoint cannot choose a route.
        let groups = Dictionary(grouping: decoded.values, by: \.endpointId)
        servers = groups.values.compactMap { values in
            guard let first = values.first,
                  values.allSatisfy({ $0.txt == first.txt && $0.addresses == first.addresses }) else { return nil }
            return first
        }.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    func refresh(_ session: AccountSession?) async {
        guard let session, let reference = session.meshSessionId else { return }
        let epoch = generation
        for observation in servers {
            do {
                var saved = try MeshSessionVault.load(reference: reference)
                var fields = observation.refreshFields
                fields["serverAddress"] = saved.serverAddress.foundation
                let address: MeshJSON = try await RustBridge.call("meshLanRefresh", fields: fields)
                try Task.checkCancellation()
                guard epoch == generation,
                      try SessionVault.loadChecked()?.meshSessionId == reference else { return }
                // Never overwrite a newer route update or a logout.
                let current = try MeshSessionVault.load(reference: reference)
                guard current.serverAddress == saved.serverAddress else { continue }
                saved.serverAddress = address
                try MeshSessionVault.save(saved)
                return
            } catch { continue }
        }
    }
}
