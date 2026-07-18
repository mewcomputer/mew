import SwiftUI
import MewMobileCore

/// Root navigation: Daemons → Sessions → Chat
struct RootView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var bannerTask: Task<Void, Never>?

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
        .overlay(alignment: .top) {
            if let banner = store.activeBanner {
                alertBanner(banner)
                    .transition(reduceMotion ? .opacity : .move(edge: .top).combined(with: .opacity))
                    .zIndex(100)
            }
        }
        .onChange(of: store.activeBanner) { _, banner in
            bannerTask?.cancel()
            if banner != nil {
                bannerTask = Task {
                    try? await Task.sleep(for: .seconds(5))
                    if !Task.isCancelled {
                        withAnimation(Theme.Motion.value(Theme.Motion.surface, reduced: reduceMotion)) {
                            store.activeBanner = nil
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func alertBanner(_ item: AlertItem) -> some View {
        Button {
            // Navigate to the alert's session.
            if let daemonId = store.selectedDaemonId {
                store.path.append(.chat(daemonNodeId: daemonId.nodeId, sessionId: item.sessionId))
            }
            withAnimation(Theme.Motion.value(Theme.Motion.surface, reduced: reduceMotion)) {
                store.activeBanner = nil
            }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: iconForAlertKind(item.kind))
                    .font(.body)
                    .foregroundStyle(.white)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    if let detail = item.detail {
                        Text(detail)
                            .font(.caption)
                            .foregroundStyle(.white.opacity(0.8))
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(
                RoundedRectangle(cornerRadius: Theme.Layout.panelRadius, style: .continuous)
                    .fill(.black.opacity(0.85))
            )
            .padding(.horizontal, 12)
            .padding(.top, 8)
        }
        .buttonStyle(.plain)
    }

    private func iconForAlertKind(_ kind: String) -> String {
        switch kind {
        case "permission":     return "hand.raised.fill"
        case "ask_user":       return "questionmark.bubble.fill"
        case "error":          return "exclamationmark.triangle.fill"
        default:               return "bell.fill"
        }
    }
}

enum NavigationRoute: Hashable {
    case sessions(daemonNodeId: String)
    case chat(daemonNodeId: String, sessionId: String)
    case settings
}
