import SwiftUI

/// Lightweight page positions remain after records are evicted, so scrolling back
/// rehydrates the same slots without removing rows or changing their identities.
struct MediaPageWindow {
    struct Page: Identifiable {
        let start: Int
        var count: Int
        var items: [MediaItem]?
        var id: Int { start }
        var range: Range<Int> { start..<(start + count) }
    }
    static let pageSize = 60
    static let cachedPageLimit = 5
    private(set) var pages: [Page] = []
    private var positions: [Int: Int] = [:]
    private var cachedPositions = Set<Int>()
    private(set) var nextIndex = 0
    private(set) var total = 0
    var hasMore: Bool { nextIndex < total }
    var cachedPageCount: Int { cachedPositions.count }
    var cachedItemCount: Int { cachedPositions.reduce(0) { $0 + (pages[$1].items?.count ?? 0) } }
    var items: [MediaItem] {
        var ids = Set<String>()
        return cachedPositions.sorted().flatMap { pages[$0].items ?? [] }.filter { ids.insert($0.Id).inserted }
    }
    private func pagePosition(containing index: Int) -> Int? {
        // Pages are ordered. Binary search avoids scanning the scroll history.
        var lower = 0, upper = pages.count
        while lower < upper {
            let middle = (lower + upper) / 2
            if pages[middle].start <= index { lower = middle + 1 } else { upper = middle }
        }
        guard lower > 0, pages[lower - 1].range.contains(index) else { return nil }
        return lower - 1
    }
    func page(containing index: Int) -> Page? {
        pagePosition(containing: index).map { pages[$0] }
    }
    func page(startingAt start: Int) -> Page? {
        positions[start].map { pages[$0] }
    }
    func nearbyStarts(around index: Int) -> Set<Int> {
        guard !pages.isEmpty else { return [] }
        let pivot = pagePosition(containing: index) ?? pages.count - 1
        let nearby = max(0, pivot - Self.cachedPageLimit)..<min(pages.count, pivot + Self.cachedPageLimit + 1)
        return Set(nearby.sorted {
            abs(pages[$0].start - index) < abs(pages[$1].start - index)
        }.prefix(Self.cachedPageLimit).map { pages[$0].start })
    }
    func item(at index: Int) -> MediaItem? {
        guard let page = page(containing: index), let items = page.items else { return nil }
        let offset = index - page.start
        return items.indices.contains(offset) ? items[offset] : nil
    }
    func isDuplicate(at index: Int) -> Bool {
        guard let item = item(at: index) else { return false }
        return cachedPositions.contains { position in
            let page = pages[position]
            guard page.start <= index, let values = page.items else { return false }
            return values.prefix(max(0, index - page.start)).contains { $0.Id == item.Id }
        }
    }
    mutating func append(_ result: ItemPage) { install(result, at: nextIndex) }
    mutating func install(_ result: ItemPage, at start: Int) {
        if let position = positions[start] {
            // Preserve the old page's geometry if the catalog changed while away.
            pages[position].items = Array(result.Items.prefix(pages[position].count))
            cachedPositions.insert(position)
        } else if start == nextIndex {
            if !result.Items.isEmpty {
                positions[start] = pages.count
                cachedPositions.insert(pages.count)
                pages.append(.init(start: start, count: result.Items.count, items: result.Items))
                nextIndex += result.Items.count
            }
            total = result.Items.isEmpty ? nextIndex : max(nextIndex, result.TotalRecordCount ?? nextIndex)
        }
    }
    mutating func trim(around index: Int, protecting visible: Set<Int> = []) {
        let keepCount = max(Self.cachedPageLimit, visible.count)
        let cached = cachedPositions
        let keep = Set(cached.sorted {
            let leftProtected = visible.contains(pages[$0].start)
            let rightProtected = visible.contains(pages[$1].start)
            if leftProtected != rightProtected { return leftProtected }
            return abs(pages[$0].start - index) < abs(pages[$1].start - index)
        }.prefix(keepCount))
        for position in cached where !keep.contains(position) {
            pages[position].items = nil
            cachedPositions.remove(position)
        }
    }
}

