#if canImport(UIKit) && !os(watchOS)
import SwiftUI
import UIKit
import CryptoKit

struct NativeLegalText: UIViewRepresentable {
    let text: String
    let html: Bool
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        #if !os(tvOS)
        view.isEditable = false
        view.dataDetectorTypes = []
        #endif
        view.isSelectable = true
        view.backgroundColor = .clear
        view.adjustsFontForContentSizeCategory = true
        view.textContainerInset = UIEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        view.delegate = context.coordinator
        return view
    }
    func updateUIView(_ view: UITextView, context: Context) {
        guard context.coordinator.previousText != text || context.coordinator.previousHTML != html || context.coordinator.previousDynamicType != dynamicTypeSize else { return }
        context.coordinator.previousText = text
        context.coordinator.previousHTML = html
        context.coordinator.previousDynamicType = dynamicTypeSize
        do {
            let document: NSMutableAttributedString
            if html {
                // These exact bundled originals have no active content or subresources.
                // A new toolchain document needs qualification before HTML admission.
                let bytes = Data(text.utf8)
                let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
                guard Self.qualifiedHTML.contains(digest) else { throw NoticeReadError.invalid }
                document = try NSMutableAttributedString(data: bytes, options: [
                    .documentType: NSAttributedString.DocumentType.html,
                    .characterEncoding: String.Encoding.utf8.rawValue
                ], documentAttributes: nil)
                context.coordinator.anchors = Self.anchors(in: text)
            } else {
                document = NSMutableAttributedString(string: text)
                context.coordinator.anchors = [:]
            }
            let all = NSRange(location: 0, length: document.length)
            let body = UIFont.preferredFont(forTextStyle: .body)
            let baseline = UIFont.preferredFont(forTextStyle: .body, compatibleWith: UITraitCollection(preferredContentSizeCategory: .large))
            let mono = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont.monospacedSystemFont(ofSize: baseline.pointSize, weight: .regular), compatibleWith: view.traitCollection)
            document.enumerateAttribute(.font, in: all) { value, range, _ in
                let traits = (value as? UIFont)?.fontDescriptor.symbolicTraits ?? []
                var font = traits.contains(.traitMonoSpace) ? mono : body
                if let descriptor = font.fontDescriptor.withSymbolicTraits(traits.intersection([.traitBold, .traitItalic, .traitMonoSpace])) { font = UIFont(descriptor: descriptor, size: body.pointSize) }
                document.addAttribute(.font, value: font, range: range)
            }
            document.addAttribute(.foregroundColor, value: UIColor.label, range: all)
            document.enumerateAttribute(.link, in: all) { value, range, _ in
                let url = (value as? URL) ?? (value as? String).flatMap(URL.init(string:))
                if let fragment = url?.fragment, context.coordinator.anchors[fragment] != nil,
                   let local = URL(string: "bluefin-legal://document/#" + fragment) {
                    document.addAttribute(.link, value: local, range: range)
                } else { document.removeAttribute(.link, range: range) }
            }
            view.attributedText = document
        } catch {
            context.coordinator.anchors = [:]
            view.attributedText = NSAttributedString(string: "This local notice could not be opened. Use Back to return.", attributes: [.font: UIFont.preferredFont(forTextStyle: .body), .foregroundColor: UIColor.label])
        }
    }
    private static let qualifiedHTML: Set<String> = [
        "90567e2718bf7fd65a71a3a43c5596488e80e5f51ed02bfea6fec54458b5f3d1",
        "5647be074c8edf7339fd863055923a8fc80bc5610a8d4661ec3b767b9d392c27"
    ]
    static func dismantleUIView(_ view: UITextView, coordinator: Coordinator) {
        view.delegate = nil
        view.attributedText = nil
        coordinator.previousText = nil
        coordinator.previousDynamicType = nil
        coordinator.anchors = [:]
    }
    private static func anchors(in source: String) -> [String: String] {
        guard let regex = try? NSRegularExpression(pattern: "<h[1-6]\\s+id=\"([^\"]+)\"[^>]*>([^<]+)</h[1-6]>", options: [.caseInsensitive]) else { return [:] }
        let value = source as NSString
        return Dictionary(regex.matches(in: source, range: NSRange(location: 0, length: value.length)).map { (value.substring(with: $0.range(at: 1)), value.substring(with: $0.range(at: 2))) }, uniquingKeysWith: { first, _ in first })
    }
    final class Coordinator: NSObject, UITextViewDelegate {
        var previousText: String?
        var previousDynamicType: DynamicTypeSize?
        var previousHTML = false
        var anchors: [String: String] = [:]
        #if !os(tvOS)
        @available(iOS 17.0, *)
        func textView(_ textView: UITextView, primaryActionFor textItem: UITextItem, defaultAction: UIAction) -> UIAction? {
            guard case let .link(url) = textItem.content else { return nil }
            return UIAction { [weak self, weak textView] _ in
                guard let self, let textView else { return }
                self.follow(url, in: textView)
            }
        }
        @available(iOS 17.0, *)
        func textView(_ textView: UITextView, menuConfigurationFor textItem: UITextItem, defaultMenu: UIMenu) -> UITextItem.MenuConfiguration? { nil }
        #endif
        @available(iOS, introduced: 10.0, deprecated: 17.0)
        @available(tvOS, introduced: 10.0, deprecated: 17.0)
        @available(visionOS, introduced: 1.0, deprecated: 1.0)
        func textView(_ textView: UITextView, shouldInteractWith url: URL, in characterRange: NSRange, interaction: UITextItemInteraction) -> Bool {
            follow(url, in: textView)
            return false
        }
        private func follow(_ url: URL, in textView: UITextView) {
            guard url.scheme == "bluefin-legal", url.host == "document", url.path == "/",
                  let fragment = url.fragment, let title = anchors[fragment],
                  let text = textView.attributedText?.string,
                  let regex = try? NSRegularExpression(pattern: "^[\\t ]*" + NSRegularExpression.escapedPattern(for: title) + "[\\t ]*$", options: [.anchorsMatchLines]),
                  let match = regex.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) else { return }
            textView.scrollRangeToVisible(match.range)
        }
    }
}
#endif
