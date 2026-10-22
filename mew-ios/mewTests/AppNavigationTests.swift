import XCTest
import MewMobileCore
@testable import mew

@MainActor
final class AppNavigationTests: XCTestCase {
    private var suiteName: String!
    private var defaults: UserDefaults!
    private var store: AppStore!
    private var core: MockMobileCore!

    override func setUp() {
        super.setUp()
        suiteName = "mew.navigation-tests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        core = MockMobileCore()
        store = AppStore(defaults: defaults)
        store.injectCore(core)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        store = nil
        core = nil
        defaults = nil
        suiteName = nil
        super.tearDown()
    }

    func testSavedDaemonAndChatRestoreRegardlessOfConnectionOrder() {
        defaults.set("b", forKey: "mew.lastDaemon")
        defaults.set(["a": "local", "b": "remote"], forKey: "mew.lastChats")
        for order in [["a", "b"], ["b", "a"]] {
            let restored = AppStore(defaults: defaults)
            let transport = MockMobileCore()
            restored.injectCore(transport)
            restored.daemons = [daemon("a"), daemon("b")]
            transport.snapshotsByDaemon["b"] = snapshot("remote", text: "remote history")

            restored.restoreWorkingLocation()
            for id in order {
                restored.handleEvent(.daemonStatusChanged(daemon: id, status: .connected))
            }
            restored.handleEvent(.sessionReloaded(daemon: "b", sessionId: "remote"))

            XCTAssertEqual(restored.path, [.sessions(daemonNodeId: "b"), .chat(daemonNodeId: "b", sessionId: "remote")])
            XCTAssertEqual(restored.selectedDaemonId?.nodeId, "b")
            XCTAssertEqual(restored.selectedSessionId, "remote")
            XCTAssertEqual(restored.messages.first?.parts.first?.text, "remote history")
        }
    }

