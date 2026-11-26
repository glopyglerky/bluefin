import Foundation
import SwiftUI
#if canImport(FoundationModels) && !os(tvOS) && !os(watchOS)
import FoundationModels
#endif

enum LocalAssistancePlaybackStatus: String, Sendable { case buffering, paused, playing, stopped, failed }

struct LocalAssistanceTicket: Codable, Equatable, Sendable {
    let id: UInt64
    let generation: UInt64
    var fields: [String: Any] { ["id": id, "generation": generation] }
}

struct LocalAssistanceProvenance: Decodable, Sendable {
    let provider: String
    let on_device: Bool
    let no_tools: Bool
    let no_training: Bool
    let ephemeral: Bool
    var admitted: Bool {
        provider == "apple_system_language_model" && on_device && no_tools && no_training && ephemeral
    }
}

struct LocalAssistancePrepared: Decodable, Sendable {
    let ticket: LocalAssistanceTicket
    let instructions: String
    let prompt: String
    let maximum_response_tokens: Int
    let timeout_ms: UInt64
    let provenance: LocalAssistanceProvenance
}

struct LocalAssistanceSuggestion: Decodable, Sendable {
    let id: String
    let title: String
}

struct LocalAssistanceResult: Decodable, Sendable {
    let items: [LocalAssistanceSuggestion]
    let excerpts: [String]
    let source_item_id: String?
    let provenance: LocalAssistanceProvenance
}

/// Pure policy seam can be tested without implying hardware model availability.
enum LocalAssistanceAvailability {
    static func admitted(systemSupported: Bool, modelReady: Bool, localeSupported: Bool) -> Bool {
        systemSupported && modelReady && localeSupported
    }

    static var isAvailable: Bool {
        #if canImport(FoundationModels) && !os(tvOS) && !os(watchOS)
        if #available(iOS 26.0, macOS 26.0, visionOS 26.0, *) {
            let model = SystemLanguageModel.default
            let ready: Bool
            switch model.availability {
            case .available: ready = true
            case .unavailable: ready = false
            @unknown default: ready = false
            }
            return admitted(systemSupported: true, modelReady: ready, localeSupported: model.supportsLocale())
        }
        #endif
        return false
    }
}

/// Cancellation does not release a real in-flight permit. Only the completion
/// of the framework call releases it, preventing abandoned inference buildup.
@MainActor final class LocalAssistancePermit {
    private(set) var active: LocalAssistanceTicket?
    private(set) var cancellationRequested = false

    func acquire(_ ticket: LocalAssistanceTicket) -> Bool {
        guard active == nil else { return false }
        active = ticket
        cancellationRequested = false
        return true
    }

    func requestCancellation(_ ticket: LocalAssistanceTicket) {
        if active == ticket { cancellationRequested = true }
    }

    func finished(_ ticket: LocalAssistanceTicket) {
        guard active == ticket else { return }
        active = nil
        cancellationRequested = false
    }
}

private enum LocalAssistanceFailure: Error {
    case unavailable, busy, invalid
}

#if canImport(FoundationModels) && !os(tvOS) && !os(watchOS)
@available(iOS 26.0, macOS 26.0, visionOS 26.0, *)
@Generable
private struct LocalAssistanceSelection {
    @Guide(description: "Supplied catalog slots, or empty for sentence selection.", .maximumCount(5))
    var items: [String]
    @Guide(description: "Supplied sentence indices in source order, or empty for catalog selection.", .maximumCount(3))
    var sentences: [Int]
}
#endif

@MainActor private enum LocalAssistanceRunner {
    static let permit = LocalAssistancePermit()

    static func requestCancellation(_ ticket: LocalAssistanceTicket) {
        permit.requestCancellation(ticket)
    }

    static func generate(_ request: LocalAssistancePrepared) async throws -> String {
        guard request.provenance.admitted,
              request.instructions.utf8.count + request.prompt.utf8.count <= 8192,
              (1...256).contains(request.maximum_response_tokens),
              request.timeout_ms <= 20_000,
              LocalAssistanceAvailability.isAvailable else {
            throw LocalAssistanceFailure.unavailable
        }
        try Task.checkCancellation()
        guard permit.acquire(request.ticket) else { throw LocalAssistanceFailure.busy }
        defer { permit.finished(request.ticket) }
        #if canImport(FoundationModels) && !os(tvOS) && !os(watchOS)
        if #available(iOS 26.0, macOS 26.0, visionOS 26.0, *) {
            // This exact provider is on-device. No tools, custom adapters,
            // transcript reuse, Private Cloud Compute or remote provider exists.
            let model = SystemLanguageModel.default
            guard model.supportsLocale() else { throw LocalAssistanceFailure.unavailable }
            let session = LanguageModelSession(model: model, tools: []) {
                request.instructions
            }
            let response = try await session.respond(
                to: request.prompt,
                generating: LocalAssistanceSelection.self,
                options: GenerationOptions(temperature: 0.0, maximumResponseTokens: request.maximum_response_tokens)
            )
            try Task.checkCancellation()
            guard !permit.cancellationRequested,
                  response.content.items.count <= 5,
                  response.content.items.allSatisfy({ $0.utf8.count <= 16 }),
                  response.content.sentences.count <= 3,
                  response.content.sentences.allSatisfy({ (0..<24).contains($0) }) else {
                throw LocalAssistanceFailure.invalid
            }
            let data = try JSONSerialization.data(withJSONObject: [
                "items": response.content.items,
                "sentences": response.content.sentences
            ])
            guard data.count <= 2048, let output = String(data: data, encoding: .utf8) else {
                throw LocalAssistanceFailure.invalid
            }
            return output
        }
        #endif
        throw LocalAssistanceFailure.unavailable
    }
}

