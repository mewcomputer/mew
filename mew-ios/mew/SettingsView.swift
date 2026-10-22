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
            fontSection
            phoneSection
            daemonsSection
            aboutSection
        }
        .navigationTitle("App settings")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: - Font

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
