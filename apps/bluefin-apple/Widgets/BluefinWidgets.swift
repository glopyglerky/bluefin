import SwiftUI
import WidgetKit

private struct QuickAccessEntry: TimelineEntry {
    let date: Date
    let status: WidgetSnapshot.Status
}

private struct QuickAccessProvider: TimelineProvider {
    func placeholder(in context: Context) -> QuickAccessEntry { .init(date: Date(), status: .unavailable) }
    func getSnapshot(in context: Context, completion: @escaping (QuickAccessEntry) -> Void) {
        completion(entry(at: Date()))
    }
    func getTimeline(in context: Context, completion: @escaping (Timeline<QuickAccessEntry>) -> Void) {
        let now = Date()
        let group = Bundle.main.object(forInfoDictionaryKey: "BluefinWidgetAppGroup") as? String
        let state = WidgetSnapshotReader.presentation(io: WidgetSnapshotFile(configuredGroup: group), now: now)
        var entries = [QuickAccessEntry(date: now, status: state.status)]
        if let expiry = state.expiresAt { entries.append(.init(date: expiry, status: .stale)) }
        completion(Timeline(entries: entries, policy: .after(state.expiresAt ?? now.addingTimeInterval(900))))
    }
    private func entry(at now: Date) -> QuickAccessEntry {
        let group = Bundle.main.object(forInfoDictionaryKey: "BluefinWidgetAppGroup") as? String
        return .init(date: now, status: WidgetSnapshotReader.status(io: WidgetSnapshotFile(configuredGroup: group), now: now))
    }
}

private struct QuickAccessView: View {
    @Environment(\.widgetFamily) private var family
    @Environment(\.colorScheme) private var scheme
    @Environment(\.widgetRenderingMode) private var renderingMode
    let entry: QuickAccessEntry

    private func color(_ light: UInt32, _ dark: UInt32) -> Color {
        let hex = scheme == .dark ? dark : light
        return Color(red: Double((hex >> 16) & 255) / 255,
                     green: Double((hex >> 8) & 255) / 255,
                     blue: Double(hex & 255) / 255)
    }
    private var background: Color { color(0xA3ACB1, 0x061727) }
    private var text: Color { color(0x061727, 0xCCD0D3) }
    private var accent: Color { color(0x333B5A, 0xADB5D1) }
    private var statusText: String {
        switch entry.status {
        case .signedOut: "Sign in on your iPhone"
        case .unavailable: "Open Bluefin on your iPhone"
        case .stale: "Status needs refresh"
        case .current: "Browse in Bluefin"
        }
    }
    var body: some View {
        Group {
            switch family {
            case .accessoryCircular:
                Image(systemName: WidgetRoute.library.symbol).accessibilityLabel("Open Bluefin Library")
            case .accessoryInline:
                Label("Bluefin Library", systemImage: WidgetRoute.library.symbol)
            case .accessoryRectangular:
                VStack(alignment: .leading) {
                    Text("Bluefin Library").font(.headline)
                    Text(statusText).font(.caption).privacySensitive()
                }
            case .systemSmall:
                VStack(alignment: .leading, spacing: 8) {
                    Image(systemName: WidgetRoute.library.symbol).font(.title)
                    Text("Bluefin Library").font(.headline)
                    Text(statusText).font(.caption).privacySensitive()
                }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            default:
                VStack(alignment: .leading, spacing: 12) {
                    Text("Bluefin").font(.headline)
                    Text(statusText).font(.caption).foregroundStyle(.secondary).privacySensitive()
                    if family == .systemLarge {
                        ForEach(WidgetRoute.allCases, id: \.rawValue) { route in routeLink(route) }
                    } else {
                        HStack { routeLink(.library); routeLink(.music) }
                        HStack { routeLink(.liveTV); routeLink(.search) }
                    }
                }
            }
        }
        .widgetURL(WidgetRoute.library.url)
        .foregroundStyle(renderingMode == .fullColor ? text : Color.primary)
        .tint(accent)
        .containerBackground(renderingMode == .fullColor ? background : Color.clear, for: .widget)
    }
    private func routeLink(_ route: WidgetRoute) -> some View {
        Link(destination: route.url) {
            Label(route.title, systemImage: route.symbol).font(.subheadline)
                .frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4)
        }.accessibilityLabel("Open Bluefin " + route.title)
    }
}

@main struct BluefinQuickAccessWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "BluefinQuickAccess", provider: QuickAccessProvider()) { entry in
            QuickAccessView(entry: entry)
        }
        .configurationDisplayName("Bluefin quick access")
        .description("Open Library, Music, Live TV or Search on your iPhone. Personal media stays in Bluefin.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge, .accessoryCircular, .accessoryRectangular, .accessoryInline])
        // Navigation requires the native iPhone scene. Do not offer broken projection controls in a vehicle.
        .disfavoredLocations([.carPlay], for: [.systemSmall])
    }
}
