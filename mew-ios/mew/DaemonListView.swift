import SwiftUI
import MewMobileCore

struct DaemonListView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.mewTheme) private var theme
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
                .listRowBackground(Color.clear)
            } else {
                ForEach(store.daemons, id: \.nodeId) { daemon in
                    daemonRow(daemon)
                        .listRowBackground(theme.card)
                        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                            Button(role: .destructive) {
                                daemonToDelete = daemon
                            } label: {
                                Label("Remove", systemImage: "trash")
                            }
                        }
                        .swipeActions(edge: .leading) {
                            NavigationLink(value: NavigationRoute.daemonDetails(daemonNodeId: daemon.nodeId)) {
                                Label("Machine details", systemImage: "gearshape")
                            }
                            .tint(.blue)
                        }
                        .contextMenu {
                            NavigationLink(value: NavigationRoute.daemonDetails(daemonNodeId: daemon.nodeId)) {
                                Label("Machine details", systemImage: "gearshape")
                            }
                            Button("Remove", role: .destructive) {
                                daemonToDelete = daemon
                            }
                        }
                }
            }
        }
        .mewThemedForm(theme)
        .navigationTitle("Machines")
        .toolbarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text("Machines")
                    .font(.headline)
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
            AddDaemonView { info, name in
                store.addDaemon(info: info, name: name)
            }
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

        Button {
            store.openDaemon(daemon)
        } label: {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(daemon.name)
                        .font(.headline)
                    DaemonConnectionStatusView(status: status)
                        .font(.caption)
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

}

struct PulseModifier: ViewModifier {
    let active: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulsing = false

    func body(content: Content) -> some View {
        content
            .scaleEffect(pulsing ? 1.3 : 1.0)
            .animation(
                active && !reduceMotion
                    ? .easeInOut(duration: 0.8).repeatForever(autoreverses: true)
                    : nil,
                value: pulsing
            )
            .onAppear { pulsing = active && !reduceMotion }
            .onChange(of: active) { _, isActive in
                pulsing = isActive && !reduceMotion
            }
            .onChange(of: reduceMotion) { _, isReduced in
                pulsing = active && !isReduced
            }
    }
}

// MARK: - Add Daemon Sheet

struct AddDaemonView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme
    let onAdd: (DialInfo, String) -> String?

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
                        .font(.mewMono(15))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Name (e.g. Homelab)", text: $name)
                }
                .listRowBackground(theme.card)

                Section {
                    Button {
                        showScanner = true
                    } label: {
                        Label("Scan QR Code", systemImage: "qrcode.viewfinder")
                    }
                }
                .listRowBackground(theme.card)

                Section {
                    Text("Run `mew pair` on your daemon machine, then paste the Node ID or scan the QR code.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .listRowBackground(theme.card)

                if let error {
                    Section {
                        Text(error)
                            .foregroundStyle(.red)
                    }
                    .listRowBackground(theme.card)
                }
            }
            .mewThemedForm(theme)
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
            if let error = onAdd(info, displayName) {
                self.error = error
                return
            }
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// Scanner sheet wrapping the camera view with a dismiss button.
struct QRScannerSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var store: AppStore
    let onScan: (String) -> Void

    var body: some View {
        NavigationStack {
            QRScannerView { payload in
                if store.phoneSettings.haptics {
                    UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                }
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
