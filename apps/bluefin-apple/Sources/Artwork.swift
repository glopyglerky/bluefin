import SwiftUI
import ImageIO

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
    private var key: String { "\(store.session?.server ?? "preview")/\(store.session?.userId ?? "")/\(item.Id)/\(backdrop)/\(item.ImageTags?["Primary"] ?? "")" }
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
        let cache = SameOriginImages.shared
        let cacheKey = key as NSString
        if let cached = cache.cache.object(forKey: cacheKey) { image = cached; return }
        guard var components = URLComponents(string: session.server), components.host != nil,
              item.Id.range(of: "^[A-Za-z0-9_-]+$", options: .regularExpression) != nil else { return }
        let kind = backdrop ? "Backdrop/0" : "Primary"
        components.path = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).isEmpty ? "/Items/\(item.Id)/Images/\(kind)" : components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).appending("/Items/\(item.Id)/Images/\(kind)")
        if !components.path.hasPrefix("/") { components.path = "/" + components.path }
        components.queryItems = [URLQueryItem(name: "maxWidth", value: backdrop ? "1600" : "500"), URLQueryItem(name: "quality", value: "85")]
        guard let url = components.url else { return }
        var request = URLRequest(url: url)
        request.setValue(session.token, forHTTPHeaderField: "X-Emby-Token")
        do {
            let (file, response) = try await cache.session.download(for: request, delegate: cache)
            defer { try? FileManager.default.removeItem(at: file) }
            guard let http = response as? HTTPURLResponse, http.statusCode == 200, http.mimeType?.hasPrefix("image/") == true,
                  response.expectedContentLength <= 8 * 1024 * 1024 else { return }
            try Task.checkCancellation()
            let decoded: UIImage? = await Task.detached(priority: .utility) {
                guard let size = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 8 * 1024 * 1024,
                      let source = CGImageSourceCreateWithURL(file as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
                      let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: backdrop ? 1600 : 500, kCGImageSourceCreateThumbnailWithTransform: true] as CFDictionary) else { return nil }
                return UIImage(cgImage: cg)
            }.value
            try Task.checkCancellation()
            guard let decoded else { return }
            cache.cache.setObject(decoded, forKey: cacheKey, cost: Int(decoded.size.width * decoded.size.height * 4))
            image = decoded
        } catch {}
    }
    static func clearCache() { SameOriginImages.shared.cache.removeAllObjects() }
}
