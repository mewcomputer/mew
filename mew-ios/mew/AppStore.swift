import SwiftUI
import MewMobileCore

// MARK: - Font preference

enum MewFontChoice: String, CaseIterable {
    case system = "System"
    case miSans = "Mi Sans"
    case junicode = "Junicode"
    case goudy = "OFL Goudy"

    var displayName: String { rawValue }

    /// Apply this font choice to the UIKit appearance.
    func apply() {
        switch self {
        case .system:
            UILabel.appearance().font = UIFont.systemFont(ofSize: UIFont.systemFontSize)
        case .miSans:
            if MewFonts.sansAvailable {
                UILabel.appearance().font = UIFont(name: "MiSans", size: UIFont.systemFontSize)
            }
        case .junicode:
            if MewFonts.serifAvailable {
                UILabel.appearance().font = UIFont(name: "JunicodeVF-Roman", size: UIFont.systemFontSize)
            }
        case .goudy:
            if MewFonts.goudyAvailable {
                UILabel.appearance().font = UIFont(name: "OFLGoudyStMTT", size: UIFont.systemFontSize)
            }
        }
    }

    /// The SwiftUI font for a given size.
    func swiftUIFont(_ size: CGFloat) -> Font {
        switch self {
        case .system:       return .system(size: size)
        case .miSans:       return .mewSans(size)
        case .junicode:      return .mewSerif(size)
        case .goudy:         return .mewGoudy(size)
        }
    }

    /// Preview text in the picker.
    var previewText: String {
        switch self {
        case .system:   return "The quick brown fox"
        case .miSans:    return "The quick brown fox"
        case .junicode:  return "The quick brown fox"
        case .goudy:     return "The quick brown fox"
        }
    }
}

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
    @Published var projectLists: [String: [ProjectInfo]] = [:]
    @Published var directoryListings: [String: DirListing] = [:]
    @Published var directoryLoading: Set<String> = []
    @Published var projectsLoading: Set<String> = []

    // Navigation stack, driven both by NavigationLinks and programmatically
    // (e.g. to push a freshly created session).
    @Published var path: [NavigationRoute] = []

    // Active daemon + session
    @Published var selectedDaemonId: DaemonId?
    @Published var selectedSessionId: String?

    // Node id of the daemon we just asked for a new session on, so the
    // resulting SessionReloaded can navigate into it. Cleared once consumed.
    private var pendingNewSessionDaemon: String?

    // Chat state for the active session
    @Published var messages: [ChatMessage] = []

    /// Messages filtered to exclude empty entries (no visible content).
    var visibleMessages: [ChatMessage] {
        messages.filter { msg in
            msg.parts.contains { part in
                switch part.kind {
                case .text:
                    return (part.text?.isEmpty == false)
                case .reasoning:
                    return (part.text?.isEmpty == false)
                case .toolCall:
                    return part.toolName != nil
                case .error:
                    return (part.text?.isEmpty == false)
                }
            }
        }
    }
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

    // Font preference (persisted to UserDefaults)
    @Published var fontChoice: MewFontChoice = MewFontChoice(rawValue: UserDefaults.standard.string(forKey: "mew.fontChoice") ?? "Mi Sans") ?? .miSans {
        didSet {
            UserDefaults.standard.set(fontChoice.rawValue, forKey: "mew.fontChoice")
        }
    }

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

            // Auto-connect to all known daemons
            for daemon in self.daemons {
                let id = DaemonId(nodeId: daemon.nodeId)
                self.connect(daemonId: id)
            }
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

    func newSession(cwd: String? = nil) {
        guard let daemonId = selectedDaemonId, let core else { return }
        pendingNewSessionDaemon = daemonId.nodeId
        core.newSession(id: daemonId, cwd: cwd)
    }

    func fetchProjects() {
        guard let daemonId = selectedDaemonId, let core else { return }
        projectsLoading.insert(daemonId.nodeId)
        core.listProjects(id: daemonId)
    }

    /// Browse a directory on the daemon. The result arrives as a
    /// `CoreEvent::DirListing` which the `onEvent` handler stores in
    /// `directoryListings`.
    func fetchDirectory(sessionId: String, path: String?) {
        guard let daemonId = selectedDaemonId, let core else { return }
        let key = directoryKey(sessionId: sessionId, path: path)
        directoryLoading.insert(key)
        core.listDir(id: daemonId, sessionId: sessionId, path: path)
    }

    private func directoryKey(sessionId: String, path: String?) -> String {
        let p = path ?? ""
        return "\(sessionId)::\(p)"
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
                // Auto-list sessions for any daemon that connects
                if let core {
                    let id = DaemonId(nodeId: daemon)
                    core.listSessions(id: id)
                }
            }

        case .daemonVersion(let daemon, let version):
            daemonVersions[daemon] = version

        case .sessionList(let daemon, let sessions):
            sessionLists[daemon] = sessions

        case .projectList(let daemon, let projects):
            projectLists[daemon] = projects
            projectsLoading.remove(daemon)
            objectWillChange.send()

        case .dirListing(let daemon, let sessionId, let path, let entries):
            let key = directoryKey(sessionId: sessionId, path: path)
            directoryListings[key] = DirListing(
                daemon: daemon,
                sessionId: sessionId,
                path: path,
                entries: entries
            )
            directoryLoading.remove(key)
            objectWillChange.send()

        case .sessionReloaded(let daemon, let sessionId):
            // Pull snapshot after reload
            if let snap = core?.snapshot(id: DaemonId(nodeId: daemon)) {
                applySnapshot(snap)
            }
            // If we just created this session via "new chat", navigate into it.
            if pendingNewSessionDaemon == daemon {
                pendingNewSessionDaemon = nil
                selectedSessionId = sessionId
                path.append(.chat(daemonNodeId: daemon, sessionId: sessionId))
            } else if selectedSessionId == nil {
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

// MARK: - Directory listing

/// A single directory listing fetched from the daemon. The view layer
/// treats path as the canonical key alongside sessionId.
struct DirListing {
    let daemon: String
    let sessionId: String
    let path: String
    let entries: [DirEntry]
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
