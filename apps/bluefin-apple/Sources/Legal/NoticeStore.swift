import Foundation

enum NoticeReadError: Error { case unavailable, invalid }

struct NoticeComponent: Identifiable, Sendable {
    let id: String
    let title: String
    let resources: [URL]
    let source: String
}

private struct NoticeIndex: Decodable {
    let platform: String
    let target: String
    let first_party: [String]
    let standard_library_html: String
    let packages: [Package]
    struct Package: Decodable {
        let package: String
        let version: String?
        let texts: [String]
        let reference: String
    }
}

struct NoticeStore: Sendable {
    let root: URL
    private let limit = 4 * 1024 * 1024

    func resource(_ reference: String, relativeTo base: URL) throws -> URL {
        guard !reference.isEmpty, reference.utf8.count <= 4096,
              !reference.hasPrefix("/"),
              !reference.contains(where: { ":\\%?#".contains($0) || $0.asciiValue.map { $0 < 32 || $0 == 127 } == true }) else {
            throw NoticeReadError.invalid
        }
        let boundary = root.standardizedFileURL.resolvingSymlinksInPath()
        let parent = base.deletingLastPathComponent().standardizedFileURL.resolvingSymlinksInPath()
        guard parent.path == boundary.path || parent.path.hasPrefix(boundary.path + "/") else {
            throw NoticeReadError.invalid
        }
        let result = parent.appendingPathComponent(reference).standardizedFileURL.resolvingSymlinksInPath()
        guard result.path.hasPrefix(boundary.path + "/") else { throw NoticeReadError.invalid }
        let values = try result.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard values.isRegularFile == true, let size = values.fileSize, size > 0, size <= limit else {
            throw NoticeReadError.unavailable
        }
        return result
    }

    func components(indexName: String?, target: String?) throws -> [NoticeComponent] {
        let entry = root.appendingPathComponent("entry")
        guard let indexName else {
            guard target == nil else { throw NoticeReadError.invalid }
            return [NoticeComponent(id: "bluefin", title: "Bluefin license and attribution",
                resources: try ["first-party/LICENSE", "first-party/README.md"].map { try resource($0, relativeTo: entry) }, source: "")]
        }
        guard let target, !target.isEmpty else { throw NoticeReadError.invalid }
        let indexURL = try resource("indexes/" + indexName + ".json", relativeTo: entry)
        let index = try JSONDecoder().decode(NoticeIndex.self, from: Data(contentsOf: indexURL))
        guard index.platform == "apple", index.target == target,
              !index.packages.isEmpty, index.packages.count <= 4096,
              !index.first_party.isEmpty, index.first_party.count <= 64 else { throw NoticeReadError.invalid }
        var entries = [NoticeComponent(id: "bluefin", title: "Bluefin license and attribution",
            resources: try index.first_party.map { try resource($0, relativeTo: indexURL) }, source: ""),
            NoticeComponent(id: "standard-library", title: "Standard-library copyright notices",
            resources: [try resource(index.standard_library_html, relativeTo: indexURL)], source: "")]
        entries += try index.packages.enumerated().map { offset, package in
            guard !package.package.isEmpty, package.package.utf8.count <= 256,
                  package.texts.count <= 64, package.reference.utf8.count <= 4096,
                  (package.version?.utf8.count ?? 0) <= 256 else { throw NoticeReadError.invalid }
            return NoticeComponent(id: "package-\(offset)", title: package.package + (package.version.map { " " + $0 } ?? ""),
                resources: try package.texts.map { try resource($0, relativeTo: indexURL) }, source: package.reference)
        }
        return entries
    }

    func provenance() -> [String: [String]] {
        guard let url = try? resource("third-party/text-provenance.json", relativeTo: root.appendingPathComponent("entry")),
              let bytes = try? Data(contentsOf: url),
              let value = try? JSONDecoder().decode([String: [String]].self, from: bytes) else { return [:] }
        return value
    }

    func label(for url: URL, in component: NoticeComponent, provenance: [String: [String]]) -> String {
        if url.path.hasSuffix("/first-party/LICENSE") { return "License" }
        if url.path.hasSuffix("/first-party/README.md") { return "About Bluefin" }
        let name = url.lastPathComponent
        if name == "COPYRIGHT-library.html" { return "Standard-library copyright notices" }
        let digest = url.deletingPathExtension().lastPathComponent
        guard url.pathExtension == "txt", digest.count == 64,
              digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { return name }
        let prefix = component.source + " : "
        let names = Set((provenance[digest] ?? []).compactMap { value -> String? in
            guard !component.source.isEmpty, value.hasPrefix(prefix) else { return nil }
            let result = String(value.dropFirst(prefix.count))
            guard !result.isEmpty, result.count <= 100,
                  !result.contains(where: { $0 == "/" || $0 == "\\" || $0.asciiValue.map { $0 < 32 || $0 == 127 } == true }) else { return nil }
            return result
        })
        if names.count == 1, let original = names.first { return original }
        return "License text \((component.resources.firstIndex(of: url) ?? 0) + 1)"
    }
}
