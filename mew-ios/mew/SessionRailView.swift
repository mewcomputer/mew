import SwiftUI
import MewMobileCore

/// Chronological conversations from saved machines, optionally filtered by daemon.
///
/// Recent messages and new alerts determine chronological order; pending
/// permissions/questions remain visible as badges. Archives are optional.
struct SessionRailView: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @State private var showArchived = false
    @State private var sessionToDelete: SessionTimelineItem?
    @State private var sessionToRename: SessionTimelineItem?
    @State private var renameText = ""
    @State private var showingProjectPicker = false
    @State private var pickerCwd = ""
    @State private var pickerError: String?
    @State private var creationDaemonId: String?

    private var daemonNodeId: String? { store.sessionFilter }

    private var status: DaemonStatus {
        daemonNodeId.flatMap { store.daemonStatuses[$0] } ?? .disconnected
    }

    private var isConnected: Bool {
        if case .connected = status { return true }
        return false
    }

    private var needsPairing: Bool {
        if case .pairedLost = status { return true }
        return false
    }

    var body: some View {
        sessionList
        .onChange(of: daemonNodeId) { _, nodeId in
            store.refreshSessions(daemonNodeId: nodeId)
        }
        .safeAreaInset(edge: .top, spacing: 0) { filters }
        .navigationTitle("Chats")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                NavigationLink(value: NavigationRoute.settings) {
                    Image(systemName: "gearshape")
                }
                .accessibilityLabel("App settings")
            }
            ToolbarItemGroup(placement: .topBarTrailing) {
                NavigationLink(value: NavigationRoute.machines) {
                    Image(systemName: "server.rack")
                }
                .accessibilityLabel("Machines")
                Button {
                    creationDaemonId = nil
                    pickerError = nil
                    pickerCwd = ""
                    if let daemonNodeId, connected(daemonNodeId) {
                        chooseCreationMachine(daemonNodeId)
                    }
                    showingProjectPicker = true
                } label: {
                    Image(systemName: "square.and.pencil")
                }
                .accessibilityLabel("New chat")
                .disabled(daemonNodeId.map { !connected($0) }
                    ?? !store.daemons.contains { connected($0.nodeId) })
            }
        }
        .alert("Delete Session", isPresented: Binding(
            get: { sessionToDelete != nil },
            set: { if !$0 { sessionToDelete = nil } }
        )) {
            Button("Cancel", role: .cancel) { sessionToDelete = nil }
            Button("Delete", role: .destructive) {
                if let item = sessionToDelete {
                    store.deleteSession(item.session.sessionId, daemonNodeId: item.daemonNodeId)
                }
                sessionToDelete = nil
            }
        } message: {
            if let session = sessionToDelete {
                Text("Delete \"\(displayName(session.session))\" on \(machineName(session.daemonNodeId))? This cannot be undone.")
            }
        }
        .alert("Rename Session", isPresented: Binding(
            get: { sessionToRename != nil },
            set: { if !$0 { sessionToRename = nil } }
        )) {
            TextField("Title", text: $renameText)
            Button("Save") {
                if let item = sessionToRename {
                    let trimmed = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !trimmed.isEmpty {
                        store.renameSession(item.session.sessionId, daemonNodeId: item.daemonNodeId, title: trimmed)
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
            if let creationDaemonId {
                ProjectPickerSheet(
                    projects: store.projectLists[creationDaemonId] ?? [],
                    loading: store.projectsLoading.contains(creationDaemonId),
                    error: pickerError,
                    machineName: machineName(creationDaemonId),
                    cwd: $pickerCwd,
                    onPick: { cwd in
                        if store.newSession(cwd: cwd, daemonNodeId: creationDaemonId) {
                            showingProjectPicker = false
                        } else {
                            pickerError = "Connect to this machine before creating a chat."
                        }
                    },
                    onChangeMachine: {
                        self.creationDaemonId = nil
                        pickerError = nil
                    },
                    onClose: { showingProjectPicker = false }
                )
            } else {
                NavigationStack {
                    List(store.daemons, id: \.nodeId) { daemon in
                        Button {
                            chooseCreationMachine(daemon.nodeId)
                        } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(daemon.name).foregroundStyle(.primary)
                                DaemonConnectionStatusView(status: store.daemonStatuses[daemon.nodeId] ?? .disconnected)
                                    .font(.caption)
                            }
                            .frame(minHeight: 44)
                        }
                        .disabled(!connected(daemon.nodeId))
                    }
                    .navigationTitle("Choose a machine")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { showingProjectPicker = false }
                        }
                    }
                }
            }
        }
    }

    private var filters: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    filter("All", daemon: nil).id("")
                    ForEach(store.daemons, id: \.nodeId) { daemon in
                        filter(daemon.name, daemon: daemon.nodeId).id(daemon.nodeId)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
            }
            .scrollIndicators(.hidden)
            .background(.bar)
            .onChange(of: daemonNodeId, initial: true) { _, nodeId in
                proxy.scrollTo(nodeId ?? "", anchor: .center)
            }
        }
    }

    private func filter(_ title: String, daemon: String?) -> some View {
        let selected = daemonNodeId == daemon
        return Button {
            store.sessionFilter = daemon
        } label: {
            Text(title)
                .font(.subheadline.weight(selected ? .semibold : .regular))
                .foregroundStyle(selected ? Color.accentColor : .primary)
                .padding(.horizontal, 16)
                .frame(minHeight: 44)
                .background(selected ? Color.accentColor.opacity(0.12) : Color(.secondarySystemFill), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityLabel(daemon.map { "Chats on \(machineName($0))" } ?? "Chats on all machines")
    }

    private func chooseCreationMachine(_ nodeId: String) {
        creationDaemonId = nodeId
        pickerCwd = ""
        pickerError = nil
        store.fetchProjects(daemonNodeId: nodeId)
    }

    // MARK: - Conversation list

    @ViewBuilder
    private var sessionList: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            let sections = store.sessionTimeline(daemonNodeId: daemonNodeId, includeArchived: showArchived, now: context.date)
            List {
                connectionNotice
                if store.daemons.isEmpty {
                    ContentUnavailableView {
                        Label("No machines", systemImage: "server.rack")
                    } description: {
                        Text("Pair a machine to open its conversations.")
                    } actions: {
                        NavigationLink("Pair a machine", value: NavigationRoute.machines)
                    }
                    .listRowSeparator(.hidden)
                }
                if hasArchived {
                    Section {
                        Toggle(isOn: $showArchived) {
                            Label("Show archived", systemImage: "archivebox")
                        }
                    }
                }
                if sections.isEmpty && !store.daemons.isEmpty {
                    emptySessions.listRowSeparator(.hidden)
                }
                ForEach(sections) { section in
                    Section {
                        ForEach(section.items) { item in
                            sessionRow(item)
                                .listRowInsets(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
                                .listRowSeparator(.hidden)
                        }
                    } header: {
                        Text(section.period.title)
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(.primary)
                            .textCase(nil)
                            .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 4, trailing: 16))
                    }
                    .listSectionSeparator(.hidden)
                }
            }
            .listStyle(.plain)
            .listSectionSpacing(8)
            .environment(\.defaultMinListRowHeight, 44)
            .environment(\.defaultMinListHeaderHeight, 28)
            .refreshable {
                if let daemonNodeId, !connected(daemonNodeId) {
                    store.retryConnection(daemonNodeId: daemonNodeId)
                } else {
                    store.refreshSessions(daemonNodeId: daemonNodeId)
                }
            }
        }
    }

    private var emptySessions: some View {
        ContentUnavailableView(hasArchived && !showArchived ? "No active chats" : "No chats",
            systemImage: "bubble.left.and.bubble.right",
            description: Text(hasArchived && !showArchived
                ? "Turn on Show archived to see your archived conversations."
                : "Pull down to refresh, or tap + to start a conversation."))
    }

    // MARK: - Connection state

    @ViewBuilder
    private var connectionNotice: some View {
        if let daemonNodeId, !isConnected {
            VStack(alignment: .leading, spacing: 6) {
                Label(statusTitle, systemImage: statusIcon).font(.callout.weight(.medium))
                Text("Showing previously loaded chats. Reconnect to send or change conversations.")
                    .font(.caption).foregroundStyle(.secondary)
                HStack(spacing: 20) {
                    NavigationLink("Connection details", value: NavigationRoute.daemonDetails(daemonNodeId: daemonNodeId))
                    if !needsPairing {
                        Button("Retry") { store.retryConnection(daemonNodeId: daemonNodeId) }
                    }
                }
                .font(.callout)
                .buttonStyle(.borderless)
                .frame(minHeight: 44)
            }
            .listRowSeparator(.hidden)
        } else if daemonNodeId == nil {
            let unavailable = store.daemons.filter { !connected($0.nodeId) }
            if !unavailable.isEmpty {
                NavigationLink(value: NavigationRoute.machines) {
                    Label(unavailable.count == 1 ? "1 machine isn't connected" : "\(unavailable.count) machines aren't connected", systemImage: "wifi.slash")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                .listRowSeparator(.hidden)
            }
        }
    }

    // MARK: - Row

    @ViewBuilder
    private func sessionRow(_ item: SessionTimelineItem) -> some View {
        let session = item.session
        let daemonNodeId = item.daemonNodeId
        NavigationLink(value: NavigationRoute.chat(
            daemonNodeId: daemonNodeId,
            sessionId: session.sessionId
        )) {
            sessionContent(item)
        }
        .swipeActions(edge: .leading, allowsFullSwipe: true) {
            Button {
                store.pinSession(session.sessionId, daemonNodeId: daemonNodeId, pinned: !session.pinned)
            } label: {
                Label(session.pinned ? "Unpin" : "Pin",
                      systemImage: session.pinned ? "pin.slash" : "pin")
            }
            .tint(.yellow)
            .disabled(!connected(daemonNodeId))
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            Button(role: .destructive) {
                sessionToDelete = item
            } label: {
                Label("Delete", systemImage: "trash")
            }
            .disabled(!connected(daemonNodeId))
            Button {
                renameText = session.title.isEmpty ? displayName(session) : session.title
                sessionToRename = item
            } label: {
                Label("Rename", systemImage: "pencil")
            }
            .tint(.blue)
            .disabled(!connected(daemonNodeId))
            Button {
                store.archiveSession(session.sessionId, daemonNodeId: daemonNodeId,
                                     archived: !session.archived)
            } label: {
                Label(session.archived ? "Unarchive" : "Archive",
                      systemImage: session.archived ? "tray.and.arrow.up"
                                                   : "archivebox")
            }
            .tint(.indigo)
            .disabled(!connected(daemonNodeId))
        }
    }

    @ViewBuilder
    private func sessionContent(_ item: SessionTimelineItem) -> some View {
        let session = item.session
        HStack(spacing: 8) {
            stateIndicator(connected(item.daemonNodeId) ? session.state : "offline")

            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    if session.pinned {
                        Image(systemName: "pin.fill")
                            .font(.caption2)
                            .foregroundStyle(.yellow)
                    }
                    Text(displayName(session))
                        .font(.body)
                        .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 1)
                        .truncationMode(.tail)
                }

                let metadataLayout = dynamicTypeSize.isAccessibilitySize
                    ? AnyLayout(VStackLayout(alignment: .leading, spacing: 4))
                    : AnyLayout(HStackLayout(spacing: 8))
                metadataLayout {
                    Text(machineName(item.daemonNodeId))
                        .foregroundStyle(connected(item.daemonNodeId) ? Color.secondary : .orange)
                        .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 1)
                    if let cwd = session.cwd, !cwd.isEmpty {
                        Label((cwd as NSString).lastPathComponent, systemImage: "folder")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 1)
                    }
                    stateLabel(connected(item.daemonNodeId) ? session.state : "offline")
                        .font(.caption)
                        .foregroundStyle(stateColor(connected(item.daemonNodeId) ? session.state : "offline"))

                    if session.usageCost > 0 {
                        Label(costString(session.usageCost),
                              systemImage: "dollarsign.circle")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    if let pct = contextPercent(session, daemonNodeId: item.daemonNodeId) {
                        Label("\(pct)% ctx", systemImage: "memorychip")
                            .font(.caption)
                            .foregroundStyle(pct >= 90 ? .orange : .secondary)
                    }
                }
                .font(.caption)
            }

            Spacer(minLength: 4)

            pendingBadges(session)
        }
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
        case "offline": return Text("Offline")
        default:        return Text(state.capitalized)
        }
    }

    private func stateColor(_ state: String) -> Color {
        switch state {
        case "running": return .green
        case "active":  return .green.opacity(0.8)
        case "idle":    return .secondary
        case "offline": return .orange
        default:        return .secondary
        }
    }

    // MARK: - Status strings

    private var statusTitle: String {
        switch status {
        case .connected:                return "Connected"
        case .connecting:                return "Connecting…"
        case .backoff(let attempt, _):  return "Reconnecting (attempt \(attempt))"
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

    // MARK: - Helpers

    private var hasArchived: Bool {
        let machines = daemonNodeId.map { [$0] } ?? store.daemons.map(\.nodeId)
        return machines.contains { (store.sessionLists[$0] ?? []).contains { $0.archived } }
    }

    private func connected(_ nodeId: String) -> Bool {
        if case .connected? = store.daemonStatuses[nodeId] { return true }
        return false
    }

    private func machineName(_ nodeId: String) -> String {
        store.daemons.first { $0.nodeId == nodeId }?.name ?? "Machine"
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

    /// Context occupancy as a percent of the session model's window.
    /// Matches the session's model against the daemon's model list to find
    /// the window; nil when either the reading or the window is unknown.
    private func contextPercent(_ session: SessionSummary, daemonNodeId: String) -> Int? {
        guard let used = session.contextTokens, used > 0,
              let window = contextWindow(for: session, daemonNodeId: daemonNodeId), window > 0 else {
            return nil
        }
        return Int((Double(used) / Double(window) * 100).rounded())
    }

    private func contextWindow(for session: SessionSummary, daemonNodeId: String) -> Int64? {
        guard let model = session.model else { return nil }
        return store.modelsByDaemon[daemonNodeId]?.first { m in
            if let provider = session.provider {
                return m.provider == provider && m.model == model
            }
            return m.model == model
        }?.contextWindow
    }

}

// MARK: - Project picker sheet

struct ProjectPickerSheet: View {
    let projects: [ProjectInfo]
    let loading: Bool
    let error: String?
    let machineName: String
    @Binding var cwd: String
    let onPick: (String) -> Void
    let onChangeMachine: () -> Void
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
            .navigationTitle("New chat")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    VStack(spacing: 2) {
                        Text("New chat").font(.headline)
                        Text(machineName).font(.caption).foregroundStyle(.secondary)
                    }
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button("Machines", action: onChangeMachine)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Cancel", action: onClose)
                }
            }
        }
    }
}
