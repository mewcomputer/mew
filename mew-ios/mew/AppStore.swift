import SwiftUI
import MewMobileCore

// MARK: - App Store

/// The single source of truth for the app, mirroring the web UI's session store.
/// All CoreEvents are funneled through here; views subscribe via @Published.
@MainActor
final class AppStore: ObservableObject {
    @Published var daemons: [DaemonEntry] = []
    @Published var daemonStatuses: [String: DaemonStatus] = [:]
    @Published var daemonVersions: [String: String] = [:]

    // Per-daemon session lists
    @Published var sessionLists: [String: [SessionSummary]] = [:]

    // Active daemon + session
    @Published var selectedDaemonId: DaemonId?
    @Published var selectedSessionId: String?

    // Chat state for the active session
    @Published var messages: [ChatMessage] = []
    @Published var streamingText: String = ""
    @Published var streamingPartId: String?
    @Published var isStreaming: Bool = false

    // Pending requests
    @Published var pendingPermissions: [PendingPermission] = []
    @Published var pendingAskUser: [PendingAskUser] = []

    // Models
    @Published var availableModels: [ModelSummary] = []

    // Alerts
    @Published var alerts: [AlertItem] = []

    // Connection
    @Published var isConnecting: Bool = false

    // The core
    private var core: MobileCore?
    private var listener: CoreListenerBridge?

    // MARK: - Lifecycle

    func start() {
        guard core == nil else { return }
        Task {
            await initializeCore()
        }
    }

    private func initializeCore() async {
        // Load or create the phone's persistent secret key from keychain.
        let keyBytes = KeychainHelper.loadOrCreateSecretKey()
        let dataDir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .path

        do {
            let core = try await MobileCore(secretKeyBytes: keyBytes, dataDir: dataDir)
            let listener = CoreListenerBridge(store: self)
            core.setListener(listener: listener)
            self.core = core
            self.listener = listener
            self.daemons = core.listDaemons()
        } catch {
            print("Failed to initialize mobile core: \(error)")
        }
    }

    // MARK: - Daemon management

    var phoneNodeId: String {
        core?.nodeId() ?? ""
    }

    func addDaemon(nodeId: String, name: String) {
        guard let core else { return }
        let id = core.addDaemon(nodeId: nodeId, name: name)
        daemons = core.listDaemons()
        connect(daemonId: id)
    }

    func removeDaemon(_ daemon: DaemonEntry) {
        guard let core else { return }
        let id = DaemonId(nodeId: daemon.nodeId)
        core.removeDaemon(id: id)
        daemons = core.listDaemons()
    }

    func connect(daemonId: DaemonId) {
        guard let core else { return }
        core.connect(id: daemonId)
    }

    func disconnect(daemonId: DaemonId) {
        guard let core else { return }
        core.disconnect(id: daemonId)
    }

    // MARK: - Session management

    func selectDaemon(_ daemon: DaemonEntry) {
        selectedDaemonId = DaemonId(nodeId: daemon.nodeId)
        if let core {
            core.listSessions(id: selectedDaemonId!)
        }
    }

    func selectSession(_ sessionId: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        selectedSessionId = sessionId
        core.attach(id: daemonId, sessionId: sessionId)

        // Pull snapshot for initial state
        if let snap = core.snapshot(id: daemonId) {
            applySnapshot(snap)
        }
    }

    func newSession() {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.newSession(id: daemonId, cwd: nil)
    }

    func sendPrompt(_ text: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.prompt(id: daemonId, text: text)
    }

    func cancelTurn() {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.cancel(id: daemonId)
    }

