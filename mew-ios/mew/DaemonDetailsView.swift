import SwiftUI
import MewMobileCore

struct DaemonDetailsView: View {
    let daemonNodeId: String

    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme
    @State private var nickname = ""
    @State private var nameError: String?
    @State private var showsRemoveConfirmation = false
    @State private var copiedNodeId = false
    @State private var showsPairing = false

    private var daemon: DaemonEntry? {
        store.daemons.first { $0.nodeId == daemonNodeId }
    }

    private var status: DaemonStatus {
        store.daemonStatuses[daemonNodeId] ?? .disconnected
    }

    private var isConnected: Bool {
        if case .connected = status { return true }
        return false
    }

    var body: some View {
        Group {
            if let daemon {
                Form {
                    connectionSection
                    nicknameSection(daemon)
                    automationSections
                    Section("Daemon settings") {
                        NavigationLink("New chat defaults") { DaemonDefaultsView(daemonNodeId: daemonNodeId) }
                        NavigationLink("Paired devices") { PairedDevicesView(daemonNodeId: daemonNodeId) }
                    }
                    .listRowBackground(theme.card)
                    advancedSection(daemon)
                    Section {
                        Button("Remove from this phone", role: .destructive) {
                            showsRemoveConfirmation = true
                        }
                        .frame(minHeight: 44)
                    }
                    .listRowBackground(theme.card)
                }
                .mewThemedForm(theme)
            } else {
                ContentUnavailableView("Connection removed", systemImage: "server.rack",
                    description: Text("This machine is no longer saved on this phone."))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(theme.background.ignoresSafeArea())
            }
        }
        .navigationTitle(daemon?.name ?? "Machine details")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            nickname = daemon?.name ?? ""
            if isConnected { store.readDaemonSettings(daemonNodeId: daemonNodeId) }
        }
        .onChange(of: status) { _, status in
            if case .connected = status {
                store.readDaemonSettings(daemonNodeId: daemonNodeId)
            }
        }
        .sheet(isPresented: $showsPairing) {
            AddDaemonView { info, name in
                store.addDaemon(info: info, name: daemon?.name ?? name, replacingDaemonNodeId: daemonNodeId)
            }
        }
        .confirmationDialog("Remove \(daemon?.name ?? "this machine") from this phone?",
            isPresented: $showsRemoveConfirmation, titleVisibility: .visible) {
            Button("Remove connection", role: .destructive) {
                if let daemon { store.removeDaemon(daemon) }
                dismiss()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This disconnects this phone and removes the saved connection. The machine keeps its conversations and this phone's authorization.")
        }
    }

    private var connectionSection: some View {
        Section("Connection") {
            DaemonConnectionStatusView(status: status)
                .frame(minHeight: 44, alignment: .leading)
            if case .backoff(_, let error) = status {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            if case .pairedLost = status {
                Button("Pair again", systemImage: "qrcode") { showsPairing = true }
                    .frame(minHeight: 44)
            } else if !isConnected {
                Button("Retry connection", systemImage: "arrow.clockwise") {
                    store.retryConnection(daemonNodeId: daemonNodeId)
                }
                .frame(minHeight: 44)
            }
        }
        .listRowBackground(theme.card)
    }

    private func nicknameSection(_ daemon: DaemonEntry) -> some View {
        Section {
            TextField("Nickname", text: $nickname)
                .textInputAutocapitalization(.words)
                .submitLabel(.done)
                .onSubmit { saveNickname(daemon) }
                .frame(minHeight: 44)
            Button("Save nickname") { saveNickname(daemon) }
                .disabled(nickname.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || nickname == daemon.name)
                .frame(minHeight: 44)
            if let nameError {
                Text(nameError).font(.callout).foregroundStyle(.red)
            }
        } header: {
            Text("Name on this phone")
        } footer: {
            Text("This nickname is saved on this phone.").font(.footnote)
        }
        .listRowBackground(theme.card)
    }

    private func saveNickname(_ daemon: DaemonEntry) {
        nameError = store.renameDaemon(daemon, name: nickname)
        if nameError == nil { nickname = nickname.trimmingCharacters(in: .whitespacesAndNewlines) }
    }

    @ViewBuilder
    private var automationSections: some View {
        if let settings = store.daemonSettings[daemonNodeId] {
            Section {
                Toggle("Auto-title", isOn: Binding(
                    get: { store.daemonSettings[daemonNodeId]?.autoTitleEnabled ?? settings.autoTitleEnabled },
                    set: { store.setAutoTitle(daemonNodeId: daemonNodeId, enabled: $0) }
                ))
                .disabled(!isConnected)
                .frame(minHeight: 44)
            } header: {
                Text("This phone connection")
            } footer: {
                Text("Generate conversation titles for prompts sent through this phone's current connection to this machine.").font(.footnote)
            }
            .listRowBackground(theme.card)
            Section {
                Toggle("Auto-summary", isOn: Binding(
                    get: { store.daemonSettings[daemonNodeId]?.autoSummaryEnabled ?? settings.autoSummaryEnabled },
                    set: { store.setAutoSummary(daemonNodeId: daemonNodeId, enabled: $0) }
                ))
                .disabled(!isConnected)
                .frame(minHeight: 44)
            } header: {
                Text("Daemon automation")
            } footer: {
                Text("Generate summaries while this daemon is idle. This setting affects all its sessions and connected clients.").font(.footnote)
            }
            .listRowBackground(theme.card)
        } else {
            Section("Automation") {
                if isConnected && store.daemonSettingsErrors[daemonNodeId] == nil {
                    ProgressView("Reading settings…")
                } else if !isConnected {
                    Text("Connect to read this machine's automation settings.")
                        .foregroundStyle(.secondary)
                }
            }
            .listRowBackground(theme.card)
        }
        if let error = store.daemonSettingsErrors[daemonNodeId] {
            Section {
                Label("Automation settings unavailable", systemImage: "exclamationmark.triangle")
                Text(error).font(.callout).foregroundStyle(.secondary)
                if isConnected {
                    Button("Try again") { store.readDaemonSettings(daemonNodeId: daemonNodeId) }
                        .frame(minHeight: 44)
                }
            }
            .listRowBackground(theme.card)
        }
    }

    private func advancedSection(_ daemon: DaemonEntry) -> some View {
        Section {
            DisclosureGroup("Advanced") {
                LabeledContent("Daemon version", value: store.daemonVersions[daemonNodeId] ?? daemon.lastKnownVersion ?? "Unknown")
                Text(daemonNodeId)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                Button(copiedNodeId ? "Copied Node ID" : "Copy Node ID", systemImage: copiedNodeId ? "checkmark" : "doc.on.doc") {
                    UIPasteboard.general.string = daemonNodeId
                    copiedNodeId = true
                }
                .frame(minHeight: 44)
            }
        } footer: {
            Text("The Node ID identifies this machine. Pairing a new phone requires the complete invite from `mew pair`.").font(.footnote)
        }
        .listRowBackground(theme.card)
    }
}
