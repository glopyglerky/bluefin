import TVServices

final class ContentProvider: TVTopShelfContentProvider {
    override func loadTopShelfContent(completionHandler: @escaping ((any TVTopShelfContent)?) -> Void) {
        guard let snapshot = TopShelfSnapshot.read(), let folder = snapshot.folder else {
            completionHandler(nil)
            return
        }
        let items = snapshot.cards.compactMap { card -> TVTopShelfCarouselItem? in
            let image = folder.appendingPathComponent(card.image.uuidString + ".jpg")
            guard FileManager.default.isReadableFile(atPath: image.path),
                  let url = snapshot.launchURL(for: card) else { return nil }
            let item = TVTopShelfCarouselItem(identifier: card.id)
            item.title = card.title
            item.contextTitle = "Recently added"
            item.expirationDate = snapshot.expires
            item.setImageURL(image, for: [.screenScale1x, .screenScale2x])
            item.displayAction = TVTopShelfAction(url: url)
            return item
        }
        completionHandler(items.isEmpty ? nil : TVTopShelfCarouselContent(style: .actions, items: items))
    }
}
