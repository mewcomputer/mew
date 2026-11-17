import SwiftUI
import MewMobileCore

/// The combined conversation list owns navigation to chats and settings.
struct RootView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @State private var bannerTask: Task<Void, Never>?

    var body: some View {
        NavigationStack(path: $store.path) {
            SessionRailView()
                .navigationDestination(for: NavigationRoute.self) { route in
                    switch route {
                    case .machines:
                        DaemonListView()
                    case .chat(let daemonNodeId, let sessionId):
                        ChatView(daemonNodeId: daemonNodeId, sessionId: sessionId)
                    case .settings:
                        SettingsView()
                    case .daemonDetails(let daemonNodeId):
                        DaemonDetailsView(daemonNodeId: daemonNodeId)
                    }
                }
        }
        .tint(.accentColor)
        .preferredColorScheme(store.phoneSettings.appearance.colorScheme)
        .onAppear { store.isAppActive = scenePhase == .active }
        .onChange(of: scenePhase) { _, phase in store.isAppActive = phase == .active }
        .onChange(of: store.feedback) { _, feedback in
            guard store.isAppActive, store.phoneSettings.haptics, let feedback else { return }
            switch feedback.kind {
            case .sent: UIImpactFeedbackGenerator(style: .light).impactOccurred()
            case .completed: UINotificationFeedbackGenerator().notificationOccurred(.success)
            case .attention: UINotificationFeedbackGenerator().notificationOccurred(.warning)
            case .failed: UINotificationFeedbackGenerator().notificationOccurred(.error)
            }
        }
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
            store.openAlertSession(item)
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
        case "permissionneeded": return "hand.raised.fill"
        case "inputneeded": return "questionmark.bubble.fill"
        case "error", "turnfailed": return "exclamationmark.triangle.fill"
        default:               return "bell.fill"
        }
    }
}

enum NavigationRoute: Hashable {
    case machines
    case chat(daemonNodeId: String, sessionId: String)
    case settings
    case daemonDetails(daemonNodeId: String)
}
