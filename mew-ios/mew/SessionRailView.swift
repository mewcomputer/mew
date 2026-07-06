import SwiftUI
import MewMobileCore

/// Session rail: lists sessions for a single daemon, mirroring the web UI.
///
/// Sorted by "needs attention": pending permissions/questions first, then
/// running, active, idle. Archived sessions are hidden behind a toggle.
struct SessionRailView: View {
    let daemonNodeId: String

    @EnvironmentObject private var store: AppStore
    @State private var showArchived = false
    @State private var sessionToDelete: SessionSummary?
    @State private var sessionToRename: SessionSummary?
    @State private var renameText = ""
    @State private var showingProjectPicker = false
    @State private var pickerCwd = ""
    @State private var pickerError: String?

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

    private var sessions: [SessionSummary] {
        let all = store.sessionLists[daemonNodeId] ?? []
        let filtered = showArchived ? all : all.filter { !$0.archived }
        return filtered.sorted { lhs, rhs in attentionRank(lhs) < attentionRank(rhs) }
    }

    var body: some View {
        Group {
            if isConnected {
                sessionList
            } else {
                connectingState
            }
        }
        .navigationTitle(navigationTitle)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showingProjectPicker = true
                    pickerError = nil
                    pickerCwd = ""
                    store.fetchProjects()
                } label: {
                    Image(systemName: "plus")
                }
                .disabled(!isConnected)
            }
        }
        .onAppear {
            ensureDaemonSelected()
        }
        .alert("Delete Session", isPresented: Binding(
            get: { sessionToDelete != nil },
            set: { if !$0 { sessionToDelete = nil } }
        )) {
            Button("Cancel", role: .cancel) { sessionToDelete = nil }
            Button("Delete", role: .destructive) {
                if let session = sessionToDelete {
                    store.deleteSession(session.sessionId)
                }
                sessionToDelete = nil
            }
        } message: {
            if let session = sessionToDelete {
                Text("Delete \"\(displayName(session))\"? This cannot be undone.")
            }
        }
        .alert("Rename Session", isPresented: Binding(
            get: { sessionToRename != nil },
            set: { if !$0 { sessionToRename = nil } }
        )) {
            TextField("Title", text: $renameText)
            Button("Save") {
                if let session = sessionToRename {
                    let trimmed = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !trimmed.isEmpty {
                        store.renameSession(session.sessionId, title: trimmed)
                    }
                }
                sessionToRename = nil
                renameText = ""
            }
            Button("Cancel", role: .cancel) {
                sessionToRename = nil
                renameText = ""
            }
        }
        .sheet(isPresented: $showingProjectPicker) {
            ProjectPickerSheet(
                projects: store.projectLists[daemonNodeId] ?? [],
                loading: store.projectsLoading.contains(daemonNodeId),
                error: pickerError,
                cwd: $pickerCwd,
                onPick: { cwd in
                    showingProjectPicker = false
                    store.newSession(cwd: cwd)
                },
                onClose: { showingProjectPicker = false }
            )
        }
    }

    // MARK: - Connected list

    @ViewBuilder
    private var sessionList: some View {
        if sessions.isEmpty {
            ContentUnavailableView(
                showArchived ? "No Archived Sessions" : "No Sessions",
                systemImage: "bubble.left.and.bubble.right",
                description: Text(showArchived
                    ? "Archived sessions will appear here."
                    : "Tap + to start a new conversation with this daemon.")
            )
        } else {
            List {
                Section {
                    ForEach(sessions, id: \.sessionId) { session in
                        sessionRow(session)
                    }
                } header: {
                    if hasArchived {
                        Toggle(isOn: $showArchived) {
                            Label("Show Archived", systemImage: "archivebox")
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .refreshable {
                // The store owns session listing via CoreEvents; nothing to
                // call directly here. The control is shown for parity with
                // the web UI's pull-to-refresh affordance.
            }
        }
    }

    // MARK: - Connecting / disconnected

    @ViewBuilder
    private var connectingState: some View {
        ScrollView {
            ContentUnavailableView {
                Label(statusTitle, systemImage: statusIcon)
                    .font(.title2)
            } description: {
                Text(statusDescription)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity)
            .padding(.top, 80)
        }
        .refreshable {
            // Force a reconnect attempt
            if let id = store.selectedDaemonId {
                store.connect(daemonId: id)
            } else {
                ensureDaemonSelected()
                if let id = store.selectedDaemonId {
                    store.connect(daemonId: id)
                }
            }
        }
    }

    // MARK: - Row

    @ViewBuilder
    private func sessionRow(_ session: SessionSummary) -> some View {
        NavigationLink(value: NavigationRoute.chat(
            daemonNodeId: daemonNodeId,
            sessionId: session.sessionId
        )) {
            sessionContent(session)
        }
        .swipeActions(edge: .leading, allowsFullSwipe: true) {
            Button {
                store.pinSession(session.sessionId, pinned: !session.pinned)
            } label: {
                Label(session.pinned ? "Unpin" : "Pin",
                      systemImage: session.pinned ? "pin.slash" : "pin")
            }
            .tint(.yellow)
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            Button(role: .destructive) {
                sessionToDelete = session
            } label: {
                Label("Delete", systemImage: "trash")
            }
            Button {
                renameText = session.title.isEmpty ? displayName(session) : session.title
                sessionToRename = session
            } label: {
                Label("Rename", systemImage: "pencil")
            }
            .tint(.blue)
            Button {
                store.archiveSession(session.sessionId,
                                     archived: !session.archived)
            } label: {
                Label(session.archived ? "Unarchive" : "Archive",
                      systemImage: session.archived ? "tray.and.arrow.up"
                                                   : "archivebox")
            }
            .tint(.indigo)
        }
    }

    @ViewBuilder
    private func sessionContent(_ session: SessionSummary) -> some View {
        HStack(spacing: 10) {
            stateIndicator(session.state)

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    if session.pinned {
                        Image(systemName: "pin.fill")
                            .font(.caption2)
                            .foregroundStyle(.yellow)
                    }
                    Text(displayName(session))
                        .font(.body)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                HStack(spacing: 8) {
                    stateLabel(session.state)
                        .font(.caption)
                        .foregroundStyle(stateColor(session.state))

                    if session.usageCost > 0 {
                        Label(costString(session.usageCost),
                              systemImage: "dollarsign.circle")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            Spacer()

            pendingBadges(session)
        }
        .padding(.vertical, 2)
    }

    // MARK: - Badges

    @ViewBuilder
    private func pendingBadges(_ session: SessionSummary) -> some View {
        let perms = Int(session.pendingPermissions)
        let questions = Int(session.pendingQuestions)

        HStack(spacing: 4) {
            if perms > 0 {
                badge(count: perms, color: .orange, icon: "hand.raised")
            }
            if questions > 0 {
                badge(count: questions, color: .blue, icon: "questionmark.bubble")
            }
        }
    }

    @ViewBuilder
    private func badge(count: Int, color: Color, icon: String) -> some View {
        Label("\(count)", systemImage: icon)
            .font(.caption.bold())
            .foregroundStyle(.white)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(color, in: Capsule())
    }

    // MARK: - State indicators

    @ViewBuilder
    private func stateIndicator(_ state: String) -> some View {
        switch state {
        case "running":
            Image(systemName: "circle.fill")
                .font(.system(size: 8))
                .foregroundStyle(.green)
                .modifier(PulseModifier(active: true))
        case "active":
            Circle()
                .fill(.green.opacity(0.8))
                .frame(width: 8, height: 8)
        case "idle":
            Circle()
                .stroke(.gray.opacity(0.6), lineWidth: 1.5)
                .frame(width: 8, height: 8)
        default:
            Circle()
                .fill(.gray.opacity(0.4))
                .frame(width: 8, height: 8)
        }
    }

    private func stateLabel(_ state: String) -> Text {
        switch state {
        case "running": return Text("Running")
        case "active":  return Text("Active")
        case "idle":    return Text("Idle")
        default:        return Text(state.capitalized)
        }
    }

    private func stateColor(_ state: String) -> Color {
        switch state {
        case "running": return .green
        case "active":  return .green.opacity(0.8)
        case "idle":    return .secondary
        default:        return .secondary
        }
    }

    // MARK: - Status strings

    private var navigationTitle: String {
        daemon?.name ?? String(daemonNodeId.prefix(12))
    }

    private var statusTitle: String {
        switch status {
        case .connected:                return "Connected"
        case .connecting:                return "Connecting…"
        case .backoff(let attempt):      return "Reconnecting (attempt \(attempt))"
        case .pairedLost:               return "Lost Pairing"
        case .disconnected:             return "Disconnected"
        }
    }

    private var statusIcon: String {
        switch status {
        case .connected:     return "checkmark.circle.fill"
        case .connecting:    return "arrow.triangle.2.circlepath.circle"
        case .backoff:       return "exclamationmark.arrow.circlepath"
        case .pairedLost:    return "exclamationmark.triangle.fill"
        case .disconnected:  return "wifi.slash"
        }
    }

    private var statusDescription: String {
        switch status {
        case .connected:     return "Session list unavailable."
        case .connecting:    return "Establishing a connection to \(navigationTitle)."
        case .backoff:       return "Waiting to retry. The daemon will reconnect automatically."
        case .pairedLost:    return "This daemon is no longer paired. Re-pair from the daemons list."
        case .disconnected:  return "Not connected to \(navigationTitle). Pull down to retry."
        }
    }

    // MARK: - Helpers

    private var hasArchived: Bool {
        (store.sessionLists[daemonNodeId] ?? []).contains { $0.archived }
    }

    private func displayName(_ session: SessionSummary) -> String {
        if session.title.isEmpty {
            let id = session.sessionId
            return id.count > 8 ? String(id.prefix(8)) : id
        }
        return session.title
    }

    private func costString(_ cost: Double) -> String {
        if cost < 0.01 {
            return String(format: "$%.4f", cost)
        }
        return String(format: "$%.2f", cost)
    }

    /// Lower rank sorts first: pending > running > active > idle.
    private func attentionRank(_ session: SessionSummary) -> Int {
        if session.pendingPermissions > 0 || session.pendingQuestions > 0 {
            return 0
        }
        switch session.state {
        case "running": return 1
        case "active":  return 2
        case "idle":    return 3
        default:       return 4
        }
    }

    /// Ensure the store's `selectedDaemonId` matches this rail so that
    /// `newSession` / `selectSession` / archive / pin / delete succeed.
    private func ensureDaemonSelected() {
        if store.selectedDaemonId?.nodeId != daemonNodeId {
            store.selectedDaemonId = DaemonId(nodeId: daemonNodeId)
        }
    }
}

// MARK: - Project picker sheet

struct ProjectPickerSheet: View {
    let projects: [ProjectInfo]
    let loading: Bool
    let error: String?
    @Binding var cwd: String
    let onPick: (String) -> Void
    let onClose: () -> Void

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let error {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .padding(.horizontal)
                }

                if loading {
                    Spacer()
                    ProgressView().padding()
                    Spacer()
                } else if projects.isEmpty {
                    Spacer()
                    Text("No recent projects. Enter a path below.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding()
                    Spacer()
                } else {
                    List {
                        ForEach(projects, id: \.path) { project in
                            Button {
                                onPick(project.path)
                            } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(project.displayName)
                                        .font(.body)
                                        .foregroundStyle(.primary)
                                    Text(project.path)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                        .truncationMode(.middle)
                                    Text("\(project.sessionCount) session\(project.sessionCount == 1 ? "" : "s")")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                    .listStyle(.plain)
                }

                Divider()

                VStack(alignment: .leading, spacing: 6) {
                    Text("Or enter a path")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    HStack {
                        TextField("/path/to/project", text: $cwd)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .padding(8)
                            .background(Color(.systemGray6), in: RoundedRectangle(cornerRadius: 6))
                        Button("Open") {
                            let trimmed = cwd.trimmingCharacters(in: .whitespacesAndNewlines)
                            if !trimmed.isEmpty { onPick(trimmed) }
                        }
                        .disabled(cwd.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
                .padding()
            }
            .navigationTitle("New session")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Cancel", action: onClose)
                }
            }
        }
    }
}
