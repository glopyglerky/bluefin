import Foundation

struct ArtworkSource: Hashable, Sendable {
    let id: String
    let kind: String
    let tag: String?
}

/// Selects artwork advertised by this item's owner without guessing unrelated IDs.
enum ArtworkPlan {
    static func sources(for item: MediaItem, backdrop: Bool) -> [ArtworkSource] {
        var result: [ArtworkSource] = []
        func append(_ id: String?, _ kind: String, _ tag: String?) {
            guard let id, !id.isEmpty, id.utf8.count <= 128,
                  id.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || $0 == 45 || $0 == 95 }),
                  let tag, !tag.isEmpty, tag.utf8.count <= 128,
                  !tag.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else { return }
            let source = ArtworkSource(id: id, kind: kind, tag: tag)
            if !result.contains(source) { result.append(source) }
        }
        if backdrop {
            append(item.Id, "Backdrop/0", item.BackdropImageTags?.first)
            append(item.ParentBackdropItemId, "Backdrop/0", item.ParentBackdropImageTags?.first)
            append(item.Id, "Thumb", item.ImageTags?["Thumb"])
            append(item.ParentThumbItemId, "Thumb", item.ParentThumbImageTag)
        } else if item.Type == "Episode" || item.Type == "Season" {
            // Episode stills are landscape; series posters fit the portrait shelf.
            append(item.SeriesId, "Primary", item.SeriesPrimaryImageTag)
        }
        append(item.Id, "Primary", item.ImageTags?["Primary"])
        if item.Type == "Audio" {
            append(item.AlbumId, "Primary", item.AlbumPrimaryImageTag)
        }
        append(item.ParentPrimaryImageItemId, "Primary", item.ParentPrimaryImageTag)
        if backdrop {
            append(item.SeriesId, "Primary", item.SeriesPrimaryImageTag)
        }
        if result.isEmpty {
            // Older servers may omit image tags. Probe only this exact item.
            result.append(ArtworkSource(id: item.Id, kind: "Primary", tag: nil))
        }
        return result
    }

    static func pixelWidth(_ requested: Int?, backdrop: Bool) -> Int {
        min(1920, max(96, requested ?? (backdrop ? 1600 : 500)))
    }
}
