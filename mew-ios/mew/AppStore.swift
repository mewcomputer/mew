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

    /// The PostScript font name for UIKit/AttributedString rendering, or `nil`
    /// for the system font (which has no single PostScript name).
    var uiFontName: String? {
        switch self {
        case .system:   return nil
        case .miSans:   return "MiSansLatinVF"
        case .junicode: return "JunicodeVF-Regular"
        case .goudy:    return "OFLGoudyStMTT"
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

struct PendingGoalProposal: Identifiable {
    let daemon: String
    let sessionId: String
    let requestId: String
    let objective: String

    var id: String { "\(daemon):\(sessionId):\(requestId)" }
}

struct WorkspaceReview {
    let daemon: String
    let sessionId: String
    let requestId: String
    var files: [ReviewFile] = []
    var isLoading = true
    var error: String?
}

struct WorkerHistory {
    let daemon: String
    let sessionId: String
    let childSessionId: String
    let requestId: String
    var messages: [ChatMessage] = []
    var isLoading = true
    var error: String?
}

struct JobOutputDetail {
    let daemon: String
    let sessionId: String
    let jobId: String
    var requestId = ""
    var output = ""
    var nextOffset: UInt64 = 0
    var isLoading = false
    var done = false
    var incomplete = false
    var error: String?
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
    @Published private var sessionAlertTimes: [String: [String: Int64]] = [:]
    @Published var projectLists: [String: [ProjectInfo]] = [:]
    @Published var directoryListings: [String: DirListing] = [:]
    @Published var directoryLoading: Set<String> = []
    @Published var projectsLoading: Set<String> = []

    // Navigation stack, driven both by NavigationLinks and programmatically
    // (e.g. to push a freshly created session).
    @Published var path: [NavigationRoute] = [] {
        didSet {
            if path != oldValue { pendingNewSessionDaemon = nil }
            synchronizeNavigation()
        }
    }

    // Active daemon + session
    @Published var selectedDaemonId: DaemonId?
    @Published var selectedSessionId: String?

    // Creation results navigate only while the originating location is still open.
    private var pendingNewSessionDaemon: String?

    // Chat state for the active session
    @Published var messages: [ChatMessage] = []
    @Published var isLoadingSession = false
    @Published var sessionLoadError: String?
    @Published var review: WorkspaceReview?
    @Published var historyBefore: UInt64?
    @Published var isLoadingOlderHistory = false
    @Published var historyError: String?
    @Published var subagents: [WorkerInfo] = []
    @Published var jobs: [BackgroundJob] = []
    @Published var workerHistory: WorkerHistory?
    @Published var jobOutput: JobOutputDetail?

    /// Completed content; preserve turn boundaries before hiding non-renderable messages.
    var conversationItems: [ConversationItem] {
        ConversationItem.grouped(messages: messages, hidingPartId: streamingText.isEmpty ? nil : streamingPartId)
    }
    @Published var streamingText: String = ""
    @Published var streamingPartId: String?
    @Published var isStreaming: Bool = false

    // Pending requests
    @Published var pendingPermissions: [PendingPermission] = []
    @Published var pendingAskUser: [PendingAskUser] = []
    @Published var pendingGoals: [PendingGoalProposal] = []

    // Models
    @Published var modelsByDaemon: [String: [ModelSummary]] = [:]
    @Published var daemonSettings: [String: AutomationSettings] = [:]
    @Published var daemonSettingsErrors: [String: String] = [:]
    @Published var managementSettings: [String: DaemonManagementSettings] = [:]
    @Published var managementErrors: [String: String] = [:]
    @Published private(set) var managementRequestIds: [String: String] = [:]
    private var managementTimeouts: [String: Task<Void, Never>] = [:]
    @Published var drafts: [NavigationRoute: String] = [:]
    private var settingsRequests: [String: Task<Void, Never>] = [:]
    private var removedDaemons: Set<String> = []
    let preferences: UserDefaults
    var pendingRestoredChat: NavigationRoute?

    // Permission mode, current model, thinking variant (per daemon)
    @Published var permissionMode: [String: String] = [:]
    @Published var currentModel: [String: String] = [:]
    @Published var currentProvider: [String: String] = [:]
    @Published var thinkingVariant: [String: String?] = [:]

    // Personas (per daemon)
    @Published var availablePersonas: [String: [PersonaInfo]] = [:]
    @Published var currentPersona: [String: String] = [:]

    // Session usage (per sessionId)
    @Published var sessionUsage: [String: SessionUsage] = [:]

    // Todos (per sessionId)
    @Published var todos: [String: [TodoItem]] = [:]

    // Alerts
    @Published var alerts: [AlertItem] = []
    @Published var activeBanner: AlertItem?
    @Published private(set) var feedback: PhoneFeedback?
    var isAppActive = false {
        didSet {
            if !isAppActive {
                activeBanner = nil
                feedback = nil
            }
        }
    }
    @Published var phoneSettings: PhoneSettings {
        didSet {
            phoneSettings.save(to: preferences)
            if !phoneSettings.haptics { feedback = nil }
            if let banner = activeBanner,
               !phoneSettings.allowsAlert(kind: banner.kind, daemon: banner.daemon) {
                activeBanner = nil
            }
        }
    }

    @Published private var pendingSubmissions: [String: Set<String>] = [:]

    // Retry: last prompt per session + per-session turn-failure tracking
    @Published var lastPrompt: [String: String] = [:]
    @Published var lastTurnFailed: [String: Bool] = [:]

    // Connection
    @Published var isConnecting: Bool = false

    // Font preference (persisted to UserDefaults)
    @Published var fontChoice: MewFontChoice {
        didSet {
            preferences.set(fontChoice.rawValue, forKey: "mew.fontChoice")
        }
    }

    // The core
    private var core: MobileCoreProtocol?
    private var listener: CoreListenerBridge?

    // MARK: - Lifecycle

    init(defaults: UserDefaults = .standard) {
        preferences = defaults
        phoneSettings = PhoneSettings(defaults: defaults)
        fontChoice = MewFontChoice(rawValue: defaults.string(forKey: "mew.fontChoice") ?? "Mi Sans") ?? .miSans
    }

    func start() {
        guard core == nil else { return }
        Task {
            await initializeCore()
        }
    }

    /// Test-only initializer: inject a mock core for unit testing.
    /// The mock must already have a listener set (use `setListener`).
    func injectCore(_ mock: MobileCoreProtocol) {
        core = mock
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
            if !path.isEmpty {
                synchronizeNavigation()
                if let daemon = selectedDaemonId, let session = selectedSessionId {
                    core.attach(id: daemon, sessionId: session)
                }
            }
            restoreWorkingLocation()
        } catch {
            print("Failed to initialize mobile core: \(error)")
        }
    }

    // MARK: - Daemon management

    var phoneNodeId: String {
        core?.nodeId() ?? ""
    }

    @discardableResult
    func addDaemon(info: DialInfo, name: String, replacingDaemonNodeId: String? = nil) -> String? {
        if let replacingDaemonNodeId, info.nodeId != replacingDaemonNodeId {
            return "This invite is for a different machine. Use a new invite from the machine you're pairing again."
        }
        guard let core else { return "Connection storage is unavailable. Try again shortly." }
        let wasSaved = daemons.contains { $0.nodeId == info.nodeId }
        let id: DaemonId
        if let token = info.pairingToken {
            id = core.addDaemonWithToken(nodeId: info.nodeId, name: name, token: token)
        } else {
            id = core.addDaemon(nodeId: info.nodeId, name: name)
        }
        daemons = core.listDaemons()
        removedDaemons.remove(info.nodeId)
        if wasSaved { core.disconnect(id: id) }
        connect(daemonId: id)
        if wasSaved, selectedDaemonId?.nodeId == id.nodeId, let session = selectedSessionId {
            retrySessionAttachment(daemonNodeId: id.nodeId, sessionId: session)
        }
        return nil
    }

    func removeDaemon(_ daemon: DaemonEntry) {
        guard let core else { return }
        if pendingNewSessionDaemon == daemon.nodeId { pendingNewSessionDaemon = nil }
        let id = DaemonId(nodeId: daemon.nodeId)
        core.removeDaemon(id: id)
        removedDaemons.insert(daemon.nodeId)
        phoneSettings.mutedMachines.remove(daemon.nodeId)
        daemons = core.listDaemons()
        forgetWorkingLocation(daemonNodeId: daemon.nodeId)
        daemonStatuses.removeValue(forKey: daemon.nodeId)
        sessionLists.removeValue(forKey: daemon.nodeId)
        modelsByDaemon.removeValue(forKey: daemon.nodeId)
        daemonSettings.removeValue(forKey: daemon.nodeId)
        clearManagementSettings(daemonNodeId: daemon.nodeId)
        settingsRequests.removeValue(forKey: daemon.nodeId)?.cancel()
        pendingPermissions.removeAll { $0.daemon == daemon.nodeId }
        pendingAskUser.removeAll { $0.daemon == daemon.nodeId }
        pendingGoals.removeAll { $0.daemon == daemon.nodeId }
        alerts.removeAll { $0.daemon == daemon.nodeId }
        if activeBanner?.daemon == daemon.nodeId { activeBanner = nil }
    }

    func renameDaemon(_ daemon: DaemonEntry, name: String) -> String? {
        guard let core else { return "Connection storage is unavailable. Try again shortly." }
        do {
            try core.renameDaemon(id: DaemonId(nodeId: daemon.nodeId), name: name)
            daemons = core.listDaemons()
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    func retryConnection(daemonNodeId: String) {
        let id = DaemonId(nodeId: daemonNodeId)
        if core?.retryConnection(id: id) == true,
           selectedDaemonId?.nodeId == daemonNodeId, let session = selectedSessionId {
            core?.attach(id: id, sessionId: session)
        }
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

    func sessionTimeline(daemonNodeId: String, includeArchived: Bool, now: Date = Date(),
                         calendar: Calendar = .current) -> [SessionTimelineSection] {
        var items: [SessionTimelineItem] = []
        for session in sessionLists[daemonNodeId] ?? [] where includeArchived || !session.archived {
            let activityAt = max(session.createdAt, session.lastMessageAt ?? 0,
                sessionAlertTimes[daemonNodeId]?[session.sessionId] ?? 0)
            items.append(SessionTimelineItem(daemonNodeId: daemonNodeId, session: session, activityAt: activityAt))
        }
        return SessionTimelineSection.grouped(items, now: now, calendar: calendar)
    }

    private func clearChat() {
        messages = []
        streamingText = ""
        streamingPartId = nil
        isStreaming = false
        isLoadingSession = false
        sessionLoadError = nil
        subagents = []
        jobs = []
        workerHistory = nil
        jobOutput = nil
        review = nil
        historyBefore = nil
        isLoadingOlderHistory = false
        historyError = nil
    }

    func selectDaemon(nodeId: String) {
        if selectedDaemonId?.nodeId != nodeId || selectedSessionId != nil {
            clearChat()
        }
        selectedDaemonId = DaemonId(nodeId: nodeId)
        selectedSessionId = nil
        core?.listSessions(id: DaemonId(nodeId: nodeId))
    }

    func clearSelection() {
        clearChat()
        selectedSessionId = nil
        selectedDaemonId = nil
    }

    func refreshSessions(daemonNodeId: String) {
        core?.listSessions(id: DaemonId(nodeId: daemonNodeId))
    }

    func openAlertSession(_ alert: AlertItem) {
        guard daemons.contains(where: { $0.nodeId == alert.daemon }), !alert.sessionId.isEmpty,
              alert.daemon != selectedDaemonId?.nodeId || alert.sessionId != selectedSessionId else { return }
        path.append(.chat(daemonNodeId: alert.daemon, sessionId: alert.sessionId))
    }

    func selectSession(_ sessionId: String, daemonNodeId: String? = nil) {
        guard let nodeId = daemonNodeId ?? selectedDaemonId?.nodeId else { return }
        let daemonId = DaemonId(nodeId: nodeId)
        let changed = selectedDaemonId?.nodeId != nodeId || selectedSessionId != sessionId
        if changed {
            clearChat()
            isLoadingSession = true
        }
        selectedDaemonId = daemonId
        selectedSessionId = sessionId
        if changed {
            core?.attach(id: daemonId, sessionId: sessionId)
        }

        // Pull snapshot for initial state
        if changed || sessionLoadError == nil, let snap = core?.snapshot(id: daemonId) {
            applySnapshot(snap)
        }
    }

    func retrySessionAttachment(daemonNodeId: String, sessionId: String) {
        guard selectedDaemonId?.nodeId == daemonNodeId, selectedSessionId == sessionId,
              let core else { return }
        sessionLoadError = nil
        isLoadingSession = true
        core.attach(id: DaemonId(nodeId: daemonNodeId), sessionId: sessionId)
    }

    func newSession(cwd: String? = nil) {
        guard let daemonId = selectedDaemonId, let core else { return }
        let nodeId = daemonId.nodeId
        pendingNewSessionDaemon = nodeId
        core.newSession(id: daemonId, cwd: cwd)
    }

    private func failSessionCreation(daemon: String, message: String) {
        guard pendingNewSessionDaemon == daemon else { return }
        pendingNewSessionDaemon = nil
        let item = AlertItem(daemon: daemon, sessionId: "", kind: "error",
            title: "Couldn't create conversation", detail: message)
        alerts.append(item)
        if isAppActive { activeBanner = item }
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

    func refreshReview(daemon: String, sessionId: String) {
        guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
        let requestId = UUID().uuidString
        var files: [ReviewFile] = []
        if let review, review.daemon == daemon, review.sessionId == sessionId {
            files = review.files
        }
        review = WorkspaceReview(daemon: daemon, sessionId: sessionId, requestId: requestId, files: files)
        if core?.readSessionDiff(id: DaemonId(nodeId: daemon), sessionId: sessionId, requestId: requestId) != true {
            review?.isLoading = false
            review?.error = "Connect to this session before reviewing changes."
        }
    }

    @discardableResult
    func loadOlderHistory() -> Bool {
        guard let daemonId = selectedDaemonId, let sessionId = selectedSessionId,
              historyBefore != nil, !isLoadingOlderHistory,
              case .connected? = daemonStatuses[daemonId.nodeId] else { return false }
        isLoadingOlderHistory = true
        historyError = nil
        if core?.loadOlderHistory(id: daemonId, sessionId: sessionId) != true {
            isLoadingOlderHistory = false
            historyError = "Couldn't request older messages. Reconnect and try again."
            return false
        }
        return true
    }

    func openWorkerHistory(daemon: String, sessionId: String, childSessionId: String) {
        guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
        let requestId = UUID().uuidString
        var messages: [ChatMessage] = []
        if let workerHistory, workerHistory.daemon == daemon, workerHistory.sessionId == sessionId,
           workerHistory.childSessionId == childSessionId {
            messages = workerHistory.messages
        }
        workerHistory = WorkerHistory(daemon: daemon, sessionId: sessionId, childSessionId: childSessionId,
            requestId: requestId, messages: messages)
        if core?.readSubagentHistory(id: DaemonId(nodeId: daemon), sessionId: sessionId, childSessionId: childSessionId, requestId: requestId) != true {
            workerHistory?.isLoading = false
            workerHistory?.error = "Connect to the parent conversation to read this worker's history."
        }
    }

    func openJobOutput(daemon: String, sessionId: String, jobId: String) {
        guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
        jobOutput = JobOutputDetail(daemon: daemon, sessionId: sessionId, jobId: jobId)
        requestJobOutput()
    }

    func loadMoreJobOutput() {
        guard jobOutput?.isLoading == false, jobOutput?.done == false else { return }
        requestJobOutput()
    }

    private func requestJobOutput() {
        guard let detail = jobOutput, selectedDaemonId?.nodeId == detail.daemon,
              selectedSessionId == detail.sessionId else { return }
        let requestId = UUID().uuidString
        jobOutput?.requestId = requestId
        jobOutput?.isLoading = true
        jobOutput?.error = nil
        if core?.readJobOutput(id: DaemonId(nodeId: detail.daemon), sessionId: detail.sessionId,
            jobId: detail.jobId, requestId: requestId, offset: detail.nextOffset) != true {
            jobOutput?.isLoading = false
            jobOutput?.error = "Connect to this conversation to read job output."
        }
    }

    private func directoryKey(sessionId: String, path: String?) -> String {
        let p = path ?? ""
        return "\(sessionId)::\(p)"
    }

    var canSendPrompt: Bool {
        guard sessionLoadError == nil, !isLoadingSession, !isStreaming, let daemonId = selectedDaemonId,
              let sessionId = selectedSessionId,
              case .connected? = daemonStatuses[daemonId.nodeId],
              let snapshot = core?.snapshot(id: daemonId) else { return false }
        return snapshot.attachedSession == sessionId && pendingSubmissions[daemonId.nodeId]?.contains(sessionId) != true
    }

    @discardableResult
    func sendPrompt(_ text: String) -> Bool {
        guard canSendPrompt, let daemonId = selectedDaemonId, let core,
              core.prompt(id: daemonId, text: text) else { return false }
        if let sessionId = selectedSessionId {
            pendingSubmissions[daemonId.nodeId, default: []].insert(sessionId)
            lastPrompt[sessionId] = text
        }
        emitFeedback(.sent)
        return true
    }

    /// Re-send the last prompt for the selected session (retry after a failed
    /// turn). No-op if there's no stored prompt to retry.
    func retryLastTurn() {
        guard let sessionId = selectedSessionId,
              let prompt = lastPrompt[sessionId],
              !prompt.isEmpty else { return }
        if sendPrompt(prompt) {
            lastTurnFailed[sessionId] = false
        }
    }

    func cancelTurn() {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.cancel(id: daemonId)
    }

    func respondPermission(requestId: String, decision: Decision) {
        guard let daemonId = selectedDaemonId, let core,
              let request = pendingPermissions.first(where: {
                  $0.daemon == daemonId.nodeId && $0.sessionId == selectedSessionId && $0.requestId == requestId
              }) else { return }
        core.respondPermission(id: DaemonId(nodeId: request.daemon), requestId: request.requestId, decision: decision)
    }

    func respondToGoal(requestId: String, accepted: Bool) {
        guard let daemonId = selectedDaemonId, let core,
              let goal = pendingGoals.first(where: {
                  $0.requestId == requestId && $0.daemon == daemonId.nodeId
                      && $0.sessionId == selectedSessionId
              }) else { return }
        core.respondToGoal(id: DaemonId(nodeId: goal.daemon), requestId: goal.requestId, accepted: accepted)
    }

    func respondAskUser(requestId: String, answers: [String]) {
        guard let daemonId = selectedDaemonId, let core,
              let request = pendingAskUser.first(where: {
                  $0.daemon == daemonId.nodeId && $0.sessionId == selectedSessionId && $0.requestId == requestId
              }) else { return }
        core.respondAskUser(id: DaemonId(nodeId: request.daemon), requestId: request.requestId, answers: answers)
    }

    func cancelAskUser(requestId: String) {
        guard let daemonId = selectedDaemonId, let core,
              let request = pendingAskUser.first(where: {
                  $0.daemon == daemonId.nodeId && $0.sessionId == selectedSessionId && $0.requestId == requestId
              }), request.subagent != nil else { return }
        core.cancelAskUser(id: DaemonId(nodeId: request.daemon), requestId: request.requestId)
    }

    func listModels() {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.listModels(id: daemonId)
    }

    func selectedModel(daemonNodeId: String) -> ModelSummary? {
        guard !isLoadingSession, let model = currentModel[daemonNodeId],
              let provider = currentProvider[daemonNodeId] else { return nil }
        return modelsByDaemon[daemonNodeId]?.first { $0.provider == provider && $0.model == model }
    }

    func switchModel(provider: String, model: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.switchModel(id: daemonId, provider: provider, model: model)
    }

    func setPermissionMode(_ mode: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.setPermissionMode(id: daemonId, mode: mode)
    }

    @discardableResult
    func sendSlashCommand(_ command: String) -> Bool {
        guard canSendPrompt, let daemonId = selectedDaemonId, let core else { return false }
        return core.slashCommand(id: daemonId, command: command)
    }

    func setThinkingVariant(_ variant: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.setThinkingVariant(id: daemonId, variant: variant)
    }

    func listPersonas() {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.listPersonas(id: daemonId)
    }

    func switchPersona(name: String) {
        guard let daemonId = selectedDaemonId, let core else { return }
        core.switchPersona(id: daemonId, name: name)
    }

    func readDaemonSettings(daemonNodeId: String) {
        beginSettingsRequest(daemonNodeId: daemonNodeId,
            accepted: core?.readAutomationSettings(id: DaemonId(nodeId: daemonNodeId)) == true)
    }

    func setAutoTitle(daemonNodeId: String, enabled: Bool) {
        beginSettingsRequest(daemonNodeId: daemonNodeId,
            accepted: core?.setAutoTitle(id: DaemonId(nodeId: daemonNodeId), enabled: enabled) == true)
    }

    func setAutoSummary(daemonNodeId: String, enabled: Bool) {
        beginSettingsRequest(daemonNodeId: daemonNodeId,
            accepted: core?.setAutoSummary(id: DaemonId(nodeId: daemonNodeId), enabled: enabled) == true)
    }

    private func beginSettingsRequest(daemonNodeId: String, accepted: Bool) {
        settingsRequests.removeValue(forKey: daemonNodeId)?.cancel()
        daemonSettingsErrors.removeValue(forKey: daemonNodeId)
        guard accepted else {
            daemonSettingsErrors[daemonNodeId] = "Couldn't send the request. Reconnect and try again."
            return
        }
        settingsRequests[daemonNodeId] = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(5)) } catch { return }
            self?.daemonSettingsErrors[daemonNodeId] = "The daemon didn't confirm its settings. Update the daemon and try again."
            self?.settingsRequests.removeValue(forKey: daemonNodeId)
        }
    }

    func readDaemonManagementSettings(daemonNodeId: String) {
        beginManagementRequest(daemonNodeId: daemonNodeId) { id, request in
            core?.readDaemonSettings(id: id, requestId: request) == true
        }
    }

    func saveSessionDefaults(daemonNodeId: String, sessionId: String) {
        guard selectedDaemonId?.nodeId == daemonNodeId, selectedSessionId == sessionId, canSendPrompt else {
            managementErrors[daemonNodeId] = "Open this conversation and wait for it to finish loading before saving its defaults."
            return
        }
        beginManagementRequest(daemonNodeId: daemonNodeId) { id, request in
            core?.saveSessionDefaults(id: id, sessionId: sessionId, requestId: request) == true
        }
    }

    func resetSessionDefaults(daemonNodeId: String) {
        beginManagementRequest(daemonNodeId: daemonNodeId) { id, request in
            core?.resetSessionDefaults(id: id, requestId: request) == true
        }
    }

    func revokeDevice(daemonNodeId: String, deviceId: String) {
        guard managementSettings[daemonNodeId]?.devices?.contains(where: { $0.id == deviceId }) == true else {
            managementErrors[daemonNodeId] = "Refresh settings before revoking this device."
            return
        }
        beginManagementRequest(daemonNodeId: daemonNodeId) { id, request in
            core?.revokeDevice(id: id, deviceId: deviceId, requestId: request) == true
        }
    }

    private func beginManagementRequest(daemonNodeId: String, send: (DaemonId, String) -> Bool) {
        guard managementRequestIds[daemonNodeId] == nil else { return }
        guard !removedDaemons.contains(daemonNodeId), case .connected? = daemonStatuses[daemonNodeId] else {
            managementErrors[daemonNodeId] = "Connect to this machine to manage its settings."
            return
        }
        managementErrors.removeValue(forKey: daemonNodeId)
        let request = UUID().uuidString
        guard send(DaemonId(nodeId: daemonNodeId), request) else {
            managementErrors[daemonNodeId] = "Couldn't send the request. Reconnect and try again."
            return
        }
        managementRequestIds[daemonNodeId] = request
        managementTimeouts[daemonNodeId] = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(5)) } catch { return }
            guard let self, managementRequestIds[daemonNodeId] == request else { return }
            managementRequestIds.removeValue(forKey: daemonNodeId)
            managementTimeouts.removeValue(forKey: daemonNodeId)
            managementErrors[daemonNodeId] = "The daemon didn't confirm the request. Update the daemon if needed, then refresh settings before trying again."
        }
    }

    private func clearManagementSettings(daemonNodeId: String) {
        managementSettings.removeValue(forKey: daemonNodeId)
        managementErrors.removeValue(forKey: daemonNodeId)
        managementRequestIds.removeValue(forKey: daemonNodeId)
        managementTimeouts.removeValue(forKey: daemonNodeId)?.cancel()
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

    func handleEvent(_ event: CoreEvent, receivedAt: Date = Date()) {
        switch event {
        case .daemonStatusChanged(let daemon, let status):
            daemonStatuses[daemon] = status
            if case .connected = status {
                // Auto-list sessions for any daemon that connects
                if let core {
                    let id = DaemonId(nodeId: daemon)
                    core.listSessions(id: id)
                }
            } else {
                failSessionCreation(daemon: daemon,
                    message: "Connection lost before the conversation was created. Reconnect and try again.")
                settingsRequests.removeValue(forKey: daemon)?.cancel()
                daemonSettings.removeValue(forKey: daemon)
                daemonSettingsErrors.removeValue(forKey: daemon)
                clearManagementSettings(daemonNodeId: daemon)
                pendingSubmissions.removeValue(forKey: daemon)
                if workerHistory?.daemon == daemon, workerHistory?.isLoading == true {
                    workerHistory?.isLoading = false
                    workerHistory?.error = "Connection lost. Reconnect and refresh this worker's history."
                }
                if jobOutput?.daemon == daemon, jobOutput?.isLoading == true {
                    jobOutput?.isLoading = false
                    jobOutput?.error = "Connection lost. Reconnect to read more output."
                }
                if selectedDaemonId?.nodeId == daemon, isLoadingOlderHistory {
                    isLoadingOlderHistory = false
                    historyError = "Connection lost. Reconnect to load older messages."
                }
                if review?.daemon == daemon, review?.isLoading == true {
                    review?.isLoading = false
                    review?.error = "Connection lost. Reconnect and refresh to review changes."
                }
            }

        case .activityUpdated(let daemon, let sessionId, let workers, let backgroundJobs):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            subagents = workers
            jobs = backgroundJobs

        case .workerHistoryLoaded(let daemon, let sessionId, let childSessionId, let requestId, let messages, let error):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId,
                  workerHistory?.daemon == daemon, workerHistory?.sessionId == sessionId,
                  workerHistory?.childSessionId == childSessionId, workerHistory?.requestId == requestId,
                  workerHistory?.isLoading == true else { return }
            if error == nil { workerHistory?.messages = messages }
            workerHistory?.error = error
            workerHistory?.isLoading = false

        case .jobOutputLoaded(let daemon, let sessionId, let jobId, let requestId, let offset, let nextOffset, let output, let done, let incomplete):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId,
                  jobOutput?.daemon == daemon, jobOutput?.sessionId == sessionId,
                  jobOutput?.jobId == jobId, jobOutput?.requestId == requestId,
                  jobOutput?.isLoading == true else { return }
            guard offset == jobOutput?.nextOffset, nextOffset >= offset,
                  nextOffset - offset == UInt64(output.utf8.count) else {
                jobOutput?.error = "The daemon returned an invalid output chunk. Refresh to try again."
                jobOutput?.isLoading = false
                return
            }
            jobOutput?.output.append(output)
            jobOutput?.nextOffset = nextOffset
            jobOutput?.done = done
            jobOutput?.incomplete = jobOutput?.incomplete == true || incomplete
            jobOutput?.isLoading = false

        case .jobOutputFailed(let daemon, let sessionId, let jobId, let requestId, let message):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId,
                  jobOutput?.daemon == daemon, jobOutput?.sessionId == sessionId,
                  jobOutput?.jobId == jobId, jobOutput?.requestId == requestId,
                  jobOutput?.isLoading == true else { return }
            jobOutput?.error = message
            jobOutput?.isLoading = false

        case .reviewLoaded(let daemon, let sessionId, let requestId, let files):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId,
                  review?.daemon == daemon, review?.sessionId == sessionId,
                  review?.requestId == requestId, review?.isLoading == true else { return }
            review?.files = files
            review?.isLoading = false

        case .reviewFailed(let daemon, let sessionId, let requestId, let message):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId,
                  review?.daemon == daemon, review?.sessionId == sessionId,
                  review?.requestId == requestId, review?.isLoading == true else { return }
            review?.isLoading = false
            review?.error = message

        case .daemonVersion(let daemon, let version):
            daemonVersions[daemon] = version

        case .sessionList(let daemon, let sessions):
            sessionLists[daemon] = sessions
            validateRestoredChat(daemonNodeId: daemon, sessions: sessions)
            // Seed per-session turn-failure state from the rail summaries so
            // the ChatView retry banner is correct on fresh attach (before any
            // providerMessageEnded event fires for this session).
            for session in sessions {
                lastTurnFailed[session.sessionId] = session.lastTurnFailed
            }

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

        case .sessionReady(let daemon, let sessionId):
            pendingSubmissions[daemon]?.remove(sessionId)
            pendingPermissions.removeAll { $0.daemon == daemon }
            pendingAskUser.removeAll { $0.daemon == daemon }
            pendingGoals.removeAll { $0.daemon == daemon }
            if selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId {
                historyBefore = nil
                isLoadingOlderHistory = false
                historyError = nil
            }

        case .historyExtended(let daemon, let sessionId):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            refreshMessages()
            isLoadingOlderHistory = false
            historyError = nil

        case .sessionCreated(let daemon, let sessionId):
            guard pendingNewSessionDaemon == daemon else { return }
            pendingNewSessionDaemon = nil
            guard selectedDaemonId?.nodeId == daemon else { return }
            path.append(.chat(daemonNodeId: daemon, sessionId: sessionId))

        case .sessionCreationFailed(let daemon, let message):
            failSessionCreation(daemon: daemon, message: message)

        case .sessionAttachmentFailed(let daemon, let sessionId, let message):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            isLoadingSession = false
            sessionLoadError = message

        case .sessionReloaded(let daemon, let sessionId):
            guard selectedDaemonId?.nodeId == daemon else { return }
            guard selectedSessionId == sessionId else { return }
            // Pull snapshot after reload
            if let snap = core?.snapshot(id: DaemonId(nodeId: daemon)) {
                applySnapshot(snap)
            }
            // Re-fetch persona list — it may have changed during disconnection.
            listPersonas()
        case .textDelta(let daemon, let sessionId, let partId, let delta):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            if streamingPartId == partId {
                streamingText += delta
            } else {
                streamingPartId = partId
                streamingText = delta
            }

        case .partUpdated(let daemon, let sessionId, let partId, _, let state):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            if state == "completed", streamingPartId == partId {
                streamingPartId = nil
                streamingText = ""
            }
            // Update the message part in place
            updatePartInMessages(partId: partId)

        case .sessionActivityChanged(let daemon, let sessionId, let running):
            if running { pendingSubmissions[daemon]?.remove(sessionId) }
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            isStreaming = running
            if !running {
                streamingPartId = nil
                streamingText = ""
            }

        case .providerMessageEnded(let daemon, let sessionId, let inputTokens, let outputTokens, let cost, let failed, _):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            streamingPartId = nil
            streamingText = ""
            if let sessionId = selectedSessionId {
                lastTurnFailed[sessionId] = failed
            }
            // Capture usage from the event, merging turns from snapshot if available.
            let turns: UInt32
            if let snap = core?.snapshot(id: DaemonId(nodeId: daemon)),
               let session = snap.sessions.first {
                turns = session.turns
            } else {
                turns = (sessionUsage[sessionId]?.turns ?? 0) + 1
            }
            sessionUsage[sessionId] = SessionUsage(
                inputTokens: inputTokens,
                outputTokens: outputTokens,
                cost: cost,
                turns: turns
            )
            // Refresh messages from snapshot
            refreshMessages()

        case .permissionRequested(let daemon, let sessionId, let requestId, let toolName, let input):
            // Dedup: the daemon replays outstanding requests on re-attach.
            if !pendingPermissions.contains(where: { $0.daemon == daemon && $0.sessionId == sessionId && $0.requestId == requestId }) {
                pendingPermissions.append(PendingPermission(
                    daemon: daemon,
                    requestId: requestId,
                    sessionId: sessionId,
                    toolName: toolName,
                    input: input
                ))
            }

        case .askUserRequested(let subagent, let daemon, let sessionId, let requestId, let callId, let questions):
            if !pendingAskUser.contains(where: { $0.daemon == daemon && $0.sessionId == sessionId && $0.requestId == requestId }) {
                pendingAskUser.append(PendingAskUser(
                    subagent: subagent,
                    daemon: daemon,
                    requestId: requestId,
                    sessionId: sessionId,
                    callId: callId,
                    questions: questions
                ))
            }

        case .goalProposed(let daemon, let sessionId, let requestId, _, let objective):
            if !pendingGoals.contains(where: { $0.daemon == daemon && $0.requestId == requestId }) {
                pendingGoals.append(PendingGoalProposal(daemon: daemon, sessionId: sessionId,
                    requestId: requestId, objective: objective))
            }

        case .subagentEnd:
            break

        case .requestResolved(let daemon, let requestId):
            pendingGoals.removeAll { $0.daemon == daemon && $0.requestId == requestId }
            pendingPermissions.removeAll { $0.daemon == daemon && $0.requestId == requestId }
            pendingAskUser.removeAll { $0.daemon == daemon && $0.requestId == requestId }

        case .alert(let daemon, let sessionId, let kind, let title, let detail):
            guard !removedDaemons.contains(daemon) else { return }
            if !sessionId.isEmpty {
                let timestamp = Int64(receivedAt.timeIntervalSince1970 * 1_000)
                sessionAlertTimes[daemon, default: [:]][sessionId] = max(
                    sessionAlertTimes[daemon]?[sessionId] ?? 0, timestamp)
                if sessionLists[daemon]?.contains(where: { $0.sessionId == sessionId }) != true {
                    refreshSessions(daemonNodeId: daemon)
                }
            }
            switch kind {
            case "error", "turncomplete", "turnfailed":
                pendingSubmissions[daemon]?.remove(sessionId)
            default:
                break
            }
            let item = AlertItem(
                daemon: daemon,
                sessionId: sessionId,
                kind: kind,
                title: title,
                detail: detail
            )
            alerts.append(item)
            if isAppActive, phoneSettings.allowsAlert(kind: kind, daemon: daemon) {
                if kind == "error" || kind == "turnfailed" || daemon != selectedDaemonId?.nodeId || sessionId != selectedSessionId {
                    activeBanner = item
                }
                switch kind {
                case "turncomplete": emitFeedback(.completed)
                case "error", "turnfailed": emitFeedback(.failed)
                default: emitFeedback(.attention)
                }
            }
            if kind == "error", daemon == selectedDaemonId?.nodeId, sessionId == selectedSessionId {
                isLoadingSession = false
                lastTurnFailed[sessionId] = true
                if isLoadingOlderHistory {
                    isLoadingOlderHistory = false
                    historyError = detail ?? title
                }
            }

        case .attentionChanged(let daemon, let sessionId, let permissions, let questions):
            if let index = sessionLists[daemon]?.firstIndex(where: { $0.sessionId == sessionId }) {
                sessionLists[daemon]?[index].pendingPermissions = permissions
                sessionLists[daemon]?[index].pendingQuestions = questions
            }

        case .todosUpdated(_, let sessionId, let todoItems):
            todos[sessionId] = todoItems

        case .modelList(let daemon, let models):
            modelsByDaemon[daemon] = models

        case .automationSettings(let daemon, let settings):
            daemonSettings[daemon] = settings
            daemonSettingsErrors.removeValue(forKey: daemon)
            settingsRequests.removeValue(forKey: daemon)?.cancel()

        case .daemonManagementSettings(let daemon, let requestId, let settings):
            guard !removedDaemons.contains(daemon), case .connected? = daemonStatuses[daemon] else { return }
            if let requestId {
                guard managementRequestIds[daemon] == requestId else { return }
                managementRequestIds.removeValue(forKey: daemon)
                managementTimeouts.removeValue(forKey: daemon)?.cancel()
                managementErrors.removeValue(forKey: daemon)
            } else if managementSettings[daemon] == nil {
                return
            }
            managementSettings[daemon] = settings

        case .daemonManagementFailed(let daemon, let requestId, let message):
            guard managementRequestIds[daemon] == requestId else { return }
            managementRequestIds.removeValue(forKey: daemon)
            managementTimeouts.removeValue(forKey: daemon)?.cancel()
            managementErrors[daemon] = message

        case .autoSummaryChanged(let daemon, let enabled):
            daemonSettings[daemon]?.autoSummaryEnabled = enabled

        case .slashResult(let daemon, let sessionId, let text):
            guard selectedDaemonId?.nodeId == daemon, selectedSessionId == sessionId else { return }
            // Append the slash result as a synthetic assistant message so it
            // appears inline in the chat transcript.
            messages.append(ChatMessage(
                id: UUID().uuidString,
                role: "assistant",
                parts: [MessagePart(
                    id: UUID().uuidString,
                    kind: .text,
                    text: text,
                    toolName: nil,
                    toolState: nil,
                    toolInput: nil,
                    toolOutput: nil,
                    toolError: nil,
                    toolCallId: nil,
                    toolTimeStart: nil,
                    toolTimeEnd: nil,
                    toolSensitivity: nil
                )],
                assistantMeta: nil
            ))

        case .permissionModeChanged(let daemon, let mode):
            permissionMode[daemon] = mode

        case .modelSwitched(let daemon, let provider, let model):
            currentProvider[daemon] = provider
            currentModel[daemon] = model

        case .thinkingVariantChanged(let daemon, let variant):
            thinkingVariant[daemon] = variant

        case .personaList(let daemon, let personas):
            availablePersonas[daemon] = personas
            currentPersona[daemon] = personas.first(where: { $0.active })?.name

        case .personaSwitched(let daemon, let name):
            currentPersona[daemon] = name
        }
    }

    // MARK: - Helpers

    private func emitFeedback(_ kind: PhoneFeedback.Kind) {
        guard isAppActive, phoneSettings.haptics else { return }
        feedback = PhoneFeedback(kind: kind)
    }

    /// Export an allowlist of connection metadata, excluding free-form names and errors.
    var connectionDiagnostics: String {
        func version(_ value: String?) -> String {
            guard let value else { return "unknown" }
            let core = value.split(separator: "-", maxSplits: 1).first.map(String.init) ?? ""
            let parts = core.split(separator: ".", omittingEmptySubsequences: false)
            guard (2...4).contains(parts.count), parts.allSatisfy({ !$0.isEmpty && $0.count <= 5 && $0.allSatisfy(\.isNumber) }) else {
                return "unknown"
            }
            return core
        }
        let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String
        var lines = ["mew connection diagnostics",
            "App: \(version(appVersion))",
            "iOS: \(version(UIDevice.current.systemVersion))",
            "Saved machines: \(daemons.count)"]
        for (index, daemon) in daemons.enumerated() {
            let status: String
            switch daemonStatuses[daemon.nodeId] ?? .disconnected {
            case .connected: status = "connected"
            case .connecting: status = "connecting"
            case .disconnected: status = "disconnected"
            case .backoff: status = "retrying"
            case .pairedLost: status = "pairing required"
            }
            lines.append("Machine \(index + 1): \(status); daemon \(version(daemonVersions[daemon.nodeId] ?? daemon.lastKnownVersion))")
        }
        return lines.joined(separator: "\n")
    }

    private func applySnapshot(_ snap: DaemonSnapshot) {
        if let session = snap.sessions.first(where: { $0.sessionId == selectedSessionId }) {
            isLoadingSession = false
            sessionLoadError = nil
            messages = session.messages
            historyBefore = session.historyBefore
            subagents = session.subagents
            jobs = session.jobs
            isStreaming = session.running
            if let daemon = selectedDaemonId?.nodeId {
                pendingPermissions.removeAll { $0.daemon == daemon && $0.sessionId == session.sessionId }
                pendingPermissions.append(contentsOf: snap.pendingPermissions.filter {
                    $0.daemon == daemon && $0.sessionId == session.sessionId
                })
                pendingAskUser.removeAll { $0.daemon == daemon && $0.sessionId == session.sessionId }
                pendingAskUser.append(contentsOf: snap.pendingAskUser.filter {
                    $0.daemon == daemon && $0.sessionId == session.sessionId
                })
            }
            // Seed usage + todos from snapshot
            if let sessionId = session.sessionId.isEmpty ? nil : Optional(session.sessionId) {
                sessionUsage[sessionId] = SessionUsage(
                    inputTokens: session.inputTokens,
                    outputTokens: session.outputTokens,
                    cost: session.usageCost,
                    turns: session.turns
                )
                todos[sessionId] = session.todos
            }
        }
        // Seed daemon-level state from snapshot
        if let daemonId = selectedDaemonId?.nodeId {
            if let mode = snap.permissionMode {
                permissionMode[daemonId] = mode
            }
            if let model = snap.currentModel {
                currentModel[daemonId] = model
            }
            if let provider = snap.currentProvider {
                currentProvider[daemonId] = provider
            }
            thinkingVariant[daemonId] = snap.thinkingVariant
            // Unconditionally assign — handles both Some and None/empty,
            // preventing stale state after daemon-side persona clear.
            currentPersona[daemonId] = snap.currentPersona
            availablePersonas[daemonId] = snap.availablePersonas
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
    let daemon: String
    let sessionId: String
    let kind: String
    let title: String
    let detail: String?
}

// MARK: - Session usage

/// Usage stats for a session, assembled from provider messages and SessionInfo.
struct SessionUsage: Equatable {
    var inputTokens: UInt64
    var outputTokens: UInt64
    var cost: Double
    var turns: UInt32
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
