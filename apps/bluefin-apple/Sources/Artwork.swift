import SwiftUI
import ImageIO
import CryptoKit

private final class SameOriginImages: NSObject, URLSessionTaskDelegate, URLSessionDownloadDelegate {
    static let shared = SameOriginImages()
    lazy var session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 15
        config.timeoutIntervalForResource = 25
        config.urlCache = nil
        config.httpMaximumConnectionsPerHost = 4
        return URLSession(configuration: config, delegate: self, delegateQueue: nil)
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
    @EnvironmentObject private var store: LibraryStore
    @State private var image: UIImage?
    private var key: String { Self.cacheKey(item, session: store.session, backdrop: backdrop) }
    private static func cacheKey(_ item: MediaItem, session: AccountSession?, backdrop: Bool) -> String {
        "\(session?.server ?? "preview")/\(session?.userId ?? "")/\(item.Id)/\(backdrop)/\(backdrop ? item.BackdropImageTags?.first ?? "" : item.ImageTags?["Primary"] ?? "")"
    }
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                LinearGradient(colors: colors, startPoint: .topLeading, endPoint: .bottomTrailing)
                if let image { Image(uiImage: image).resizable().scaledToFill() }
                else {
                    VStack(spacing: 12) {
                        Image(systemName: symbol).font(.system(size: backdrop ? 72 : 40, weight: .ultraLight)).opacity(0.5)
                        if item.Id.hasPrefix("preview") { Text(item.title).font(backdrop ? .largeTitle.bold() : .title3.bold()).multilineTextAlignment(.center).padding(.horizontal, 12) }
                    }.foregroundColor(.white)
                }
            }.frame(width: geometry.size.width, height: geometry.size.height).clipped()
        }
        .accessibilityHidden(true)
        .task(id: key) { await load() }
    }
    private var symbol: String {
        switch item.Type { case "Audio", "MusicAlbum", "MusicArtist": return "waveform"; case "TvChannel": return "dot.radiowaves.left.and.right"; case "Series", "Episode": return "tv"; default: return "film" }
    }
    private var colors: [Color] {
        // Stable per item; no process-random hash affects artwork between launches.
        let hue = Double(item.Id.utf8.reduce(0) { ($0 + Int($1)) % 360 }) / 360
        return [Color(hue: hue, saturation: 0.52, brightness: 0.43), Color(hue: hue, saturation: 0.72, brightness: 0.16)]
    }
    private func load() async {
        image = nil
        guard let session = store.session, !store.preview else { return }
        image = await Self.nativeImage(for: item, session: session, backdrop: backdrop)
    }
    static func nativeImage(for item: MediaItem, session: AccountSession, backdrop: Bool = false) async -> UIImage? {
        let key = cacheKey(item, session: session, backdrop: backdrop)
        let cache = SameOriginImages.shared
        let cacheKey = key as NSString
        if let cached = cache.cache.object(forKey: cacheKey) { return cached }
        let generation = cache.generation()
        let identity = key
        if let file = cache.cachedFile(identity) {
            let cached = await Task.detached(priority: .utility) { UIImage(contentsOfFile: file.path) }.value
            try? Task.checkCancellation()
            if !Task.isCancelled, cache.generation() == generation, let cached {
                cache.cache.setObject(cached, forKey: cacheKey, cost: Int(cached.size.width * cached.size.height * 4))
                return cached
            }
        }
        guard !Task.isCancelled, var components = URLComponents(string: session.server), components.host != nil,
              item.Id.range(of: "^[A-Za-z0-9_-]+$", options: .regularExpression) != nil else { return nil }
        let kind = backdrop ? "Backdrop/0" : "Primary"
        components.path = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).isEmpty ? "/Items/\(item.Id)/Images/\(kind)" : components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).appending("/Items/\(item.Id)/Images/\(kind)")
        if !components.path.hasPrefix("/") { components.path = "/" + components.path }
        components.queryItems = [URLQueryItem(name: "maxWidth", value: backdrop ? "1600" : "500"), URLQueryItem(name: "quality", value: "85")]
        guard let url = components.url else { return nil }
        var request = URLRequest(url: url)
        guard !session.token.isEmpty, !session.token.contains(where: { $0 == "\"" || $0 == "\\" || $0.asciiValue.map { $0 < 32 || $0 == 127 } == true }) else { return nil }
        request.setValue("MediaBrowser Client=\"Bluefin Apple\", Token=\"\(session.token)\"", forHTTPHeaderField: "Authorization")
        do {
            let (file, response) = try await cache.session.download(for: request, delegate: cache)
            defer { try? FileManager.default.removeItem(at: file) }
            guard let http = response as? HTTPURLResponse, http.statusCode == 200, http.mimeType?.hasPrefix("image/") == true,
                  response.expectedContentLength <= 8 * 1024 * 1024 else { return nil }
            try Task.checkCancellation()
            let decoded: UIImage? = await Task.detached(priority: .utility) {
                guard let size = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 8 * 1024 * 1024,
                      let source = CGImageSourceCreateWithURL(file as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
                      let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: backdrop ? 1600 : 500, kCGImageSourceCreateThumbnailWithTransform: true] as CFDictionary) else { return nil }
                return UIImage(cgImage: cg)
            }.value
            try Task.checkCancellation()
            guard let decoded else { return nil }
            guard cache.generation() == generation else { return nil }
            await Task.detached(priority: .utility) {
                if let data = decoded.jpegData(compressionQuality: 0.82) { cache.storeFile(data, key: identity, generation: generation) }
            }.value
            try Task.checkCancellation()
            guard cache.generation() == generation else { return nil }
            cache.cache.setObject(decoded, forKey: cacheKey, cost: Int(decoded.size.width * decoded.size.height * 4))
            return decoded
        } catch { return nil }
    }
    static func clearCache() { SameOriginImages.shared.clear() }
}
