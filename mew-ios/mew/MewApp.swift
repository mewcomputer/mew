import SwiftUI
import MewMobileCore

@main
struct MewApp: App {
    @StateObject private var appStore = AppStore()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(appStore)
                .onAppear { appStore.start() }
        }
    }
}
