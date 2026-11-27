import Foundation

/// Route hints for an already-owned connection. This never rewrites the vault,
/// an imported invitation, or any identity field. Rust still validates the
/// address, application relay policy, pinned handshake and live account session.
enum MeshRelayRoutes {
    static func applyingApprovedFallback(
        to request: [String: Any], approvedRelays: [String]
    ) -> [String: Any] {
        guard !approvedRelays.isEmpty, approvedRelays.count <= 2,
              approvedRelays.allSatisfy(isCanonicalRelay),
              var mesh = request["mesh"] as? [String: Any],
              var address = mesh["serverAddress"] as? [String: Any],
              let existing = address["relay_urls"] as? [String], existing.isEmpty else {
            return request
        }
        address["relay_urls"] = Array(Set(approvedRelays)).sorted()
        mesh["serverAddress"] = address
        var routed = request
        routed["mesh"] = mesh
        return routed
    }

    private static func isCanonicalRelay(_ value: String) -> Bool {
        guard value.utf8.count <= 256, value.utf8.allSatisfy({ $0 < 128 }),
              let url = URLComponents(string: value),
              url.scheme == "https", let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil, url.path == "/",
              url.query == nil, url.fragment == nil,
              url.url?.absoluteString == value else { return false }
        return true
    }
}
