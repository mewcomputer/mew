import SwiftUI
import MewMobileCore

// MARK: - SettingsView

/// Preferences and saved connections on this phone.
struct SettingsView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var copiedNodeId = false

    var body: some View {
        Form {
            appearanceSection
            feedbackSection
            reviewSection
            Section("This phone") {
                NavigationLink("Connection identity") {
                    Form { phoneSection }
                        .navigationTitle("Connection identity")
                        .navigationBarTitleDisplayMode(.inline)
                }
            }
            daemonsSection
            aboutSection
        }
        .navigationTitle("App settings")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: - Font

    private var appearanceSection: some View {
        Section("Appearance") {
            Picker("Theme", selection: $store.phoneSettings.appearance) {
                ForEach(PhoneAppearance.allCases, id: \.self) { choice in
                    Text(choice.label).tag(choice)
                }
            }
            NavigationLink {
                Form { fontSection }
                    .navigationTitle("Chat font")
                    .navigationBarTitleDisplayMode(.inline)
            } label: {
                LabeledContent("Chat font", value: store.fontChoice.displayName)
            }
        }
    }

    private var feedbackSection: some View {
        Section {
            Toggle("Haptic feedback", isOn: $store.phoneSettings.haptics)
            NavigationLink("Foreground alerts") {
                Form {
                    Section {
                        Toggle("Completed turns", isOn: $store.phoneSettings.completionAlerts)
                        Toggle("Turn and delivery errors", isOn: $store.phoneSettings.errorAlerts)
                        Toggle("Requests needing attention", isOn: $store.phoneSettings.requestAlerts)
                    } footer: {
                        Text("Banners and alert haptics appear while the app is open. Pending requests and conversation activity stay visible when alerts are off. Errors from actions you take are always shown.")
                    }
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
                    }
                }
                .navigationTitle("Foreground alerts")
                .navigationBarTitleDisplayMode(.inline)
            }
        } header: {
            Text("Feedback")
        } footer: {
            Text("Haptics acknowledge sent messages, completed turns, errors, and requests needing attention.")
        }
    }

    private var reviewSection: some View {
        Section {
            NavigationLink("Review readability") {
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
                    Section("Preview") {
                        ReviewDiffView(file: Self.previewFile, settings: store.phoneSettings)
                            .frame(height: 170)
                            .listRowInsets(EdgeInsets())
                    }
                }
                .navigationTitle("Review readability")
                .navigationBarTitleDisplayMode(.inline)
            }
        } header: {
            Text("Review")
        }
    }

    private static let previewFile = ReviewFile(path: "src/main.rs", oldPath: nil, status: .modified,
        hunks: [ReviewHunk(oldStart: 12, oldCount: 2, newStart: 12, newCount: 2, lines: [
            ReviewLine(kind: .context, oldLine: 12, newLine: 12, text: "fn greeting(name: &str) -> String {"),
            ReviewLine(kind: .deletion, oldLine: 13, newLine: nil, text: "    format!(\"Hello, {}\", name)"),
            ReviewLine(kind: .addition, oldLine: nil, newLine: 13, text: "    format!(\"Hello, {name}. Your workspace is ready.\")")
        ])], added: 1, removed: 1)

    @ViewBuilder
    private var fontSection: some View {
        Section {
            ForEach(MewFontChoice.allCases, id: \.self) { choice in
                Button {
                    store.fontChoice = choice
                } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(choice.displayName)
                                .font(choice.swiftUIFont(17))
                                .foregroundStyle(.primary)
                            Text(choice.previewText)
                                .font(choice.swiftUIFont(13))
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        if store.fontChoice == choice {
                            Image(systemName: "checkmark")
                                .foregroundStyle(.tint)
                        }
                    }
                    .frame(minHeight: 44)
                }
                .buttonStyle(.plain)
            }
        } header: {
            Text("Chat font")
        } footer: {
            Text("Choose the font for conversation text on this phone. Changes apply immediately.").font(.footnote)
        }
    }

    // MARK: - This Phone

    @ViewBuilder
    private var phoneSection: some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Text(store.phoneNodeId)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .truncationMode(.middle)
                    .textSelection(.enabled)

                Button {
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
                    Label(copiedNodeId ? "Copied!" : "Copy Node ID", systemImage: copiedNodeId ? "checkmark" : "doc.on.doc")
                        .font(.subheadline)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
                .disabled(store.phoneNodeId.isEmpty)
            }
            .padding(.vertical, 2)
        } header: {
            Text("This Phone")
        } footer: {
            Text("This is your phone's connection identity. To pair a new machine, scan or paste the complete invite from `mew pair`.").font(.footnote)
        }
    }

    // MARK: - Daemons

    @ViewBuilder
    private var daemonsSection: some View {
        Section("Saved machines") {
            if store.daemons.isEmpty {
                Text("No saved machines")
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
        }
    }

    // MARK: - About

    @ViewBuilder
    private var aboutSection: some View {
        Section("About") {
            NavigationLink("Connection diagnostics") {
                Form {
                    Section {
                        Text(store.connectionDiagnostics)
                            .font(.callout.monospaced())
                            .textSelection(.enabled)
                        ShareLink(item: store.connectionDiagnostics) {
                            Label("Share diagnostics", systemImage: "square.and.arrow.up")
                        }
                    } footer: {
                        Text("Includes app, iOS, and daemon versions and connection states. Excludes names, identities, credentials, error details, paths, and conversation content.")
                    }
                }
                .navigationTitle("Diagnostics")
                .navigationBarTitleDisplayMode(.inline)
            }
            LabeledContent {
                Text(appVersionString)
                    .foregroundStyle(.secondary)
            } label: {
                Label("Version", systemImage: "info.circle")
            }

            LabeledContent {
                Text(store.daemons.count, format: .number)
                    .foregroundStyle(.secondary)
            } label: {
                Label("Saved machines", systemImage: "server.rack")
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
