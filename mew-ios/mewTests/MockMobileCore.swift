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
    var cancelCalls: [String] = []
    var attachCalls: [(nodeId: String, sessionId: String)] = []
    var listSessionsCalls: [String] = []
    var listModelsCalls: [String] = []
    var newSessionCalls: [String?] = []

    // Configurable return values
    var snapshotResult: DaemonSnapshot?
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
        DaemonId(nodeId: nodeId)
    }

    func addDaemonWithToken(nodeId: String, name: String, token: String) -> DaemonId {
        DaemonId(nodeId: nodeId)
    }

    func removeDaemon(id: DaemonId) {}

    func listDaemons() -> [DaemonEntry] { daemonsResult }

    func connect(id: DaemonId) { connectCalls.append(id.nodeId) }
    func disconnect(id: DaemonId) {}

    func attach(id: DaemonId, sessionId: String) {
        attachCalls.append((id.nodeId, sessionId))
    }

    func snapshot(id: DaemonId) -> DaemonSnapshot? { snapshotResult }

    func prompt(id: DaemonId, text: String) -> Bool {
        promptCalls.append((id.nodeId, text))
        return promptAccepted
    }

    func cancel(id: DaemonId) { cancelCalls.append(id.nodeId) }

    func respondPermission(id: DaemonId, requestId: String, decision: Decision) {
        permissionResponseCalls.append((id.nodeId, requestId, decision))
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

    func listModels(id: DaemonId) { listModelsCalls.append(id.nodeId) }
    func switchModel(id: DaemonId, provider: String, model: String) {
        switchModelCalls.append((id.nodeId, provider, model))
    }

    func listPersonas(id: DaemonId) {}
    func switchPersona(id: DaemonId, name: String) {
        switchPersonaCalls.append((id.nodeId, name))
    }

    func setAutoTitle(id: DaemonId, enabled: Bool) {}
    func setAutoSummary(id: DaemonId, enabled: Bool) {}

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
