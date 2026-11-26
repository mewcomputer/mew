import SwiftUI
import MewMobileCore

struct DaemonSwitcherButton: View {
    let daemonNodeId: String
    var title: String? = nil

    @EnvironmentObject private var store: AppStore
    @State private var showsSwitcher = false

    private var machineName: String {
        store.daemons.first { $0.nodeId == daemonNodeId }?.name ?? "Choose a machine"
    }

    var body: some View {
        Button {
            showsSwitcher = true
        } label: {
            VStack(spacing: 2) {
                if let title, !title.isEmpty {
                    Text(title)
                        .font(.headline)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                HStack(spacing: 4) {
                    Text(machineName)
                        .font(title == nil ? .headline : .caption)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Image(systemName: "chevron.down")
                        .font(.caption2.weight(.semibold))
                }
                .foregroundStyle(title == nil ? .primary : .secondary)
            }
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title.map { "\($0), \(machineName)" } ?? machineName)
        .accessibilityHint("Switch machines or open machine details")
        .sheet(isPresented: $showsSwitcher) {
            DaemonSwitcherView { daemon in
                showsSwitcher = false
                store.openDaemon(daemon)
            }
        }
    }
}

struct DaemonSwitcherView: View {
    let onChoose: (DaemonEntry) -> Void

    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme
    @State private var showsPairing = false
    @State private var detailsNodeId: String?

    var body: some View {
        NavigationStack {
            List {
                Section("Saved machines") {
                    if store.daemons.isEmpty {
                        Text("Pair a machine to open its conversations.")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(store.daemons, id: \.nodeId) { daemon in
                        machineRow(daemon)
                    }
                }
                .listRowBackground(theme.card)
                Section {
                    Button("Pair a machine", systemImage: "plus") { showsPairing = true }
                        .frame(minHeight: 44)
                    NavigationLink {
                        SettingsView()
                    } label: {
                        Label("App settings", systemImage: "gearshape")
                            .frame(minHeight: 44)
                    }
                }
                .listRowBackground(theme.card)
            }
            .mewThemedForm(theme)
            .navigationTitle("Machines")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .sheet(isPresented: $showsPairing) {
                AddDaemonView { info, name in
                    store.addDaemon(info: info, name: name)
                }
            }
            .navigationDestination(item: $detailsNodeId) { nodeId in
                DaemonDetailsView(daemonNodeId: nodeId)
            }
        }
    }

    private func machineRow(_ daemon: DaemonEntry) -> some View {
        let selected = store.selectedDaemonId?.nodeId == daemon.nodeId
        let needsAttention = (store.sessionLists[daemon.nodeId] ?? []).reduce(0) {
            $0 + Int($1.pendingPermissions) + Int($1.pendingQuestions)
        }
        return HStack(spacing: 8) {
            Button {
                onChoose(daemon)
            } label: {
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(daemon.name).font(.headline).foregroundStyle(.primary)
                        DaemonConnectionStatusView(status: store.daemonStatuses[daemon.nodeId] ?? .disconnected)
                            .font(.caption)
                        if needsAttention > 0 {
                            Label(needsAttention == 1 ? "1 request needs attention" : "\(needsAttention) requests need attention", systemImage: "hand.raised")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 4)
                    if selected {
                        Image(systemName: "checkmark")
                            .foregroundStyle(.tint)
                            .accessibilityHidden(true)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(selected ? .isSelected : [])
            Button {
                detailsNodeId = daemon.nodeId
            } label: {
                Image(systemName: "info.circle")
                    .frame(minWidth: 44, minHeight: 44)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Details for \(daemon.name)")
        }
        .padding(.vertical, 4)
    }
}

struct DaemonConnectionStatusView: View {
    let status: DaemonStatus

    var body: some View {
        let label: String
        let symbol: String
        let color: Color
        switch status {
        case .connected:
            label = "Connected"
            symbol = "checkmark.circle.fill"
            color = Theme.connected
        case .connecting:
            label = "Connecting…"
            symbol = "arrow.triangle.2.circlepath"
            color = Theme.connecting
        case .backoff(let attempt, _):
            label = "Reconnecting · attempt \(attempt)"
            symbol = "arrow.clockwise.circle"
            color = Theme.backoff
        case .pairedLost:
            label = "Pairing needs attention"
            symbol = "exclamationmark.triangle"
            color = Theme.toolError
        case .disconnected:
            label = "Disconnected"
            symbol = "wifi.slash"
            color = Theme.disconnected
        }
        return Label {
            Text(label).foregroundStyle(.secondary)
        } icon: {
            Image(systemName: symbol).foregroundStyle(color)
        }
        .accessibilityElement(children: .combine)
    }
}