@MainActor final class PagedMediaModel: ObservableObject {
    typealias Loader = @MainActor (Int, Int) async throws -> ItemPage
    @Published private(set) var window = MediaPageWindow()
    @Published private(set) var failures: [Int: String] = [:]
    @Published private(set) var loadingOffsets = Set<Int>()
    private var loader: Loader?
    private var identity: AnyHashable?
    private var generation = UUID()
    private struct Pending { let id: UUID; let task: Task<Void, Never> }
    private var tasks: [Int: Pending] = [:]
    private var visibleIndices = Set<Int>()
    private var anchor = 0
    var loading: Bool { !loadingOffsets.isEmpty }
    var items: [MediaItem] { window.items }
    var problem: String? { failures.keys.sorted().first.flatMap { failures[$0] } }

    func start(identity: AnyHashable? = nil, loader: @escaping Loader) async {
        if let identity, self.identity == identity, window.nextIndex > 0 {
            self.loader = loader
            return
        }
        cancel()
        self.identity = identity
        self.loader = loader
        window = .init(); failures = [:]; visibleIndices = []; anchor = 0
        await fetch(start: 0, limit: MediaPageWindow.pageSize, generation: generation)
    }

    func appear(_ index: Int) {
        visibleIndices.insert(index); anchor = index
        guard let page = window.page(containing: index) else { return }
        if page.items == nil { request(start: page.start, limit: page.count) }
        if index >= window.nextIndex - 12, window.hasMore { loadNext() }
        if index - page.start < 8, let previous = window.page(containing: page.start - 1), previous.items == nil {
            request(start: previous.start, limit: previous.count)
        }
        trim()
    }

    func disappear(_ index: Int) {
        visibleIndices.remove(index)
        trim()
    }

    func loadNext() {
        guard window.hasMore else { return }
        request(start: window.nextIndex, limit: MediaPageWindow.pageSize)
    }

    func retry(_ start: Int) {
        failures[start] = nil
        request(start: start, limit: window.page(startingAt: start)?.count ?? MediaPageWindow.pageSize)
    }

    func cancel() {
        generation = UUID()
        tasks.values.forEach { $0.task.cancel() }; tasks = [:]
        loadingOffsets = []
        visibleIndices = []
    }

    private func request(start: Int, limit: Int) {
        guard loader != nil, !loadingOffsets.contains(start), tasks[start] == nil, failures[start] == nil else { return }
        let current = generation, identity = UUID()
        let task = Task { [weak self] in
            guard let self else { return }
            await fetch(start: start, limit: limit, generation: current)
            guard generation == current, tasks[start]?.id == identity else { return }
            tasks[start] = nil
            // Cancellation can finish after the user scrolls back. Requeue the
            // still-visible missing page after its old request has released it.
            if let page = window.page(startingAt: start), page.items == nil,
               visibleIndices.contains(where: { page.range.contains($0) }), failures[start] == nil {
                request(start: start, limit: page.count)
            }
        }
        tasks[start] = .init(id: identity, task: task)
    }

    private func fetch(start: Int, limit: Int, generation current: UUID) async {
        guard generation == current, let loader, !loadingOffsets.contains(start) else { return }
        loadingOffsets.insert(start)
        defer { if generation == current { loadingOffsets.remove(start) } }
        do {
            let result = try await loader(start, limit)
            try Task.checkCancellation()
            guard generation == current else { return }
            window.install(result, at: start)
            failures[start] = nil
            trim()
        } catch is CancellationError {} catch {
            guard generation == current, !Task.isCancelled else { return }
            failures[start] = error.localizedDescription
        }
    }

    private func trim() {
        let protected = Set(visibleIndices.compactMap { window.page(containing: $0)?.start })
        window.trim(around: anchor, protecting: protected)
        // A fast jump must not leave distant image/metadata requests accumulating.
        let nearby = window.nearbyStarts(around: anchor).union(protected)
        for (start, pending) in tasks where start < window.nextIndex && !nearby.contains(start) {
            pending.task.cancel()
        }
    }
}

struct MediaLoadingRow: View {
    var message = "Loading"
    var retry: (() -> Void)?
    var body: some View {
        HStack(spacing: 12) {
            if let retry { Button("Try again", action: retry) } else { ProgressView() }
            Text(message).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
            Spacer()
        }.accessibilityElement(children: .combine)
    }
}
