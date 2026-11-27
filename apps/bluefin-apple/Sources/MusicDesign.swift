import SwiftUI

/// iOS content labels keep native text colors instead of inheriting link tint.
struct MusicContentLinkStyle: ViewModifier {
    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        content.buttonStyle(.plain)
        #else
        content
        #endif
    }
}

struct MusicCover: View {
    let item: MediaItem
    var radius: CGFloat = BluefinStyle.artworkRadius
    var body: some View {
        Artwork(item: item)
            .aspectRatio(1, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(.primary.opacity(0.06)))
    }
}

struct MusicShelfCard: View {
    let item: MediaItem
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            MusicCover(item: item)
            Text(item.title).font(.headline).lineLimit(2)
            if !item.subtitle.isEmpty { Text(item.subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(2) }
        }.accessibilityElement(children: .combine)
    }
}

struct MusicEmptyState: View {
    let title: String
    let message: String
    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "music.note.house").font(.largeTitle).foregroundStyle(.secondary).accessibilityHidden(true)
            Text(title).font(.title2.bold())
            Text(message).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }.frame(maxWidth: .infinity).padding(.vertical, 48).padding(.horizontal)
    }
}

struct MusicRow: View {
    let item: MediaItem
    var trackNumber: Int? = nil
    var body: some View {
        HStack(spacing: 14) {
            if let trackNumber {
                Text(String(trackNumber)).font(.subheadline.monospacedDigit()).foregroundStyle(.secondary).frame(minWidth: 24)
            } else { MusicCover(item: item, radius: 6).frame(width: 52, height: 52) }
            VStack(alignment: .leading, spacing: 4) {
                Text(item.title).font(.body.weight(.medium)).foregroundStyle(.primary).fixedSize(horizontal: false, vertical: true)
                if trackNumber == nil && !item.subtitle.isEmpty { Text(item.subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(2) }
            }
            Spacer(minLength: 8)
            if item.UserData?.IsFavorite == true { Image(systemName: "star.fill").font(.caption).foregroundStyle(BluefinStyle.accent).accessibilityLabel("Favorite") }
            if let ticks = item.RunTimeTicks, item.Type == "Audio" {
                Text(musicTime(Double(ticks) / 10_000_000)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
        }.padding(.vertical, 8).frame(minHeight: 44).contentShape(Rectangle()).accessibilityElement(children: .combine)
    }
}

/// Native tvOS button styles preserve Focus; iOS gets a quiet full-width action.
struct MusicPlayActions: View {
    var pageOnly: Bool
    var songCount: Int
    var disabled: Bool
    let play: (Bool) -> Void
    private var songLabel: String { "\(songCount) \(songCount == 1 ? "song" : "songs")" }
    var body: some View {
        HStack(spacing: 14) {
            Button { play(false) } label: { Label(pageOnly ? "Play \(songLabel)" : "Play", systemImage: "play.fill").frame(maxWidth: .infinity, minHeight: 44) }
            Button { play(true) } label: { Label(pageOnly ? "Shuffle \(songLabel)" : "Shuffle", systemImage: "shuffle").frame(maxWidth: .infinity, minHeight: 44) }
        }.buttonStyle(.bordered).tint(BluefinStyle.accent).disabled(disabled)
    }
}

struct MusicSearch: ViewModifier {
    @Binding var text: String
    var enabled: Bool
    var prompt: String
    @ViewBuilder func body(content: Content) -> some View {
        if enabled { content.searchable(text: $text, prompt: Text(prompt)) }
        else { content }
    }
}

struct MusicNavigationTitle: ViewModifier {
    var large: Bool
    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        content.navigationBarTitleDisplayMode(large ? .large : .inline)
        #else
        content
        #endif
    }
}
