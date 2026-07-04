import SwiftUI
import MewMobileCore

@main
struct MewApp: App {
    @StateObject private var appStore = AppStore()

    init() {
        // Font is applied per-view in ChatView, not globally.
        MewFonts.verify()
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(appStore)
                .onAppear { appStore.start() }
        }
    }
}
