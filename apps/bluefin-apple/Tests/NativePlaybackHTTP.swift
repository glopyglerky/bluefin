import Foundation
import Network

/// Task-owned, bounded HTTP/1.1 fixture. Each connection serves one request and closes.
// Mutable server state is confined to queue. origin is published by start's continuation;
// onHeldDownload is configured before downloadStart and never changed during a request.
final class NativePlaybackHTTP: @unchecked Sendable {
    private final class Readiness: @unchecked Sendable { var completed = false }
    struct Request { let method: String; let path: String; let headers: [String: String]; let body: Data }
    struct Response {
        let bytes: Data
        var status = "200 OK"
        var type = "application/json"
    }
    let token = "native-test-only-token"
    let queue = DispatchQueue(label: "bluefin.tests.loopback")
    private let listener: NWListener
    private let video: Data
    private let audio: Data
    private var connections: [ObjectIdentifier: NWConnection] = [:]
    private var recorded: [Request] = []
    private var held: NWConnection?
    var onHeldDownload: (() -> Void)?
    /// Configured before start. Both the interceptor and its completion run on queue.
    var intercept: ((Request, @escaping (Response) -> Void) -> Bool)?
    var publicPaths: Set<String> = []
    private(set) var origin = ""

    init(files: NativePlaybackFixtures.Files) throws {
        video = try Data(contentsOf: files.video); audio = try Data(contentsOf: files.audio)
        guard video.count < 1_048_576, audio.count < 1_048_576 else {
            throw NativePlaybackFixtures.Failure.invalid("Fixture exceeds 1 MiB")
        }
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        listener = try NWListener(using: parameters)
    }

