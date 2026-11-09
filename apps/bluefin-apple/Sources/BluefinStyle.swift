import SwiftUI

/// Semantic colors shared by the native library and playback views.
enum BluefinStyle {
    static let accent = Color.accentColor
    #if os(tvOS)
    static let background = Color(uiColor: UIColor { traits in
        traits.userInterfaceStyle == .dark ? UIColor(white: 0.055, alpha: 1) : UIColor(white: 0.98, alpha: 1)
    })
    static let secondaryBackground = Color(uiColor: UIColor { traits in
        traits.userInterfaceStyle == .dark ? UIColor(white: 0.11, alpha: 1) : UIColor(white: 0.94, alpha: 1)
    })
    #else
    static let background = Color(uiColor: .systemBackground)
    static let secondaryBackground = Color(uiColor: .secondarySystemBackground)
    #endif
    static let artworkRadius: CGFloat = 12
    static let contentWidth: CGFloat = 1200
}

struct BluefinBackground: View {
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        ZStack {
            BluefinStyle.background
            LinearGradient(colors: [BluefinStyle.accent.opacity(scheme == .dark ? 0.09 : 0.035), .clear], startPoint: .top, endPoint: .center)
        }.ignoresSafeArea()
    }
}
