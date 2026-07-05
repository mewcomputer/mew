import SwiftUI
import MewMobileCore

/// Root navigation: Daemons → Sessions → Chat
struct RootView: View {
    @EnvironmentObject var store: AppStore

    var body: some View {
        NavigationStack(path: $store.path) {
            DaemonListView()
                .navigationDestination(for: NavigationRoute.self) { route in
                    switch route {
                    case .sessions(let daemonNodeId):
                        SessionRailView(daemonNodeId: daemonNodeId)
                    case .chat(let daemonNodeId, let sessionId):
                        ChatView(daemonNodeId: daemonNodeId, sessionId: sessionId)
                    case .settings:
                        SettingsView()
                    }
                }
        }
        .tint(.accentColor)
    }
}

enum NavigationRoute: Hashable {
    case sessions(daemonNodeId: String)
    case chat(daemonNodeId: String, sessionId: String)
    case settings
}
