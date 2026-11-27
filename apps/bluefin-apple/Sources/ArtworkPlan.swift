import Foundation

struct ArtworkSource: Hashable, Sendable {
    let id: String
    let kind: String
    let tag: String?
}

/// Selects art for the title represented by a card, retaining playback's item ID.
enum ArtworkPlan {
    static func sources(for item: MediaItem, backdrop: Bool) -> [ArtworkSource] {
        var result: [ArtworkSource] = []
        func append(_ id: String?, _ kind: String, _ tag: String?, allowUntagged: Bool = false) {
            guard let id, !id.isEmpty, id.utf8.count <= 128,
                  id.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || $0 == 45 || $0 == 95 }) else { return }
            if let tag {
                guard !tag.isEmpty, tag.utf8.count <= 128,
                      !tag.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else { return }
            } else if !allowUntagged { return }
            // A tagged request already represents this image. Do not retry the same
            // source without its version if the server rejects it.
            guard !result.contains(where: { $0.id == id && $0.kind == kind }) else { return }
            result.append(ArtworkSource(id: id, kind: kind, tag: tag))
        }
        if item.Type == "Episode" || item.Type == "Season" {
            func belongsToSeries(_ parent: String?) -> Bool {
                guard let parent else { return false }
                guard let series = item.SeriesId else { return true }
                return MediaItem.matchesID(parent, series)
            }
            if backdrop {
                // Thumb is the series' landscape title card. Backdrop and poster
                // remain fallbacks for libraries without a title card.
                if belongsToSeries(item.ParentThumbItemId) {
                    append(item.ParentThumbItemId, "Thumb", item.ParentThumbImageTag)
                }
                append(item.SeriesId, "Thumb", nil, allowUntagged: true)
                if belongsToSeries(item.ParentBackdropItemId) {
                    append(item.ParentBackdropItemId, "Backdrop/0", item.ParentBackdropImageTags?.first)
                }
                append(item.SeriesId, "Backdrop/0", nil, allowUntagged: true)
            }
            append(item.SeriesId, "Primary", item.SeriesPrimaryImageTag, allowUntagged: true)
            if belongsToSeries(item.ParentPrimaryImageItemId) {
                append(item.ParentPrimaryImageItemId, "Primary", item.ParentPrimaryImageTag)
            }
            // An episode still is not series art. Missing series identity/art
            // keeps the neutral TV placeholder instead of showing the wrong image.
            return result
        }
        if item.Type == "Movie" {
            if backdrop {
                append(item.Id, "Thumb", item.ImageTags?["Thumb"], allowUntagged: true)
                append(item.Id, "Backdrop/0", item.BackdropImageTags?.first, allowUntagged: true)
            }
            append(item.Id, "Primary", item.ImageTags?["Primary"], allowUntagged: true)
            return result
        }
        if backdrop {
            append(item.Id, "Thumb", item.ImageTags?["Thumb"])
            append(item.Id, "Backdrop/0", item.BackdropImageTags?.first)
            append(item.ParentThumbItemId, "Thumb", item.ParentThumbImageTag)
            append(item.ParentBackdropItemId, "Backdrop/0", item.ParentBackdropImageTags?.first)
        }
        append(item.Id, "Primary", item.ImageTags?["Primary"])
        if item.Type == "Audio" {
            append(item.AlbumId, "Primary", item.AlbumPrimaryImageTag)
        }
        append(item.ParentPrimaryImageItemId, "Primary", item.ParentPrimaryImageTag)
        if result.isEmpty { append(item.Id, "Primary", nil, allowUntagged: true) }
        return result
    }

    static func pixelWidth(_ requested: Int?, backdrop: Bool) -> Int {
        min(1920, max(96, requested ?? (backdrop ? 1600 : 500)))
    }
}