    func testSavedOfflineMachineRemainsSelectedWhenAnotherMachineConnects() {
        defaults.set("b", forKey: "mew.lastDaemon")
        defaults.set(["b": "remote"], forKey: "mew.lastChats")
        store.daemons = [daemon("a"), daemon("b")]
        store.daemonStatuses["b"] = .disconnected

        store.restoreWorkingLocation()
        store.handleEvent(.daemonStatusChanged(daemon: "a", status: .connected))
        store.handleEvent(.daemonStatusChanged(daemon: "b", status: .backoff(attempt: 1, error: "Machine is offline")))

        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "b", sessionId: "remote"))
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "b")
        XCTAssertEqual(store.selectedSessionId, "remote")
        XCTAssertFalse(store.canSendPrompt)
    }

    func testMissingSavedMachineReturnsToThePicker() {
        defaults.set("removed", forKey: "mew.lastDaemon")
        defaults.set(["removed": "old-chat"], forKey: "mew.lastChats")
        store.daemons = [daemon("available")]

        store.restoreWorkingLocation()

        XCTAssertTrue(store.path.isEmpty)
        XCTAssertNil(store.selectedDaemonId)
        XCTAssertNil(store.selectedSessionId)
    }

    func testFirstLaunchOpensTheOnlyMachineButKeepsThePickerForSeveral() {
        store.daemons = [daemon("a")]
        store.restoreWorkingLocation()
        XCTAssertEqual(store.path, [.sessions(daemonNodeId: "a")])

        defaults.removePersistentDomain(forName: suiteName)
        let multiple = AppStore(defaults: defaults)
        multiple.injectCore(MockMobileCore())
        multiple.daemons = [daemon("a"), daemon("b")]
        multiple.restoreWorkingLocation()
        XCTAssertTrue(multiple.path.isEmpty)
        XCTAssertNil(multiple.selectedDaemonId)
    }

    func testConfirmedMissingRestoredChatFallsBackOnlyToItsOwnMachineList() {
        defaults.set("b", forKey: "mew.lastDaemon")
        defaults.set(["b": "gone"], forKey: "mew.lastChats")
        store.daemons = [daemon("a"), daemon("b")]
        store.restoreWorkingLocation()

        store.handleEvent(.sessionList(daemon: "a", sessions: []))
        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "b", sessionId: "gone"))
        store.handleEvent(.sessionList(daemon: "b", sessions: [summary("different")]))

        XCTAssertEqual(store.path, [.sessions(daemonNodeId: "b")])
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "b")
        XCTAssertNil(store.selectedSessionId)
        XCTAssertTrue(store.messages.isEmpty)
    }

    func testExplicitNavigationBeforeStartupRestorationWinsOverSavedLocation() {
        defaults.set("a", forKey: "mew.lastDaemon")
        defaults.set(["a": "saved"], forKey: "mew.lastChats")
        store.daemons = [daemon("a"), daemon("b")]
        store.path = [.sessions(daemonNodeId: "b"), .chat(daemonNodeId: "b", sessionId: "chosen")]

        store.restoreWorkingLocation()

        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "b", sessionId: "chosen"))
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "b")
        XCTAssertEqual(store.selectedSessionId, "chosen")
    }

    func testSwitchingMachinesRestoresTheirOwnRememberedChatAcrossStoreLifetimes() {
        let first = daemon("a")
        let second = daemon("b")
        store.daemons = [first, second]
        store.path = [.sessions(daemonNodeId: "a"), .chat(daemonNodeId: "a", sessionId: "local")]
        store.path = [.sessions(daemonNodeId: "b"), .chat(daemonNodeId: "b", sessionId: "remote")]

        store.openDaemon(first)
        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "a", sessionId: "local"))
        store.openDaemon(second)
        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "b", sessionId: "remote"))

        let reopened = AppStore(defaults: defaults)
        reopened.injectCore(MockMobileCore())
        reopened.daemons = [first, second]
        reopened.restoreWorkingLocation()
        XCTAssertEqual(reopened.path.last, .chat(daemonNodeId: "b", sessionId: "remote"))
        reopened.openDaemon(first)
        XCTAssertEqual(reopened.path.last, .chat(daemonNodeId: "a", sessionId: "local"))
    }

    func testIdenticalSessionIdsOnDifferentMachinesCannotShowThePreviousTranscript() {
        store.daemons = [daemon("a"), daemon("b")]
        core.snapshotsByDaemon["a"] = snapshot("same", text: "machine a history")
        store.path = [.sessions(daemonNodeId: "a"), .chat(daemonNodeId: "a", sessionId: "same")]
        XCTAssertEqual(store.messages.first?.parts.first?.text, "machine a history")
        store.streamingPartId = "a-stream"
        store.streamingText = "machine a live response"
        store.isStreaming = true

        store.path = [.sessions(daemonNodeId: "b"), .chat(daemonNodeId: "b", sessionId: "same")]

        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertTrue(store.streamingText.isEmpty)
        XCTAssertNil(store.streamingPartId)
        XCTAssertFalse(store.isStreaming)
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "b")
        XCTAssertEqual(core.attachCalls.last?.nodeId, "b")
    }

    func testRemovingTheSelectedMachineClearsItsRouteAndStaleChatState() {
        let removed = daemon("a")
        store.daemons = [removed]
        core.snapshotsByDaemon["a"] = snapshot("chat", text: "old history")
        store.path = [.sessions(daemonNodeId: "a"), .chat(daemonNodeId: "a", sessionId: "chat")]
        store.streamingText = "old live response"
        store.streamingPartId = "old-part"
        store.isStreaming = true
        store.review = WorkspaceReview(daemon: "a", sessionId: "chat", requestId: "review")
        store.historyBefore = 24
        core.daemonsResult = []

        store.removeDaemon(removed)

        XCTAssertTrue(store.path.isEmpty)
        XCTAssertNil(store.selectedDaemonId)
        XCTAssertNil(store.selectedSessionId)
        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertTrue(store.streamingText.isEmpty)
        XCTAssertNil(store.streamingPartId)
        XCTAssertFalse(store.isStreaming)
        XCTAssertNil(store.review)
        XCTAssertNil(store.historyBefore)
        XCTAssertNil(defaults.string(forKey: "mew.lastDaemon"))
    }

    func testRemovedMachineAlertsCannotReopenItsChat() {
        let removed = daemon("a")
        store.daemons = [removed]
        let alert = AlertItem(daemon: "a", sessionId: "old", kind: "question", title: "Input", detail: nil)
        store.activeBanner = alert
        store.alerts = [alert]
        store.removeDaemon(removed)
        XCTAssertNil(store.activeBanner)
        XCTAssertTrue(store.alerts.isEmpty)
        store.openAlertSession(alert)
        XCTAssertTrue(store.path.isEmpty)
        store.handleEvent(.alert(daemon: "a", sessionId: "old", kind: "question", title: "Late", detail: nil))
        XCTAssertNil(store.activeBanner)
    }

    func testDetailsNavigationDoesNotRepeatAPendingChatAttachment() {
        store.path = [.sessions(daemonNodeId: "a"), .chat(daemonNodeId: "a", sessionId: "pending")]
        XCTAssertEqual(core.attachCalls.count, 1)
        store.path.append(.daemonDetails(daemonNodeId: "a"))
        store.path.removeLast()
        XCTAssertEqual(core.attachCalls.count, 1)
        XCTAssertEqual(store.selectedSessionId, "pending")
    }

    func testDraftsStayWithTheirDaemonAndChatAcrossSwitches() {
        let first = daemon("a")
        let second = daemon("b")
        let firstChat = NavigationRoute.chat(daemonNodeId: "a", sessionId: "same")
        let secondChat = NavigationRoute.chat(daemonNodeId: "b", sessionId: "same")
        store.daemons = [first, second]
        store.path = [.sessions(daemonNodeId: "a"), firstChat]
        store.drafts[firstChat] = "unfinished local question"
        store.path = [.sessions(daemonNodeId: "b"), secondChat]
        store.drafts[secondChat] = "unfinished remote question"

        store.openDaemon(first)

        XCTAssertEqual(store.path.last, firstChat)
        XCTAssertEqual(store.drafts[firstChat], "unfinished local question")
        XCTAssertEqual(store.drafts[secondChat], "unfinished remote question")
    }

    func testSelectedModelResolvesByMachineAndProviderWhenCatalogsArriveLater() {
        let wrongProvider = model("a-wrong", provider: "other")
        let selected = model("a-selected", provider: "wanted")
        let otherMachine = model("b-model", provider: "wanted")
        store.handleEvent(.modelSwitched(daemon: "a", provider: "wanted", model: "shared-name"))
        XCTAssertNil(store.selectedModel(daemonNodeId: "a"))

        store.handleEvent(.modelList(daemon: "a", models: [wrongProvider, selected]))
        store.handleEvent(.modelList(daemon: "b", models: [otherMachine]))

        XCTAssertEqual(store.selectedModel(daemonNodeId: "a")?.id, "a-selected")
        XCTAssertEqual(store.modelsByDaemon["a"]?.map(\.id), ["a-wrong", "a-selected"])
        XCTAssertEqual(store.modelsByDaemon["b"]?.map(\.id), ["b-model"])
    }

    private func daemon(_ id: String) -> DaemonEntry {
        DaemonEntry(nodeId: id, name: "Machine \(id)", addedAt: 1, lastConnectedAt: nil,
            lastKnownVersion: nil, keepConnected: true, pairingToken: nil)
    }

    private func summary(_ id: String) -> SessionSummary {
        SessionSummary(sessionId: id, title: id, state: "idle", archived: false, pinned: false,
            pendingPermissions: 0, pendingQuestions: 0, usageCost: 0, cwd: nil, model: nil, provider: nil,
            createdAt: 1, lastMessageAt: 1, lastTurnFailed: false, groupId: nil,
            inputTokens: 0, outputTokens: 0, turns: 0, contextTokens: nil)
    }

    private func model(_ id: String, provider: String) -> ModelSummary {
        ModelSummary(id: id, provider: provider, model: "shared-name", description: nil,
            contextWindow: 200_000, thinkingVariants: [])
    }

    private func snapshot(_ id: String, text: String) -> DaemonSnapshot {
        let part = MessagePart(id: "\(id)-text", kind: .text, text: text, toolName: nil, toolState: nil,
            toolInput: nil, toolOutput: nil, toolError: nil, toolCallId: nil,
            toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil)
        let message = ChatMessage(id: "\(id)-message", role: "assistant", parts: [part], assistantMeta: nil)
        let session = SessionInfo(sessionId: id, title: id, messages: [message], running: false,
            usageCost: 0, pendingPermissions: 0, pendingQuestions: 0,
            inputTokens: 0, outputTokens: 0, turns: 0, todos: [], historyBefore: nil, subagents: [], jobs: [])
        return DaemonSnapshot(sessions: [session], attachedSession: id, pendingPermissions: [], pendingAskUser: [],
            models: [], daemonVersion: nil, permissionMode: nil, currentModel: nil, currentProvider: nil,
            thinkingVariant: nil, currentPersona: nil, availablePersonas: [], automationSettings: nil)
    }
}
