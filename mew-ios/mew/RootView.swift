import SwiftUI
import MewMobileCore

/// The combined conversation list owns navigation to chats and settings.
struct RootView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.colorScheme) private var systemColorScheme
    @State private var bannerTask: Task<Void, Never>?

    private var isDarkMode: Bool {
        switch store.phoneSettings.appearance {
        case .dark: return true
        case .light: return false
        case .system: return systemColorScheme == .dark
        }
    }

    private var activeTheme: MewThemePalette {
        let id = isDarkMode ? store.phoneSettings.darkThemeId : store.phoneSettings.lightThemeId
        return MewThemes.palette(id: id, fallbackDark: isDarkMode)
    }

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
        .environment(\.mewTheme, activeTheme)
        .tint(activeTheme.textAccent)
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
                    .foregroundStyle(colorForAlertKind(item.kind))
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    if let detail = item.detail {
                        Text(detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: Theme.Layout.panelRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Layout.panelRadius, style: .continuous)
                    .strokeBorder(Color(.separator).opacity(0.5), lineWidth: 0.5)
            )
            .shadow(color: .black.opacity(0.12), radius: 10, y: 4)
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

    private func colorForAlertKind(_ kind: String) -> Color {
        switch kind {
        case "permissionneeded": return .orange
        case "inputneeded": return .blue
        case "error", "turnfailed": return .red
        default:               return .accentColor
        }
    }
}

enum NavigationRoute: Hashable {
    case machines
    case chat(daemonNodeId: String, sessionId: String)
    case settings
    case daemonDetails(daemonNodeId: String)
}
