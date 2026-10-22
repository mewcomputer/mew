import SwiftUI
import MewMobileCore

struct DaemonDefaultsView: View {
    let daemonNodeId: String
    @EnvironmentObject private var store: AppStore
    @State private var sessionToSave: String?
    @State private var showsResetConfirmation = false

    private var isConnected: Bool {
        if case .connected? = store.daemonStatuses[daemonNodeId] { return true }
        return false
    }
    private var isBusy: Bool { store.managementRequestIds[daemonNodeId] != nil }

    var body: some View {
        Form {
            if let settings = store.managementSettings[daemonNodeId] {
                Section {
                    if let defaults = settings.defaults {
                        LabeledContent("Model", value: defaults.model ?? "Daemon default")
                        if let provider = defaults.provider { LabeledContent("Provider", value: provider) }
                        LabeledContent("Persona", value: defaults.persona ?? "Default")
                        LabeledContent("Thinking", value: defaults.thinkingVariant ?? "Off")
                        LabeledContent("Permissions", value: permissionName(defaults.permissionMode))
                    } else {
                        Label("Using startup defaults", systemImage: "checkmark.circle")
                        Text("New chats use this daemon's startup configuration.")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                } header: {
                    Text("New chats on \(store.daemons.first(where: { $0.nodeId == daemonNodeId })?.name ?? "this machine")")
                } footer: {
                    Text("Applies to new chats from every client on this machine. Existing chats keep their own settings.")
                }
            }

            DaemonManagementStatusSection(daemonNodeId: daemonNodeId)

            Section {
                if store.selectedDaemonId?.nodeId == daemonNodeId, let session = store.selectedSessionId {
                    Button("Use this chat's settings…") { sessionToSave = session }
                        .disabled(isBusy || !store.canSendPrompt || store.managementSettings[daemonNodeId] == nil)
                        .frame(minHeight: 44)
                } else {
                    Text("Open a chat on this machine to save its model, persona, thinking, and permission choices as defaults.")
                        .font(.callout).foregroundStyle(.secondary)
                }
                Button("Reset to startup defaults…") { showsResetConfirmation = true }
                    .disabled(!isConnected || isBusy || (store.managementSettings[daemonNodeId]?.defaults == nil && store.managementErrors[daemonNodeId] == nil))
                    .frame(minHeight: 44)
            }
        }
        .navigationTitle("New chat defaults")
        .navigationBarTitleDisplayMode(.inline)
        .task { store.readDaemonManagementSettings(daemonNodeId: daemonNodeId) }
        .onChange(of: store.daemonStatuses[daemonNodeId]) { _, status in
            if case .connected? = status { store.readDaemonManagementSettings(daemonNodeId: daemonNodeId) }
        }
        .confirmationDialog("Use this chat's settings for new chats?", isPresented: Binding(
            get: { sessionToSave != nil }, set: { if !$0 { sessionToSave = nil } }
        ), titleVisibility: .visible) {
            Button("Save defaults") {
                if let sessionToSave { store.saveSessionDefaults(daemonNodeId: daemonNodeId, sessionId: sessionToSave) }
                sessionToSave = nil
            }
            Button("Cancel", role: .cancel) { sessionToSave = nil }
        } message: {
            Text("Model: \(store.currentModel[daemonNodeId] ?? "Daemon default")\nPersona: \(store.currentPersona[daemonNodeId] ?? "Default")\nThinking: \(store.thinkingVariant[daemonNodeId] ?? nil ?? "Off")\nPermissions: \(permissionName(store.permissionMode[daemonNodeId] ?? "Unknown"))\n\nThis affects new chats from every client on this machine.")
        }
        .confirmationDialog("Reset new chat defaults?", isPresented: $showsResetConfirmation, titleVisibility: .visible) {
            Button("Reset defaults") { store.resetSessionDefaults(daemonNodeId: daemonNodeId) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("New chats from every client will use this daemon's startup configuration. Existing chats keep their settings.")
        }
    }

    private func permissionName(_ mode: String) -> String {
        switch mode {
        case "auto_plus": return "Auto+"
        default: return mode.capitalized
        }
    }
}

struct PairedDevicesView: View {
    let daemonNodeId: String
    @EnvironmentObject private var store: AppStore
    @State private var showsRevokeConfirmation = false

    private var canManage: Bool {
        guard case .connected? = store.daemonStatuses[daemonNodeId] else { return false }
        return store.managementRequestIds[daemonNodeId] == nil
    }

    var body: some View {
        Form {
            if let settings = store.managementSettings[daemonNodeId] {
                Section {
                    if let devices = settings.devices {
                        if devices.isEmpty {
                            Text("No authorized devices").foregroundStyle(.secondary)
                        }
                        ForEach(devices, id: \.id) { device in
                            NavigationLink(value: device) {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(deviceName(device)).foregroundStyle(.primary)
                                    Text(device.nodeId == store.phoneNodeId ? "This phone · \(accessName(device))" : accessName(device))
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                                .frame(minHeight: 44)
                            }
                        }
                    } else {
                        Text("Device management is unavailable for this connection. Use the machine's remote access controls.")
                            .foregroundStyle(.secondary)
                    }
                } header: {
                    Text("Authorized on \(store.daemons.first(where: { $0.nodeId == daemonNodeId })?.name ?? "this machine")")
                } footer: {
                    Text("Revoking access disconnects the device and requires a new pairing invite. Conversations stay on this machine.")
                }
            }
            DaemonManagementStatusSection(daemonNodeId: daemonNodeId)
        }
        .navigationDestination(for: PairedDevice.self) { device in
            deviceDetail(device)
        }
        .navigationTitle("Paired devices")
        .navigationBarTitleDisplayMode(.inline)
        .task { store.readDaemonManagementSettings(daemonNodeId: daemonNodeId) }
        .onChange(of: store.daemonStatuses[daemonNodeId]) { _, status in
            if case .connected? = status { store.readDaemonManagementSettings(daemonNodeId: daemonNodeId) }
        }
    }

    private func deviceDetail(_ device: PairedDevice) -> some View {
        Form {
            Section("Device") {
                LabeledContent("Name", value: deviceName(device))
                LabeledContent("Access", value: accessName(device))
                if let pairedAt = device.pairedAt {
                    LabeledContent("Paired") {
                        Text(Date(timeIntervalSince1970: Double(pairedAt)), style: .date)
                    }
                }
                Text(device.nodeId).font(.caption.monospaced()).textSelection(.enabled)
                Button("Copy Node ID") { UIPasteboard.general.string = device.nodeId }
                    .frame(minHeight: 44)
            }
            Section {
                if let devices = store.managementSettings[daemonNodeId]?.devices {
                    if devices.contains(where: { $0.id == device.id }) {
                        Button("Revoke access…", role: .destructive) { showsRevokeConfirmation = true }
                            .disabled(!canManage || store.managementErrors[daemonNodeId] != nil)
                            .frame(minHeight: 44)
                    } else {
                        Label("Access revoked", systemImage: "checkmark.circle")
                    }
                }
            }
            DaemonManagementStatusSection(daemonNodeId: daemonNodeId)
        }
        .navigationTitle(deviceName(device))
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog("Revoke \(deviceName(device))?", isPresented: $showsRevokeConfirmation, titleVisibility: .visible) {
            Button("Revoke access", role: .destructive) {
                store.revokeDevice(daemonNodeId: daemonNodeId, deviceId: device.id)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(device.nodeId == store.phoneNodeId
                ? "This phone will disconnect and need a new pairing invite to reconnect."
                : "This device will disconnect and need a new pairing invite to reconnect.")
        }
    }

    private func deviceName(_ device: PairedDevice) -> String {
        device.name.isEmpty ? "Device \(device.nodeId.prefix(8))" : device.name
    }

    private func accessName(_ device: PairedDevice) -> String {
        switch device.scope {
        case "observe": return "Read only"
        case "collaborate": return "Collaborate"
        case "control": return "Full control"
        case nil: return "Full control (legacy)"
        default: return "Unknown access"
        }
    }
}

private struct DaemonManagementStatusSection: View {
    let daemonNodeId: String
    @EnvironmentObject private var store: AppStore

    var body: some View {
        Group {
            if store.managementRequestIds[daemonNodeId] != nil {
                Section { ProgressView("Waiting for confirmation…") }
            } else if let error = store.managementErrors[daemonNodeId] {
                Section {
                    Label("Couldn't confirm settings", systemImage: "exclamationmark.triangle")
                    Text(error).font(.callout).foregroundStyle(.secondary)
                    Button("Refresh settings", action: refresh).frame(minHeight: 44)
                } footer: {
                    Text("Requires an updated daemon and full-control access. Changes are shown after the daemon confirms them.")
                }
            } else if store.managementSettings[daemonNodeId] == nil {
                Section {
                    Text("Connect to this machine to read its settings.").foregroundStyle(.secondary)
                    Text("Return to machine details to reconnect or pair again.").font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
    }

    private func refresh() { store.readDaemonManagementSettings(daemonNodeId: daemonNodeId) }
}