    func respondPermission(requestId: UInt64, decision: Decision) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.respondPermission(id: daemonId, requestId: requestId, decision: decision)
    }

    func respondAskUser(requestId: UInt64, answers: [String]) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.respondAskUser(id: daemonId, requestId: requestId, answers: answers)
    }

    func listModels() {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.listModels(id: daemonId)
    }

    func switchModel(provider: String, model: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.switchModel(id: daemonId, provider: provider, model: model)
    }

    func archiveSession(_ sessionId: String, archived: Bool) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.archiveSession(id: daemonId, sessionId: sessionId, archived: archived)
    }

    func pinSession(_ sessionId: String, pinned: Bool) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.pinSession(id: daemonId, sessionId: sessionId, pinned: pinned)
    }

    func deleteSession(_ sessionId: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.deleteSession(id: daemonId, sessionId: sessionId)
    }

    func renameSession(_ sessionId: String, title: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.renameSession(id: daemonId, sessionId: sessionId, title: title)
    }

    // MARK: - Event handling (called by CoreListenerBridge)

    func handleEvent(_ event: CoreEvent) {
        switch event {
        case .daemonStatusChanged(let daemon, let status):
            daemonStatuses[daemon] = status
            if case .connected = status {
                // Auto-list sessions on connect
                if let id = selectedDaemonId, id.nodeId == daemon, let core {
                    core.listSessions(id: id)
                }
            }

        case .daemonVersion(let daemon, let version):
            daemonVersions[daemon] = version

        case .sessionList(let daemon, let sessions):
            sessionLists[daemon] = sessions

        case .sessionReloaded(_, let sessionId):
            // Pull snapshot after reload
            guard let daemonId = selectedDaemonId, let core else { return }
            if let snap = core.snapshot(id: daemonId) {
                applySnapshot(snap)
            }
            if selectedSessionId == nil {
                selectedSessionId = sessionId
            }

        case .textDelta(_, _, let partId, let delta):
            if streamingPartId == partId {
                streamingText += delta
            } else {
                streamingPartId = partId
                streamingText = delta
            }

        case .partUpdated(_, _, let partId, _, _):
            // Update the message part in place
            updatePartInMessages(partId: partId)

        case .turnEnded(_, _, _, _, _, _):
            isStreaming = false
            streamingPartId = nil
            streamingText = ""
            // Refresh messages from snapshot
            refreshMessages()

        case .permissionRequested(_, _, let requestId, let toolName, let input):
            pendingPermissions.append(PendingPermission(
                requestId: requestId,
                sessionId: selectedSessionId ?? "",
                toolName: toolName,
                input: input
            ))

        case .askUserRequested(_, _, let requestId, let callId, let questions):
            pendingAskUser.append(PendingAskUser(
                requestId: requestId,
                sessionId: selectedSessionId ?? "",
                callId: callId,
                questions: questions
            ))

        case .requestResolved(_, let requestId):
            pendingPermissions.removeAll { $0.requestId == requestId }
            pendingAskUser.removeAll { $0.requestId == requestId }

        case .alert(_, let sessionId, let kind, let title, let detail):
            alerts.append(AlertItem(
                sessionId: sessionId,
                kind: kind,
                title: title,
                detail: detail
            ))

        case .attentionChanged:
            break

        case .todosUpdated:
            break

        case .modelList(_, let models):
            availableModels = models

        case .slashResult:
            break
        }
    }

    // MARK: - Helpers

    private func applySnapshot(_ snap: DaemonSnapshot) {
        if let session = snap.sessions.first {
            messages = session.messages
        }
    }

    private func refreshMessages() {
        guard let daemonId = selectedDaemonId, let core else { return }
        if let snap = core.snapshot(id: daemonId) {
            applySnapshot(snap)
        }
    }

    private func updatePartInMessages(partId: String) {
        // The Rust core already updated its internal state; pull a fresh snapshot
        refreshMessages()
    }
}

// MARK: - Alert item

struct AlertItem: Identifiable, Equatable {
    let id = UUID()
    let sessionId: String
    let kind: String
    let title: String
    let detail: String?
}

// MARK: - CoreListener Bridge

/// Bridges UniFFI CoreListener callbacks (which arrive on a background thread)
/// to the MainActor AppStore.
final class CoreListenerBridge: CoreListener, @unchecked Sendable {
    private weak var store: AppStore?

    init(store: AppStore) {
        self.store = store
    }

    func onEvent(event: CoreEvent) {
        Task { @MainActor in
            store?.handleEvent(event)
        }
    }
}
