import SwiftUI
import Foundation

private struct NoticePresentation: Sendable {
    let entries: [NoticeComponent]
    let provenance: [String: [String]]
}

struct NativeNoticeList: View {
    let store: NoticeStore
    let indexName: String?
    let target: String?
    @State private var loaded: Result<NoticePresentation, Error>?

    init(store: NoticeStore, indexName: String?, target: String?) {
        self.store = store
        self.indexName = indexName
        self.target = target
    }

    var body: some View {
        List {
            switch loaded {
            case nil: ProgressView("Loading licenses")
            case .failure: Text("Software licenses could not be opened. Use Back to return.")
            case .success(let value):
                ForEach(value.entries) { entry in
                    NavigationLink(destination: component(entry, provenance: value.provenance)) { Text(entry.title) }
                }
            }
        }
        .navigationTitle("Software licenses")
        .task {
            let model = store, index = indexName, selectedTarget = target
            let result = await Task.detached(priority: .utility) {
                Result { NoticePresentation(entries: try model.components(indexName: index, target: selectedTarget), provenance: model.provenance()) }
            }.value
            guard !Task.isCancelled else { return }
            loaded = result
        }
    }

    private func component(_ entry: NoticeComponent, provenance: [String: [String]]) -> some View {
        List {
            if !entry.source.isEmpty { Text("Source reference: " + entry.source).font(.footnote) }
            ForEach(entry.resources, id: \.self) { resource in
                let title = store.label(for: resource, in: entry, provenance: provenance)
                NavigationLink(destination: NativeNoticeDocument(url: resource, title: title)) { Text(title) }
            }
            if entry.resources.isEmpty { Text("No local text is available for this component.") }
        }.navigationTitle(entry.title)
    }
}

struct NativeNoticeDocument: View {
    let url: URL
    let title: String
    @State private var loaded: Result<String, Error>?
    var body: some View {
        Group {
            switch loaded {
            case nil: ProgressView("Opening notice")
            case .failure: Text("This local notice could not be opened. Use Back to return.")
            case .success(let text):
                #if os(watchOS)
                let paragraphs = text.components(separatedBy: "\n\n")
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        ForEach(paragraphs.indices, id: \.self) { index in Text(paragraphs[index]).font(.body) }
                    }.padding()
                }
                #else
                NativeLegalText(text: text, html: url.pathExtension.lowercased() == "html")
                #endif
            }
        }
        .navigationTitle(title)
        .task(id: url) {
            let location = url
            let result = await Task.detached(priority: .utility) { Result { try String(contentsOf: location, encoding: .utf8) } }.value
            guard !Task.isCancelled else { return }
            loaded = result
        }
    }
}
