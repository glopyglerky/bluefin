#if targetEnvironment(macCatalyst)
import SwiftUI

struct BluefinMacCommands: Commands {
    @ObservedObject var store: LibraryStore
    var body: some Commands {
        CommandMenu("Playback") {
            Button("Play or pause") { store.playback?.toggle() }
                .keyboardShortcut(.space, modifiers: [])
                .disabled(store.playback == nil)
            Button("Close player") {
                store.musicExpanded = false
                store.dismissPlayback()
            }
                .keyboardShortcut(".", modifiers: .command)
                .disabled(store.playback == nil)
        }
        CommandGroup(after: .newItem) {
            Button("Refresh library") { Task { await store.refresh() } }
                .keyboardShortcut("r", modifiers: .command)
        }
    }
}
#endif
