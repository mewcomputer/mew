import Foundation
import MewMobileCore

/// A mock implementation of `MobileCoreProtocol` for unit testing.
/// Records all method calls and returns configurable snapshot data.
/// Events are delivered via `fireEvent` — the test controls timing.
final class MockMobileCore: MobileCoreProtocol {
    // Recorded calls
    var promptCalls: [(nodeId: String, text: String)] = []
    var slashCommandCalls: [(nodeId: String, command: String)] = []
    var setPermissionModeCalls: [(nodeId: String, mode: String)] = []
    var setThinkingVariantCalls: [(nodeId: String, variant: String)] = []
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

    func prompt(id: DaemonId, text: String) {
        promptCalls.append((id.nodeId, text))
    }

    func cancel(id: DaemonId) { cancelCalls.append(id.nodeId) }

    func respondPermission(id: DaemonId, requestId: String, decision: Decision) {}
    func respondAskUser(id: DaemonId, requestId: String, answers: [String]) {}
    func respondPlanApproval(id: DaemonId, requestId: String, approved: Bool, feedback: String?) {}
    func respondToGoal(id: DaemonId, requestId: String, accepted: Bool) {}

    func listSessions(id: DaemonId) { listSessionsCalls.append(id.nodeId) }
    func newSession(id: DaemonId, cwd: String?) { newSessionCalls.append(cwd) }
    func listProjects(id: DaemonId) {}
    func listDir(id: DaemonId, sessionId: String, path: String?) {}

    func listModels(id: DaemonId) { listModelsCalls.append(id.nodeId) }
    func switchModel(id: DaemonId, provider: String, model: String) {
        switchModelCalls.append((id.nodeId, provider, model))
    }

    func listPersonas(id: DaemonId) {}
    func switchPersona(id: DaemonId, name: String) {}

    func setAutoTitle(id: DaemonId, enabled: Bool) {}
    func setAutoSummary(id: DaemonId, enabled: Bool) {}

    func setPermissionMode(id: DaemonId, mode: String) {
        setPermissionModeCalls.append((id.nodeId, mode))
    }

    func slashCommand(id: DaemonId, command: String) {
        slashCommandCalls.append((id.nodeId, command))
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
