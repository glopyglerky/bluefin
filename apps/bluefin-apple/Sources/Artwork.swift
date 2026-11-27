import SwiftUI
import ImageIO
import CryptoKit

private final class SameOriginImages: NSObject, URLSessionTaskDelegate, URLSessionDownloadDelegate {
    static let shared = SameOriginImages()
    static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 15
        config.timeoutIntervalForResource = 25
        config.urlCache = nil
        config.httpMaximumConnectionsPerHost = 4
        return URLSession(configuration: config, delegate: shared, delegateQueue: nil)
    }()
    private let diskLock = NSLock()
    private var epoch = UUID()
    private var directory: URL? {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?.appendingPathComponent("Bluefin/Artwork", isDirectory: true)
    }
    func generation() -> UUID { diskLock.lock(); defer { diskLock.unlock() }; return epoch }
    private func diskURL(_ key: String) -> URL? {
        directory?.appendingPathComponent(SHA256.hash(data: Data(key.utf8)).map { String(format: "%02x", $0) }.joined() + ".jpg")
    }
    func cachedFile(_ key: String) -> URL? {
        diskLock.lock(); defer { diskLock.unlock() }
        guard let file = diskURL(key), FileManager.default.fileExists(atPath: file.path) else { return nil }
        try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: file.path)
        return file
    }
    func storeFile(_ data: Data, key: String, generation: UUID) {
        guard data.count <= 2 * 1024 * 1024 else { return }
        diskLock.lock(); defer { diskLock.unlock() }
        guard epoch == generation, let directory, let file = diskURL(key) else { return }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try data.write(to: file, options: .atomic)
            let files = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.fileSizeKey, .contentModificationDateKey])
            var entries = files.compactMap { file -> (URL, Int, Date)? in
                guard let values = try? file.resourceValues(forKeys: [.fileSizeKey, .contentModificationDateKey]) else { return nil }
                return (file, values.fileSize ?? 0, values.contentModificationDate ?? .distantPast)
            }
            var total = entries.reduce(0) { $0 + $1.1 }
            entries.sort { $0.2 < $1.2 }
            for entry in entries where total > 64 * 1024 * 1024 {
                try FileManager.default.removeItem(at: entry.0); total -= entry.1
            }
        } catch { /* Artwork caching never prevents browsing or playback. */ }
    }
    func cachedImage(_ key: NSString, generation: UUID) -> UIImage? {
        diskLock.lock(); defer { diskLock.unlock() }
        guard epoch == generation else { return nil }
        return cache.object(forKey: key)
    }
    func cacheImage(_ image: UIImage, key: NSString, generation: UUID) -> Bool {
        diskLock.lock(); defer { diskLock.unlock() }
        guard epoch == generation else { return false }
        cache.setObject(image, forKey: key, cost: Int(image.size.width * image.size.height * 4))
        return true
    }
    func clear() {
        diskLock.lock(); defer { diskLock.unlock() }
        epoch = UUID()
        cache.removeAllObjects()
        if let directory { try? FileManager.default.removeItem(at: directory) }
    }
    let cache: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.totalCostLimit = 32 * 1024 * 1024
        cache.countLimit = 120
        return cache
    }()
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        guard let original = task.originalRequest?.url, let next = request.url,
              original.scheme == next.scheme, original.host == next.host, original.port == next.port else { completionHandler(nil); return }
        completionHandler(request)
    }
    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64, totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        if totalBytesWritten > 8 * 1024 * 1024 || totalBytesExpectedToWrite > 8 * 1024 * 1024 { downloadTask.cancel() }
    }
    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {}
}

struct Artwork: View {
    let item: MediaItem
    var backdrop = false
    var thumbnailWidth: Int? = nil
    var contentMode: ContentMode = .fill
    @EnvironmentObject private var store: LibraryStore
    @State private var image: UIImage?
    @State private var imageKey: String?
    @State private var requestGeneration = UUID()
    @MainActor private static var inFlight: [String: Task<UIImage?, Never>] = [:]

    private typealias Source = ArtworkSource

    private static func sources(for item: MediaItem, backdrop: Bool) -> [Source] {
        ArtworkPlan.sources(for: item, backdrop: backdrop)
    }

