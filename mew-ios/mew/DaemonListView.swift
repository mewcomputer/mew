import SwiftUI
import MewMobileCore

struct DaemonListView: View {
    @EnvironmentObject var store: AppStore
    @State private var showAddDaemon = false
    @State private var daemonToDelete: DaemonEntry?

    var body: some View {
        List {
            if store.daemons.isEmpty {
                ContentUnavailableView(
                    "No Daemons",
                    systemImage: "server.rack",
                    description: Text("Pair with a mew daemon to get started. Run `mew pair` on your machine, then tap +.")
                )
            } else {
                ForEach(store.daemons, id: \.nodeId) { daemon in
                    daemonRow(daemon)
                        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                            Button(role: .destructive) {
                                daemonToDelete = daemon
                            } label: {
                                Label("Delete", systemImage: "trash")
                            }
                        }
                        .swipeActions(edge: .leading) {
                            NavigationLink(value: NavigationRoute.settings) {
                                Label("Settings", systemImage: "gearshape")
                            }
                            .tint(.blue)
                        }
                        .contextMenu {
                            NavigationLink(value: NavigationRoute.settings) {
                                Label("Settings", systemImage: "gearshape")
                            }
                            Button("Remove", role: .destructive) {
                                daemonToDelete = daemon
                            }
                        }
                }
            }
        }
        .navigationTitle("mew")
        .toolbarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text("mew")
                    .font(.mewDisplay(24))
                    .foregroundStyle(.primary)
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button { showAddDaemon = true } label: {
                    Image(systemName: "plus")
                }
            }
            ToolbarItem(placement: .topBarLeading) {
                NavigationLink(value: NavigationRoute.settings) {
                    Image(systemName: "gearshape")
                }
            }
        }
        .sheet(isPresented: $showAddDaemon) {
            AddDaemonView { nodeId, name in
                store.addDaemon(nodeId: nodeId, name: name)
            }
        }
        .onAppear {
            store.daemons = store.daemons // refresh from store
        }
        .alert("Remove Daemon?", isPresented: Binding(
            get: { daemonToDelete != nil },
            set: { if !$0 { daemonToDelete = nil } }
        )) {
            Button("Cancel", role: .cancel) {}
            Button("Remove", role: .destructive) {
                if let daemon = daemonToDelete {
                    store.removeDaemon(daemon)
                    daemonToDelete = nil
                }
            }
        } message: {
            if let daemon = daemonToDelete {
                Text("Remove \(daemon.name)? This will disconnect and delete the daemon from your list.")
            }
        }
    }

    @ViewBuilder
    private func daemonRow(_ daemon: DaemonEntry) -> some View {
        let status = store.daemonStatuses[daemon.nodeId] ?? .disconnected
        let needsYou = store.sessionLists[daemon.nodeId]?.reduce(0) { acc, s in
            acc + Int(s.pendingPermissions) + Int(s.pendingQuestions)
        } ?? 0

        NavigationLink(value: NavigationRoute.sessions(daemonNodeId: daemon.nodeId)) {
            HStack(spacing: 12) {
                statusDot(status)
                VStack(alignment: .leading, spacing: 2) {
                    Text(daemon.name)
                        .font(.headline)
                    Text(daemon.nodeId.prefix(16) + "…")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                    if let version = store.daemonVersions[daemon.nodeId] {
                        Text("v\(version)")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    }
                }
                Spacer()
                if needsYou > 0 {
                    Text("\(needsYou)")
                        .font(.caption.bold())
                        .foregroundStyle(.white)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background(.orange, in: Capsule())
                }
            }
            .padding(.vertical, 4)
        }
    }

    @ViewBuilder
    private func statusDot(_ status: DaemonStatus) -> some View {
        let (color, animated): (Color, Bool) = switch status {
        case .connected: (.green, false)
        case .connecting: (.yellow, true)
        case .backoff: (.orange, true)
        case .pairedLost: (.red, false)
        case .disconnected: (.gray, false)
        }
        Circle()
            .fill(color)
            .frame(width: 10, height: 10)
            .opacity(animated ? 0.6 : 1.0)
            .modifier(PulseModifier(active: animated))
    }
}

struct PulseModifier: ViewModifier {
    let active: Bool
    @State private var pulsing = false

    func body(content: Content) -> some View {
        content
            .scaleEffect(pulsing ? 1.3 : 1.0)
            .animation(
                active ? .easeInOut(duration: 0.8).repeatForever(autoreverses: true) : .default,
                value: pulsing
            )
            .onAppear { pulsing = active }
    }
}

// MARK: - Add Daemon Sheet

struct AddDaemonView: View {
    @Environment(\.dismiss) private var dismiss
    let onAdd: (String, String) -> Void

    @State private var nodeIdInput = ""
    @State private var name = ""
    @State private var error: String?
    @State private var showScanner = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Pair") {
                    TextField("Node ID or mew001:…", text: $nodeIdInput, axis: .vertical)
                        .lineLimit(3...6)
                        .font(.body.monospaced())
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Name (e.g. Homelab)", text: $name)
                }

                Section {
                    Button {
                        showScanner = true
                    } label: {
                        Label("Scan QR Code", systemImage: "qrcode.viewfinder")
                    }
                }

                Section {
                    Text("Run `mew pair` on your daemon machine, then paste the Node ID or scan the QR code.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                if let error {
                    Section {
                        Text(error)
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Add Daemon")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") { addDaemon() }
                        .disabled(nodeIdInput.isEmpty)
                }
            }
            .sheet(isPresented: $showScanner) {
                QRScannerSheet { scanned in
                    nodeIdInput = scanned
                    showScanner = false
                }
            }
        }
    }

    private func addDaemon() {
        let trimmed = nodeIdInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }

        // Try to parse via the core's parseDialInfo
        do {
            let info = try MewMobileCore.parseDialInfo(payload: trimmed)
            let displayName = name.isEmpty ? (info.name ?? "Daemon") : name
            onAdd(info.nodeId, displayName)
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// Scanner sheet wrapping the camera view with a dismiss button.
struct QRScannerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let onScan: (String) -> Void

    var body: some View {
        NavigationStack {
            QRScannerView { payload in
                onScan(payload)
            }
            .ignoresSafeArea()
            .navigationTitle("Scan QR")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }
}
