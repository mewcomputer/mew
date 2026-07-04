import SwiftUI
import MewMobileCore

// MARK: - SettingsView

/// Settings screen reachable from the daemon list's gear button.
/// Shows this phone's NodeId, the list of paired daemons, and app about info.
struct SettingsView: View {
    @EnvironmentObject var store: AppStore

    @State private var copiedNodeId = false
    @State private var daemonToRemove: DaemonEntry?
    @State private var renameTarget: DaemonEntry?
    @State private var renameText = ""

    var body: some View {
        Form {
            fontSection
            phoneSection
            daemonsSection
            aboutSection
        }
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog(
            "Remove \(daemonToRemove?.name ?? "Daemon")?",
            isPresented: Binding(
                get: { daemonToRemove != nil },
                set: { if !$0 { daemonToRemove = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                if let daemon = daemonToRemove {
                    store.removeDaemon(daemon)
                    daemonToRemove = nil
                }
            }
            Button("Cancel", role: .cancel) { daemonToRemove = nil }
        } message: {
            Text("This will unpair the daemon and disconnect from it. It can be re-paired with `mew pair`.")
        }
        .alert("Rename Daemon", isPresented: Binding(
            get: { renameTarget != nil },
            set: { if !$0 { renameTarget = nil } }
        )) {
            TextField("Name", text: $renameText)
            Button("Save") {
                if let daemon = renameTarget {
                    renameDaemon(daemon, to: renameText)
                }
                renameTarget = nil
            }
            Button("Cancel", role: .cancel) { renameTarget = nil }
        }
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
                }
                .buttonStyle(.plain)
            }
        } header: {
            Text("Font")
        } footer: {
            Text("Choose the body font for the app. Changes apply immediately.")
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
                    withAnimation { copiedNodeId = true }
                    Task {
                        try? await Task.sleep(nanoseconds: 1_500_000_000)
                        await MainActor.run {
                            withAnimation { copiedNodeId = false }
                        }
                    }
                } label: {
                    Label(copiedNodeId ? "Copied!" : "Copy Node ID", systemImage: copiedNodeId ? "checkmark" : "doc.on.doc")
                        .font(.subheadline)
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .disabled(store.phoneNodeId.isEmpty)
            }
            .padding(.vertical, 2)
        } header: {
            Text("This Phone")
        } footer: {
            Text("This Node ID is what daemons need to allowlist before they'll accept your connection. Share it via `mew pair` or paste it directly.")
        }
    }

    // MARK: - Daemons

    @ViewBuilder
    private var daemonsSection: some View {
        Section("Daemons") {
            if store.daemons.isEmpty {
                Text("No daemons paired yet")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(store.daemons, id: \.nodeId) { daemon in
                    daemonRow(daemon)
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) {
                                daemonToRemove = daemon
                            } label: {
                                Label("Remove", systemImage: "trash")
                            }
                        }
                        .contentShape(Rectangle())
                        .onTapGesture {
                            renameText = daemon.name
                            renameTarget = daemon
                        }
                }
            }
        }
    }

    @ViewBuilder
    private func daemonRow(_ daemon: DaemonEntry) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "server.rack")
                .foregroundStyle(.secondary)
                .font(.title3)

            VStack(alignment: .leading, spacing: 2) {
                Text(daemon.name)
                    .font(.headline)
                Text(truncatedNodeId(daemon.nodeId))
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                if let version = store.daemonVersions[daemon.nodeId] ?? daemon.lastKnownVersion {
                    Text("v\(version)")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }

            Spacer()

            Image(systemName: "pencil")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
    }

    /// Truncates a long Node ID to the first 16 chars + ellipsis, matching the
    /// style used in DaemonListView.
    private func truncatedNodeId(_ nodeId: String) -> String {
        let prefix = nodeId.prefix(16)
        return nodeId.count > 16 ? "\(prefix)…" : String(prefix)
    }

    /// Renames a daemon. The current AppStore/core has no rename-daemon API,
    /// so this is a no-op stub — it keeps the UI flow intact for when a rename
    /// RPC lands. Wire it up by calling the core method here once available.
    private func renameDaemon(_ daemon: DaemonEntry, to newName: String) {
        let trimmed = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != daemon.name else { return }
        // TODO: once the core exposes a rename-daemon RPC, call it here.
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
                Label("Paired Daemons", systemImage: "server.rack")
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
