import SwiftUI
import Foundation

private enum NoticeBuildScope {
    static var target: String? {
        #if os(watchOS)
        return nil
        #elseif targetEnvironment(macCatalyst)
        #if arch(arm64)
        return "aarch64-apple-ios-macabi"
        #elseif arch(x86_64)
        return "x86_64-apple-ios-macabi"
        #else
        return nil
        #endif
        #elseif os(visionOS)
        #if targetEnvironment(simulator)
        return "aarch64-apple-visionos-sim"
        #else
        return "aarch64-apple-visionos"
        #endif
        #elseif os(tvOS)
        #if targetEnvironment(simulator)
        return "aarch64-apple-tvos-sim"
        #else
        return "aarch64-apple-tvos"
        #endif
        #elseif os(iOS) && arch(arm64)
        #if targetEnvironment(simulator)
        return "aarch64-apple-ios-sim"
        #else
        return "aarch64-apple-ios"
        #endif
        #else
        return nil
        #endif
    }
}

struct SoftwareLicensesView: View {
    var body: some View {
        Group {
            if let root = Bundle.main.url(forResource: "Legal", withExtension: nil) {
                #if os(watchOS)
                NativeNoticeList(store: NoticeStore(root: root), indexName: nil, target: nil)
                #else
                if let target = NoticeBuildScope.target {
                    NativeNoticeList(store: NoticeStore(root: root), indexName: "apple-" + target, target: target)
                } else { Text("Software licenses are unavailable for this build.") }
                #endif
            } else { Text("Software license resources could not be opened. Use Back to return.") }
        }.navigationTitle("Software licenses")
    }
}
