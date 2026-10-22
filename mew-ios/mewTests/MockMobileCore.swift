import Foundation
import MewMobileCore

/// A mock implementation of `MobileCoreProtocol` for unit testing.
/// Records all method calls and returns configurable snapshot data.
/// Events are delivered via `fireEvent` — the test controls timing.
// UniFFI exposes synchronous requirements; tests call this spy from the main actor.
@MainActor
final class MockMobileCore: @preconcurrency MobileCoreProtocol {
    // Recorded calls
    var goalResponseCalls: [(nodeId: String, requestId: String, accepted: Bool)] = []
    var permissionResponseCalls: [(nodeId: String, requestId: String, decision: Decision)] = []
    var askUserResponseCalls: [(nodeId: String, requestId: String, answers: [String])] = []
    var promptAccepted = true
    var promptCalls: [(nodeId: String, text: String)] = []
    var slashCommandCalls: [(nodeId: String, command: String)] = []
    var setPermissionModeCalls: [(nodeId: String, mode: String)] = []
    var setThinkingVariantCalls: [(nodeId: String, variant: String)] = []
    var switchPersonaCalls: [(nodeId: String, name: String)] = []
    var switchModelCalls: [(nodeId: String, provider: String, model: String)] = []
    var connectCalls: [String] = []
    var addedDaemons: [(nodeId: String, name: String, token: String?)] = []
    var disconnectCalls: [String] = []
    var settingsAccepted = true
    var settingsReadCalls: [String] = []
    var autoTitleCalls: [(nodeId: String, enabled: Bool)] = []
    var autoSummaryCalls: [(nodeId: String, enabled: Bool)] = []
    var askUserCancelCalls: [(nodeId: String, requestId: String)] = []
    var cancelCalls: [String] = []
    var attachCalls: [(nodeId: String, sessionId: String)] = []
    var listSessionsCalls: [String] = []
    var listModelsCalls: [String] = []
    var newSessionCalls: [String?] = []
    var reviewAccepted = true
    var historyAccepted = true
    var historyCalls: [(nodeId: String, sessionId: String)] = []
    var detailAccepted = true
    var reviewCalls: [(nodeId: String, sessionId: String, requestId: String)] = []

    // Configurable return values
    var snapshotResult: DaemonSnapshot?
    var snapshotsByDaemon: [String: DaemonSnapshot] = [:]
    var daemonsResult: [DaemonEntry] = []
    var nodeIdResult: String = "mock-node-id"

    // Listener (set by AppStore)
    private var listener: CoreListener?

    // MARK: - Test helpers

    /// Deliver an event to the registered listener.
    func fireEvent(_ event: CoreEvent) {
        listener?.onEvent(event: event)
    }

    // MARK: - MobileCoreProtocol conformance

    func addDaemon(nodeId: String, name: String) -> DaemonId {
        addedDaemons.append((nodeId, name, nil))
        return DaemonId(nodeId: nodeId)
    }

    func addDaemonWithToken(nodeId: String, name: String, token: String) -> DaemonId {
        addedDaemons.append((nodeId, name, token))
        return DaemonId(nodeId: nodeId)
    }

    func removeDaemon(id: DaemonId) { daemonsResult.removeAll { $0.nodeId == id.nodeId } }

    func renameDaemon(id: DaemonId, name: String) throws {
        if let index = daemonsResult.firstIndex(where: { $0.nodeId == id.nodeId }) {
            daemonsResult[index].name = name
        }
    }

    func retryConnection(id: DaemonId) -> Bool {
        connectCalls.append(id.nodeId)
        return daemonsResult.contains { $0.nodeId == id.nodeId }
    }

    func listDaemons() -> [DaemonEntry] { daemonsResult }

    func connect(id: DaemonId) { connectCalls.append(id.nodeId) }
    func disconnect(id: DaemonId) { disconnectCalls.append(id.nodeId) }

    func attach(id: DaemonId, sessionId: String) {
        attachCalls.append((id.nodeId, sessionId))
    }

    func snapshot(id: DaemonId) -> DaemonSnapshot? { snapshotsByDaemon[id.nodeId] ?? snapshotResult }

    func prompt(id: DaemonId, text: String) -> Bool {
        promptCalls.append((id.nodeId, text))
        return promptAccepted
    }

    func cancel(id: DaemonId) { cancelCalls.append(id.nodeId) }

    func respondPermission(id: DaemonId, requestId: String, decision: Decision) {
        permissionResponseCalls.append((id.nodeId, requestId, decision))
    }
    func cancelAskUser(id: DaemonId, requestId: String) {
        askUserCancelCalls.append((id.nodeId, requestId))
    }
    func respondAskUser(id: DaemonId, requestId: String, answers: [String]) {
        askUserResponseCalls.append((id.nodeId, requestId, answers))
    }
    func respondToGoal(id: DaemonId, requestId: String, accepted: Bool) {
        goalResponseCalls.append((id.nodeId, requestId, accepted))
    }

    func listSessions(id: DaemonId) { listSessionsCalls.append(id.nodeId) }
    func newSession(id: DaemonId, cwd: String?) { newSessionCalls.append(cwd) }
    func listProjects(id: DaemonId) {}
    func listDir(id: DaemonId, sessionId: String, path: String?) {}

    func readSessionDiff(id: DaemonId, sessionId: String, requestId: String) -> Bool {
        reviewCalls.append((id.nodeId, sessionId, requestId))
        return reviewAccepted
    }

    func loadOlderHistory(id: DaemonId, sessionId: String) -> Bool {
        historyCalls.append((id.nodeId, sessionId))
        return historyAccepted
    }

    func readSubagentHistory(id: DaemonId, sessionId: String, childSessionId: String, requestId: String) -> Bool {
        detailAccepted
    }

    func readJobOutput(id: DaemonId, sessionId: String, jobId: String, requestId: String, offset: UInt64) -> Bool {
        detailAccepted
    }

    func listModels(id: DaemonId) { listModelsCalls.append(id.nodeId) }
    func switchModel(id: DaemonId, provider: String, model: String) {
        switchModelCalls.append((id.nodeId, provider, model))
    }

    func listPersonas(id: DaemonId) {}
    func switchPersona(id: DaemonId, name: String) {
        switchPersonaCalls.append((id.nodeId, name))
    }

    func readAutomationSettings(id: DaemonId) -> Bool {
        settingsReadCalls.append(id.nodeId)
        return settingsAccepted
    }
    func setAutoTitle(id: DaemonId, enabled: Bool) -> Bool {
        autoTitleCalls.append((id.nodeId, enabled))
        return settingsAccepted
    }
    func setAutoSummary(id: DaemonId, enabled: Bool) -> Bool {
        autoSummaryCalls.append((id.nodeId, enabled))
        return settingsAccepted
    }

    func setPermissionMode(id: DaemonId, mode: String) {
        setPermissionModeCalls.append((id.nodeId, mode))
    }

    func slashCommand(id: DaemonId, command: String) -> Bool {
        slashCommandCalls.append((id.nodeId, command))
        return promptAccepted
    }

    func setThinkingVariant(id: DaemonId, variant: String) {
        setThinkingVariantCalls.append((id.nodeId, variant))
    }

    func archiveSession(id: DaemonId, sessionId: String, archived: Bool) {}
    func pinSession(id: DaemonId, sessionId: String, pinned: Bool) {}
    func deleteSession(id: DaemonId, sessionId: String) {}
    func renameSession(id: DaemonId, sessionId: String, title: String) {}

    func nodeId() -> String { nodeIdResult }

    func setListener(listener: CoreListener) {
        self.listener = listener
    }
}