    private var key: String {
        "\(store.accountContext)/\(item.Id)/\(backdrop)/\(thumbnailWidth ?? 0)/" +
        Self.sources(for: item, backdrop: backdrop).map { "\($0.id)/\($0.kind)/\($0.tag ?? "")" }.joined(separator: "/")
    }
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                LinearGradient(colors: colors, startPoint: .topLeading, endPoint: .bottomTrailing)
                if item.Id.hasPrefix("preview") { PreviewArtwork(music: ["Audio", "MusicAlbum", "MusicArtist"].contains(item.Type ?? "")) }
                else if let image, imageKey == key { Image(uiImage: image).resizable().aspectRatio(contentMode: contentMode) }
                else {
                    VStack(spacing: 12) {
                        Image(systemName: symbol).font(.system(size: backdrop ? 72 : 40, weight: .ultraLight)).opacity(0.5)
                    }.foregroundStyle(BluefinStyle.secondaryText)
                }
            }.frame(width: geometry.size.width, height: geometry.size.height).clipped()
        }
        .accessibilityHidden(true)
        .task(id: key) {
            let request = UUID()
            requestGeneration = request
            await load(request: request)
        }
        .onDisappear {
            requestGeneration = UUID()
            image = nil
            imageKey = nil
        }
    }
    private var symbol: String {
        switch item.Type { case "Audio", "MusicAlbum", "MusicArtist": return "waveform"; case "TvChannel": return "dot.radiowaves.left.and.right"; case "Series", "Episode": return "tv"; default: return "film" }
    }
    private var colors: [Color] {
        [BluefinStyle.secondaryBackground, BluefinStyle.background]
    }
    private func load(request: UUID) async {
        let requested = key
        guard let session = store.session, !store.preview else { return }
        let loaded = await Self.nativeImage(for: item, session: session, backdrop: backdrop, thumbnailWidth: thumbnailWidth)
        guard !Task.isCancelled, requested == key, request == requestGeneration else { return }
        image = loaded
        imageKey = requested
    }
    @MainActor static func nativeImage(
        for item: MediaItem, session: AccountSession, backdrop: Bool = false, thumbnailWidth: Int? = nil
    ) async -> UIImage? {
        // Device-owned IDs contain a private content digest. They are local
        // routing identifiers, never server item IDs or permission grants.
        // Keep all callers (rows, player, and now-playing artwork) offline.
        guard !item.Id.hasPrefix("owned-") else { return nil }
        let pixelWidth = ArtworkPlan.pixelWidth(thumbnailWidth, backdrop: backdrop)
        let cache = SameOriginImages.shared
        let generation = cache.generation()
        for source in sources(for: item, backdrop: backdrop) {
            guard !Task.isCancelled, generation == cache.generation() else { return nil }
            // Tagged art is content-versioned. Recheck untagged legacy art periodically.
            let version = source.tag ?? "untagged-\(Int(Date().timeIntervalSince1970 / 900))"
            let key = "\(session.server)/\(session.userId)/\(session.meshSessionId ?? "https")/\(source.id)/\(source.kind)/\(version)/\(pixelWidth)"
            if let cached = cache.cachedImage(key as NSString, generation: generation) { return cached }
            if let pending = inFlight[key] {
                let image = await pending.value
                guard !Task.isCancelled, generation == cache.generation() else { return nil }
                if let image { return image }
                continue
            }
            let pending = Task.detached(priority: .userInitiated) {
                await fetch(source: source, session: session, width: pixelWidth, key: key, generation: generation)
            }
            inFlight[key] = pending
            let image = await pending.value
            guard generation == cache.generation() else { return nil }
            inFlight[key] = nil
            guard !Task.isCancelled else { return nil }
            if let image { return image }
        }
        return nil
    }

    nonisolated private static func decode(_ file: URL, width: Int) -> UIImage? {
        guard let size = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 8 * 1024 * 1024,
              let source = CGImageSourceCreateWithURL(file as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                  kCGImageSourceCreateThumbnailFromImageAlways: true,
                  kCGImageSourceThumbnailMaxPixelSize: width,
                  kCGImageSourceCreateThumbnailWithTransform: true,
                  kCGImageSourceShouldCacheImmediately: true,
              ] as CFDictionary) else { return nil }
        return UIImage(cgImage: cg)
    }

    nonisolated private static func fetch(source: Source, session: AccountSession, width: Int, key: String, generation: UUID) async -> UIImage? {
        guard !Task.isCancelled, var components = URLComponents(string: session.server), components.host != nil,
              source.id.range(of: "^[A-Za-z0-9_-]+$", options: .regularExpression) != nil else { return nil }
        while components.path.hasSuffix("/") { components.path.removeLast() }
        components.path += "/Items/\(source.id)/Images/\(source.kind)"
        components.queryItems = [URLQueryItem(name: "maxWidth", value: String(width)), URLQueryItem(name: "quality", value: "85")]
        if let tag = source.tag { components.queryItems?.append(URLQueryItem(name: "tag", value: tag)) }
        guard let url = components.url else { return nil }
        return await fetchResource(url: url, session: session, width: width, key: key, generation: generation)
    }

    nonisolated private static func fetchResource(url: URL, session: AccountSession, width: Int,
                                                  key: String, generation: UUID) async -> UIImage? {
        let cache = SameOriginImages.shared
        let cacheKey = key as NSString
        if let file = cache.cachedFile(key), let diskImage = decode(file, width: width),
           !Task.isCancelled, cache.cacheImage(diskImage, key: cacheKey, generation: generation) { return diskImage }
        guard !Task.isCancelled, generation == cache.generation() else { return nil }
        if session.meshSessionId != nil {
            do {
                let response: MeshArtworkBytes = try await RustBridge.call("meshImage", session: session,
                    fields: ["resource": url.absoluteString])
                guard response.mime.hasPrefix("image/"), let data = Data(base64Encoded: response.bytesBase64),
                      data.count <= 5 * 1024 * 1024,
                      let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
                      let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                        kCGImageSourceCreateThumbnailFromImageAlways: true,
                        kCGImageSourceThumbnailMaxPixelSize: width,
                        kCGImageSourceCreateThumbnailWithTransform: true,
                        kCGImageSourceShouldCacheImmediately: true,
                      ] as CFDictionary) else { return nil }
                try Task.checkCancellation()
                let decoded = UIImage(cgImage: cg)
                guard cache.cacheImage(decoded, key: cacheKey, generation: generation) else { return nil }
                persist(decoded, key: key, generation: generation)
                return decoded
            } catch { return nil }
        }
        var request = URLRequest(url: url)
        guard !session.token.isEmpty, !session.token.contains(where: { $0 == "\"" || $0 == "\\" || $0.asciiValue.map { $0 < 32 || $0 == 127 } == true }) else { return nil }
        request.setValue("MediaBrowser Client=\"Bluefin Apple\", Token=\"\(session.token)\"", forHTTPHeaderField: "Authorization")
        do {
            let (file, response) = try await SameOriginImages.session.download(for: request, delegate: cache)
            defer { try? FileManager.default.removeItem(at: file) }
            guard let http = response as? HTTPURLResponse, http.statusCode == 200, http.mimeType?.hasPrefix("image/") == true,
                  response.expectedContentLength <= 8 * 1024 * 1024 else { return nil }
            try Task.checkCancellation()
            guard let decoded = decode(file, width: width), cache.cacheImage(decoded, key: cacheKey, generation: generation) else { return nil }
            // Show the decoded artwork before compressing or pruning the persistent cache.
            persist(decoded, key: key, generation: generation)
            return decoded
        } catch { return nil }
    }
    nonisolated private static func persist(_ image: UIImage, key: String, generation: UUID) {
        Task.detached(priority: .utility) {
            let alpha = image.cgImage?.alphaInfo
            let transparent = alpha == .premultipliedFirst || alpha == .premultipliedLast || alpha == .first || alpha == .last
            // Keep transparency in channel logos and other PNG artwork.
            let data = transparent ? image.pngData() : image.jpegData(compressionQuality: 0.82)
            if let data { SameOriginImages.shared.storeFile(data, key: key, generation: generation) }
        }
    }

    @MainActor static func nativeSportsLogo(resource: String?, session: AccountSession) async -> UIImage? {
        guard let hash = SportsLogoResource.hash(resource),
              var components = URLComponents(string: session.server), components.host != nil else { return nil }
        while components.path.hasSuffix("/") { components.path.removeLast() }
        components.path += "/sports/api/v1/logos/" + hash
        components.query = nil; components.fragment = nil
        guard let url = components.url else { return nil }
        let cache = SameOriginImages.shared
        let generation = cache.generation()
        let key = "\(session.server)/\(session.userId)/\(session.meshSessionId ?? "https")/sports-logo/\(hash)/192"
        if let image = cache.cachedImage(key as NSString, generation: generation) { return image }
        if let pending = inFlight[key] {
            let image = await pending.value
            guard !Task.isCancelled, generation == cache.generation() else { return nil }
            return image
        }
        let pending = Task.detached(priority: .userInitiated) {
            await fetchResource(url: url, session: session, width: 192, key: key, generation: generation)
        }
        inFlight[key] = pending
        let image = await pending.value
        guard generation == cache.generation() else { return nil }
        inFlight[key] = nil
        guard !Task.isCancelled else { return nil }
        return image
    }

    @MainActor static func prefetch(items: [MediaItem], session: AccountSession) async {
        var seen = Set<String>()
        let pending = Array(items.filter { seen.insert($0.Id).inserted }.prefix(12))
        await withTaskGroup(of: Void.self) { group in
            var next = 0
            func enqueue(_ item: MediaItem) {
                group.addTask {
                    guard !Task.isCancelled else { return }
                    _ = await nativeImage(for: item, session: session)
                }
            }
            for _ in 0..<min(2, pending.count) { enqueue(pending[next]); next += 1 }
            while await group.next() != nil {
                guard !Task.isCancelled else { group.cancelAll(); return }
                if next < pending.count { enqueue(pending[next]); next += 1 }
            }
        }
    }

    @MainActor static func clearCache() {
        for request in inFlight.values { request.cancel() }
        inFlight.removeAll()
        SameOriginImages.shared.clear()
    }
}

private struct MeshArtworkBytes: Decodable, Sendable { let bytesBase64: String; let mime: String }
