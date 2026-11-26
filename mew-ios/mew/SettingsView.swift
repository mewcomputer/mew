import SwiftUI
import MewMobileCore

// MARK: - SettingsView

/// Preferences and saved connections on this phone.
struct SettingsView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mewTheme) private var theme

    @State private var copiedNodeId = false

    var body: some View {
        Form {
            appearanceSection
                .listRowBackground(theme.card)
            feedbackAndReviewSection
                .listRowBackground(theme.card)
            machinesSection
                .listRowBackground(theme.card)
            aboutSection
                .listRowBackground(theme.card)
        }
        .mewThemedForm(theme)
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: - Appearance

    private var appearanceSection: some View {
        Section("Appearance") {
            Picker(selection: $store.phoneSettings.appearance) {
                ForEach(PhoneAppearance.allCases, id: \.self) { choice in
                    Text(choice.label).tag(choice)
                }
            } label: {
                Label("Theme", systemImage: "circle.lefthalf.filled")
            }
            .pickerStyle(.segmented)

            Picker(selection: $store.phoneSettings.lightThemeId) {
                ForEach(MewThemes.lightThemes) { palette in
                    Text(palette.displayName).tag(palette.id)
                }
            } label: {
                Label("Light theme", systemImage: "sun.max")
            }

            Picker(selection: $store.phoneSettings.darkThemeId) {
                ForEach(MewThemes.darkThemes) { palette in
                    Text(palette.displayName).tag(palette.id)
                }
            } label: {
                Label("Dark theme", systemImage: "moon.stars")
            }

            Picker(selection: $store.fontChoice) {
                ForEach(MewFontChoice.allCases, id: \.self) { choice in
                    Text(choice.displayName).tag(choice)
                }
            } label: {
                Label("Chat font", systemImage: "textformat")
            }

            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    ForEach([theme.background, theme.userBubble, theme.toolBackground, theme.textAccent], id: \.self) { swatch in
                        Circle()
                            .fill(swatch)
                            .frame(width: 12, height: 12)
                            .overlay(Circle().strokeBorder(theme.border, lineWidth: 0.5))
                    }
                    Text(theme.displayName)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(theme.mutedForeground)
                }
                Text(store.fontChoice.previewText)
                    .font(store.fontChoice.swiftUIFont(15))
                    .foregroundStyle(theme.foreground)
                Text("let model = \"\(store.fontChoice.displayName)\"")
                    .font(.mewMono(12))
                    .foregroundStyle(theme.mutedForeground)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(theme.background, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(theme.border, lineWidth: 0.5))
            .listRowInsets(EdgeInsets(top: 6, leading: 16, bottom: 8, trailing: 16))
        }
    }

    // MARK: - Feedback & Review

    private var feedbackAndReviewSection: some View {
        Section {
            Toggle(isOn: $store.phoneSettings.haptics) {
                Label("Haptic feedback", systemImage: "iphone.radiowaves.left.and.right")
            }

            NavigationLink {
                Form {
                    Section {
                        Toggle("Completed turns", isOn: $store.phoneSettings.completionAlerts)
                        Toggle("Turn and delivery errors", isOn: $store.phoneSettings.errorAlerts)
                        Toggle("Requests needing attention", isOn: $store.phoneSettings.requestAlerts)
                    } footer: {
                        Text("Banners and alert haptics appear while the app is open. Pending requests and conversation activity stay visible when alerts are off. Errors from actions you take are always shown.")
                    }
                    .listRowBackground(theme.card)
                    if !store.daemons.isEmpty {
                        Section {
                            ForEach(store.daemons, id: \.nodeId) { daemon in
                                Toggle(daemon.name, isOn: Binding(
                                    get: { !store.phoneSettings.mutedMachines.contains(daemon.nodeId) },
                                    set: { enabled in
                                        if enabled { store.phoneSettings.mutedMachines.remove(daemon.nodeId) }
                                        else { store.phoneSettings.mutedMachines.insert(daemon.nodeId) }
                                    }
                                ))
                            }
                        } header: {
                            Text("Alerts from machines")
                        } footer: {
                            Text("These preferences affect only this phone.")
                        }
                        .listRowBackground(theme.card)
                    }
                }
                .mewThemedForm(theme)
                .navigationTitle("Foreground alerts")
                .navigationBarTitleDisplayMode(.inline)
            } label: {
                Label("Foreground alerts", systemImage: "bell.badge")
            }

            NavigationLink {
                Form {
                    Section {
                        Toggle("Wrap long lines", isOn: $store.phoneSettings.reviewWrapLines)
                        Toggle("Line numbers", isOn: $store.phoneSettings.reviewLineNumbers)
                        Picker("Text size", selection: $store.phoneSettings.reviewTextSize) {
                            ForEach(ReviewTextSize.allCases, id: \.self) { size in
                                Text(size.label).tag(size)
                            }
                        }
                    } footer: {
                        Text("Applies to file diffs on this phone. Text also follows your iOS text size. Conversation text keeps its current size.")
                    }
                    .listRowBackground(theme.card)
                    Section("Preview") {
                        ReviewDiffView(file: Self.previewFile, settings: store.phoneSettings)
                            .frame(height: 170)
                            .listRowInsets(EdgeInsets())
                    }
                    .listRowBackground(theme.card)
                }
                .mewThemedForm(theme)
                .navigationTitle("Review readability")
                .navigationBarTitleDisplayMode(.inline)
            } label: {
                Label("Review readability", systemImage: "doc.text.magnifyingglass")
            }
        } header: {
            Text("Feedback & Review")
        }
    }

    private static let previewFile = ReviewFile(path: "src/main.rs", oldPath: nil, status: .modified,
        hunks: [ReviewHunk(oldStart: 12, oldCount: 2, newStart: 12, newCount: 2, lines: [
            ReviewLine(kind: .context, oldLine: 12, newLine: 12, text: "fn greeting(name: &str) -> String {"),
            ReviewLine(kind: .deletion, oldLine: 13, newLine: nil, text: "    format!(\"Hello, {}\", name)"),
            ReviewLine(kind: .addition, oldLine: nil, newLine: 13, text: "    format!(\"Hello, {name}. Your workspace is ready.\")")
        ])], added: 1, removed: 1)

    // MARK: - Machines & Phone Identity

    private var machinesSection: some View {
        Section {
            if store.daemons.isEmpty {
                Label("No saved machines", systemImage: "server.rack")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(store.daemons, id: \.nodeId) { daemon in
                    NavigationLink {
                        DaemonDetailsView(daemonNodeId: daemon.nodeId)
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(daemon.name)
                                .foregroundStyle(.primary)
                            DaemonConnectionStatusView(status: store.daemonStatuses[daemon.nodeId] ?? .disconnected)
                                .font(.caption)
                        }
                        .frame(minHeight: 44, alignment: .leading)
                    }
                }
            }

            Button {
                guard !store.phoneNodeId.isEmpty else { return }
                UIPasteboard.general.string = store.phoneNodeId
                withAnimation(Theme.Motion.value(Theme.Motion.surface, reduced: reduceMotion)) {
                    copiedNodeId = true
                }
                Task {
                    try? await Task.sleep(nanoseconds: 1_500_000_000)
                    await MainActor.run {
                        withAnimation(Theme.Motion.value(Theme.Motion.surface, reduced: reduceMotion)) {
                            copiedNodeId = false
                        }
                    }
                }
            } label: {
                HStack(spacing: 12) {
                    Label("Phone identity", systemImage: "iphone")
                        .foregroundStyle(.primary)
                    Spacer(minLength: 8)
                    Text(store.phoneNodeId.isEmpty ? "Unavailable" : store.phoneNodeId)
                        .font(.mewMono(11))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: 130, alignment: .trailing)
                    Image(systemName: copiedNodeId ? "checkmark" : "doc.on.doc")
                        .font(.caption)
                        .foregroundStyle(copiedNodeId ? .green : .secondary)
                }
            }
            .disabled(store.phoneNodeId.isEmpty)
        } header: {
            Text("Machines")
        } footer: {
            Text("Tap Phone identity to copy this phone's Node ID when pairing from `mew pair`.")
        }
    }

    // MARK: - About

    private var aboutSection: some View {
        Section("About") {
            NavigationLink {
                Form {
                    Section {
                        Text(store.connectionDiagnostics)
                            .font(.mewMono(13))
                            .textSelection(.enabled)
                        ShareLink(item: store.connectionDiagnostics) {
                            Label("Share diagnostics", systemImage: "square.and.arrow.up")
                        }
                    } footer: {
                        Text("Includes app, iOS, and daemon versions and connection states. Excludes names, identities, credentials, error details, paths, and conversation content.")
                    }
                    .listRowBackground(theme.card)
                }
                .mewThemedForm(theme)
                .navigationTitle("Diagnostics")
                .navigationBarTitleDisplayMode(.inline)
            } label: {
                Label("Connection diagnostics", systemImage: "stethoscope")
            }

            LabeledContent {
                Text(appVersionString)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            } label: {
                Label("Version", systemImage: "info.circle")
            }

            Link(destination: URL(string: "https://github.com/polytoken/mew")!) {
                HStack {
                    Label("mew on GitHub", systemImage: "chevron.left.forwardslash.chevron.right")
                    Spacer()
                    Image(systemName: "arrow.up.forward")
                        .foregroundStyle(.secondary)
                        .font(.caption)
                }
            }

            Link(destination: URL(string: "https://polytoken.com")!) {
                HStack {
                    Label("Documentation", systemImage: "book")
                    Spacer()
                    Image(systemName: "arrow.up.forward")
                        .foregroundStyle(.secondary)
                        .font(.caption)
                }
            }
        }
    }

    private var appVersionString: String {
        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—"
        let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "—"
        return "\(version) (\(build))"
    }
}