    func start() async throws {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            let readiness = Readiness()
            let deadline = DispatchWorkItem {
                guard !readiness.completed else { return }; readiness.completed = true
                self.listener.cancel()
                c.resume(throwing: NativePlaybackFixtures.Failure.invalid("HTTP listener deadline"))
            }
            queue.asyncAfter(deadline: .now() + 5, execute: deadline)
            listener.stateUpdateHandler = { state in
                guard !readiness.completed else { return }
                switch state {
                case .ready:
                    readiness.completed = true; deadline.cancel()
                    self.origin = "http://127.0.0.1:\(self.listener.port!.rawValue)"
                    c.resume()
                case .failed(let error): readiness.completed = true; deadline.cancel(); c.resume(throwing: error)
                default: break
                }
            }
            listener.newConnectionHandler = { [weak self] connection in
                guard let self else { connection.cancel(); return }
                guard self.connections.count < 16 else { connection.cancel(); return }
                self.connections[ObjectIdentifier(connection)] = connection
                connection.stateUpdateHandler = { [weak self, weak connection] state in
                    if case .cancelled = state, let connection {
                        self?.connections.removeValue(forKey: ObjectIdentifier(connection))
                    }
                }
                connection.start(queue: self.queue)
                self.read(connection, data: Data())
            }
            listener.start(queue: queue)
        }
    }

    func stop() async {
        await withCheckedContinuation { c in queue.async {
            self.listener.cancel()
            for connection in self.connections.values { connection.cancel() }
            self.connections.removeAll(); self.held = nil
            c.resume()
        } }
    }
    func requests() async -> [Request] {
        await withCheckedContinuation { c in queue.async { c.resume(returning: self.recorded) } }
    }
    private func read(_ connection: NWConnection, data: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 16_384) { [weak self] bytes, _, done, error in
            guard let self else { return }
            var data = data; data.append(bytes ?? Data())
            guard data.count <= 131_072 else { connection.cancel(); return }
            if let boundary = data.range(of: Data("\r\n\r\n".utf8)),
               let header = String(data: data[..<boundary.lowerBound], encoding: .utf8) {
                let lines = header.components(separatedBy: "\r\n")
                let first = lines[0].split(separator: " ")
                guard first.count == 3 else { connection.cancel(); return }
                var headers: [String: String] = [:]
                for line in lines.dropFirst() {
                    let parts = line.split(separator: ":", maxSplits: 1)
                    if parts.count == 2 { headers[String(parts[0]).lowercased()] = parts[1].trimmingCharacters(in: .whitespaces) }
                }
                guard let count = Int(headers["content-length"] ?? "0"), count >= 0, count <= 65_536 else {
                    connection.cancel(); return
                }
                if data.count >= boundary.upperBound + count {
                    let request = Request(method: String(first[0]), path: String(first[1]), headers: headers,
                                          body: data.subdata(in: boundary.upperBound..<(boundary.upperBound + count)))
                    self.recorded.append(request); self.respond(request, connection); return
                }
            }
            if done || error != nil { connection.cancel() }
            else { self.read(connection, data: data) }
        }
    }
    private func json(_ value: Any, _ connection: NWConnection) {
        do { send(try JSONSerialization.data(withJSONObject: value), type: "application/json", connection: connection) }
        catch { send(Data(), status: "500 Fixture encoding failed", connection: connection) }
    }
    private func respond(_ request: Request, _ connection: NWConnection) {
        guard publicPaths.contains(request.path) || request.headers["authorization"]?.contains(token) == true,
              !request.path.lowercased().contains(token) else {
            send(Data(), status: "401 Unauthorized", connection: connection); return
        }
        if intercept?(request, { [weak self, weak connection] response in
            guard let self, let connection else { return }
            self.send(response.bytes, status: response.status, type: response.type, connection: connection)
        }) == true { return }
        let path = request.path.components(separatedBy: "?")[0]
        let parts = path.split(separator: "/").map(String.init)
        if path == "/Users/Me" {
            json(["Id": "native-user", "Policy": ["EnableContentDownloading": true]], connection)
        } else if path == "/MediaSegments/video" {
            json(["Items": [
                ["ItemId": "video", "Type": "Intro", "StartTicks": 4_000_000, "EndTicks": 16_000_000],
                ["ItemId": "video", "Type": 3, "StartTicks": 16_000_000, "EndTicks": 19_000_000],
                ["ItemId": "video", "Type": "Commercial", "StartTicks": 19_000_000, "EndTicks": 22_000_000],
                ["ItemId": "video", "Type": "Outro", "StartTicks": 22_000_000, "EndTicks": 25_000_000],
                ["ItemId": "different-video", "Type": "Intro", "StartTicks": 0, "EndTicks": 8_000_000]
            ]], connection)
        } else if parts.count == 4 && parts[0] == "Users" && parts[2] == "Items" {
            json(["Id": parts[3], "Name": "Original synthetic tone", "Type": "Audio", "MediaType": "Audio"], connection)
        } else if parts.count == 3 && parts[0] == "Items" && parts[2] == "PlaybackInfo" {
            let id = parts[1]; let isVideo = id == "video"
            var source: [String: Any] = ["Id": id, "Container": isVideo ? "mp4" : "m4a",
                "Protocol": "File", "SupportsDirectPlay": true, "Size": isVideo ? video.count : audio.count]
            source["MediaStreams"] = isVideo ? [["Type": "Video", "Codec": "h264", "Profile": "Baseline",
                "Level": 30, "VideoRange": "SDR", "BitDepth": 8, "Width": 160, "Height": 90]] :
                [["Type": "Audio", "Codec": "alac", "BitDepth": 16, "SampleRate": 44100, "Channels": 1]]
            json(["PlaySessionId": "native-session-\(id)", "MediaSources": [source]], connection)
        } else if path.hasPrefix("/Sessions/Playing") {
            send(Data(), status: "204 No Content", connection: connection)
        } else if ["/Items/audio/Download", "/Items/cancel/Download",
                   "/Videos/video/stream.mp4", "/Audio/audio/stream.m4a"].contains(path),
                  ["HEAD", "GET"].contains(request.method) {
            let bytes = parts[1] == "video" ? video : audio
            let type = parts[1] == "video" ? "video/mp4" : "audio/mp4"
            if parts[1] == "cancel" && parts[2] == "Download" {
                // Send headers and a prefix, then await cancellation. No sleep or huge payload.
                held = connection
                let header = "HTTP/1.1 200 OK\r\nContent-Length: \(bytes.count)\r\nContent-Type: \(type)\r\nConnection: close\r\n\r\n"
                connection.send(content: Data(header.utf8) + bytes.prefix(16), completion: .contentProcessed { error in
                    if error == nil { self.onHeldDownload?() }
                })
                return
            }
            var lo = 0; var hi = bytes.count - 1; var status = "200 OK"
            var extra = "Accept-Ranges: bytes\r\n"
            if let range = request.headers["range"] {
                guard range.hasPrefix("bytes="), !range.contains(",") else {
                    send(Data(), status: "416 Range Not Satisfiable", connection: connection); return
                }
                let bounds = range.dropFirst(6).split(separator: "-", omittingEmptySubsequences: false)
                guard bounds.count == 2 else { connection.cancel(); return }
                if bounds[0].isEmpty, let suffix = Int(bounds[1]), suffix > 0 { lo = max(0, bytes.count - suffix) }
                else if let start = Int(bounds[0]) {
                    lo = start
                    if !bounds[1].isEmpty { hi = min(Int(bounds[1]) ?? -1, hi) }
                } else { connection.cancel(); return }
                guard lo >= 0, lo <= hi, hi < bytes.count else {
                    send(Data(), status: "416 Range Not Satisfiable", connection: connection); return
                }
                status = "206 Partial Content"; extra += "Content-Range: bytes \(lo)-\(hi)/\(bytes.count)\r\n"
            }
            send(bytes.subdata(in: lo..<(hi + 1)), status: status, type: type,
                 head: request.method == "HEAD", extra: extra, connection: connection)
        } else { send(Data(), status: "404 Not Found", connection: connection) }
    }
    private func send(_ data: Data, status: String = "200 OK", type: String = "application/octet-stream",
                      head: Bool = false, extra: String = "", connection: NWConnection) {
        let header = "HTTP/1.1 \(status)\r\nContent-Length: \(data.count)\r\nContent-Type: \(type)\r\n\(extra)Connection: close\r\n\r\n"
        connection.send(content: Data(header.utf8) + (head ? Data() : data),
                        completion: .contentProcessed { _ in connection.cancel() })
    }
}