@MainActor final class LocalAssistanceModel: ObservableObject {
    @Published private(set) var result: LocalAssistanceResult?
    @Published private(set) var message: String?
    @Published private(set) var busy = false
    private var task: Task<Void, Never>?
    private var watchdog: Task<Void, Never>?
    private var requestID: UUID?
    private var ticket: LocalAssistanceTicket?
    private var context: String?

    func start(input: [String: Any], context: UUID, isCurrent: @escaping @MainActor () -> Bool) {
        guard task == nil, LocalAssistanceAvailability.isAvailable else { return }
        let requestID = UUID()
        self.requestID = requestID
        self.context = context.uuidString
        self.result = nil
        self.message = nil
        self.busy = true
        task = Task { [weak self] in
            guard let self else { return }
            defer {
                if self.requestID == requestID {
                    self.watchdog?.cancel()
                    self.watchdog = nil
                    self.ticket = nil
                    self.context = nil
                    self.task = nil
                    self.requestID = nil
                    self.busy = false
                }
            }
            do {
                let prepared: LocalAssistancePrepared = try await RustBridge.call(
                    "localAssistPrepare",
                    fields: ["context_generation": context.uuidString, "input": input]
                )
                guard prepared.provenance.admitted, prepared.timeout_ms > 0, prepared.timeout_ms <= 20_000 else {
                    throw LocalAssistanceFailure.invalid
                }
                self.ticket = prepared.ticket
                guard self.requestID == requestID, isCurrent(), !Task.isCancelled else {
                    self.cancel()
                    return
                }
                self.watchdog = Task { [weak self] in
                    try? await Task.sleep(nanoseconds: prepared.timeout_ms * 1_000_000)
                    guard !Task.isCancelled, let self, self.requestID == requestID else { return }
                    self.cancel(timedOut: true)
                }
                let output = try await LocalAssistanceRunner.generate(prepared)
                try Task.checkCancellation()
                guard self.requestID == requestID, isCurrent() else { throw CancellationError() }
                let completed: LocalAssistanceResult = try await RustBridge.call(
                    "localAssistComplete",
                    fields: ["context_generation": context.uuidString,
                             "ticket": prepared.ticket.fields, "model_output": output]
                )
                try Task.checkCancellation()
                guard self.requestID == requestID, isCurrent(), completed.provenance.admitted else {
                    throw CancellationError()
                }
                self.result = completed
                if completed.items.isEmpty && completed.excerpts.isEmpty {
                    self.message = "No supported suggestion in this supplied text."
                }
            } catch is CancellationError {
                self.discardTicket()
            } catch {
                self.discardTicket()
                // Framework errors may contain input/transcript details.
                // Keep all model errors, prompts and outputs out of app logs.
                if isCurrent(), self.message == nil {
                    self.message = "Local assistance is unavailable for this request. You can keep using the normal controls."
                }
            }
        }
    }

    func cancel(timedOut: Bool = false) {
        guard task != nil else { result = nil; message = nil; return }
        result = nil
        message = timedOut ? "Local assistance took too long. Stopping this request." : "Stopping local assistance."
        if let ticket { LocalAssistanceRunner.requestCancellation(ticket) }
        task?.cancel()
        discardTicket()
        // busy/task stay retained until the actual generation call returns.
    }

    private func discardTicket() {
        guard let ticket, let context else { return }
        self.ticket = nil
        Task {
            let _: EmptyValue? = try? await RustBridge.call(
                "localAssistCancel", fields: ["context_generation": context, "ticket": ticket.fields]
            )
        }
    }
}

/// Only these fields from an already authorized MediaItem can be model input.
enum LocalAssistanceInput {
    static func catalog(query: String, items: [MediaItem]) -> [String: Any] {
        var seen = Set<String>()
        let selected = items.filter { seen.insert($0.Id).inserted }.prefix(12)
        return ["task": "catalog", "query": query,
                "items": selected.map { itemFields($0, descriptionLimit: 320) }]
    }

    static func summary(item: MediaItem, description: String) -> [String: Any] {
        var fields = itemFields(item, descriptionLimit: 0)
        // Summary uses the exact supplied description or Rust refuses the bound.
        fields["description"] = description
        return ["task": "summary", "item": fields]
    }

    static func playback(status: LocalAssistancePlaybackStatus) -> [String: Any] {
        ["task": "playback", "status": status.rawValue]
    }

    private static func itemFields(_ item: MediaItem, descriptionLimit: Int) -> [String: Any] {
        var fields: [String: Any] = [
            "id": item.Id,
            "title": boundedPrefix(item.title, bytes: 200),
            "description": boundedPrefix(item.Overview ?? "", bytes: descriptionLimit)
        ]
        if let year = item.ProductionYear, (1800...2500).contains(year) { fields["year"] = year }
        if let ticks = item.RunTimeTicks, ticks > 0 {
            let minutes = ticks / 600_000_000
            if (1...1440).contains(minutes) { fields["minutes"] = minutes }
        }
        return fields
    }

    private static func boundedPrefix(_ value: String, bytes: Int) -> String {
        guard bytes > 0 else { return "" }
        var text = ""
        for character in value {
            let next = String(character)
            if text.utf8.count + next.utf8.count > bytes { break }
            text += next
        }
        return text
    }
}
