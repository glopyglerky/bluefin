import SwiftUI

/// Navigation follows the current scene, including resized and folded windows.
struct BluefinNavigation<Content: View>: View {
    private let content: Content
    init(@ViewBuilder content: () -> Content) { self.content = content() }
    @ViewBuilder var body: some View {
        if #available(iOS 16, tvOS 16, *) {
            NavigationStack { content }
        } else {
            NavigationView { content }.navigationViewStyle(.stack)
        }
    }
}
