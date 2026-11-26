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
    var promptAttachmentCalls: [(nodeId: String, text: String, attachments: [PromptAttachment])] = []
    var readFilePreviewCalls: [(nodeId: String, sessionId: String, path: String, maxBytes: UInt64?)] = []
    var listMentionCandidatesCalls: [String] = []
    var enqueueCalls: [(nodeId: String, sessionId: String, text: String, steer: Bool)] = []
    var promoteQueueCalls: [(nodeId: String, sessionId: String, messageId: String)] = []
    var removeQueueCalls: [(nodeId: String, sessionId: String, messageId: String)] = []
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
    var managementReadCalls: [(nodeId: String, requestId: String)] = []
    var saveDefaultsCalls: [(nodeId: String, sessionId: String, requestId: String)] = []
    var resetDefaultsCalls: [(nodeId: String, requestId: String)] = []
    var revokeDeviceCalls: [(nodeId: String, deviceId: String, requestId: String)] = []
    var askUserCancelCalls: [(nodeId: String, requestId: String)] = []
    var cancelCalls: [String] = []
    var attachCalls: [(nodeId: String, sessionId: String)] = []
    var listSessionsCalls: [String] = []
    var listModelsCalls: [String] = []
    var newSessionCalls: [String?] = []
    var newSessionTargets: [String] = []
    var projectCalls: [String] = []
    var sessionMutationCalls: [(nodeId: String, sessionId: String, action: String)] = []
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

    func sendPromptWithAttachments(id: DaemonId, text: String, attachments: [PromptAttachment]) -> Bool {
        promptAttachmentCalls.append((id.nodeId, text, attachments))
        return promptAccepted
    }

    func cancel(id: DaemonId) { cancelCalls.append(id.nodeId) }

    func enqueueMessage(id: DaemonId, sessionId: String, text: String, steer: Bool) -> Bool {
        enqueueCalls.append((id.nodeId, sessionId, text, steer))
        return promptAccepted
    }

    func promoteQueuedMessage(id: DaemonId, sessionId: String, messageId: String) -> Bool {
        promoteQueueCalls.append((id.nodeId, sessionId, messageId))
        return promptAccepted
    }

    func removeQueuedMessage(id: DaemonId, sessionId: String, messageId: String) -> Bool {
        removeQueueCalls.append((id.nodeId, sessionId, messageId))
        return promptAccepted
    }

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
    func newSession(id: DaemonId, cwd: String?, scratch: Bool) {
        newSessionCalls.append(cwd)
        newSessionTargets.append(id.nodeId)
    }
    func readArtifact(id: DaemonId, requestId: String, sessionId: String, artifactId: String, path: String) {}
    func listProjects(id: DaemonId) { projectCalls.append(id.nodeId) }
    func listDir(id: DaemonId, sessionId: String, path: String?) {}
    func readFilePreview(id: DaemonId, sessionId: String, path: String, maxBytes: UInt64?) {
        readFilePreviewCalls.append((id.nodeId, sessionId, path, maxBytes))
    }
    func listMentionCandidates(id: DaemonId) {
        listMentionCandidatesCalls.append(id.nodeId)
    }

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

    func readDaemonSettings(id: DaemonId, requestId: String) -> Bool {
        managementReadCalls.append((id.nodeId, requestId))
        return settingsAccepted
    }

    func saveSessionDefaults(id: DaemonId, sessionId: String, requestId: String) -> Bool {
        saveDefaultsCalls.append((id.nodeId, sessionId, requestId))
        return settingsAccepted
    }

    func resetSessionDefaults(id: DaemonId, requestId: String) -> Bool {
        resetDefaultsCalls.append((id.nodeId, requestId))
        return settingsAccepted
    }

    func revokeDevice(id: DaemonId, deviceId: String, requestId: String) -> Bool {
        revokeDeviceCalls.append((id.nodeId, deviceId, requestId))
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

    func archiveSession(id: DaemonId, sessionId: String, archived: Bool) {
        sessionMutationCalls.append((id.nodeId, sessionId, "archive:\(archived)"))
    }
    func pinSession(id: DaemonId, sessionId: String, pinned: Bool) {
        sessionMutationCalls.append((id.nodeId, sessionId, "pin:\(pinned)"))
    }
    func deleteSession(id: DaemonId, sessionId: String) {
        sessionMutationCalls.append((id.nodeId, sessionId, "delete"))
    }
    func renameSession(id: DaemonId, sessionId: String, title: String) {
        sessionMutationCalls.append((id.nodeId, sessionId, "rename:\(title)"))
    }

    func nodeId() -> String { nodeIdResult }

    func setListener(listener: CoreListener) {
        self.listener = listener
    }
}
