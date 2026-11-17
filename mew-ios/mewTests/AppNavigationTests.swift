import XCTest
import SwiftUI
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

    func testListFilterPersistsIndependentlyOfTheOpenChatAndDraft() {
        store.daemons = [daemon("a"), daemon("b")]
        store.path = [.chat(daemonNodeId: "a", sessionId: "same")]
        store.drafts[.chat(daemonNodeId: "a", sessionId: "same")] = "unfinished"
        store.sessionFilter = "b"
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "a")
        XCTAssertEqual(store.selectedSessionId, "same")
        store.path = []
        let restored = AppStore(defaults: defaults)
        restored.injectCore(MockMobileCore())
        restored.daemons = store.daemons
        restored.restoreWorkingLocation()
        XCTAssertEqual(restored.sessionFilter, "b")
        XCTAssertTrue(restored.path.isEmpty)
        XCTAssertEqual(store.drafts[.chat(daemonNodeId: "a", sessionId: "same")], "unfinished")
        restored.sessionFilter = nil
        XCTAssertNil(AppStore(defaults: defaults).sessionFilter)
    }

    func testAllFilterSurvivesRestoringAChatFromAnotherMachine() {
        store.daemons = [daemon("a"), daemon("b")]
        store.sessionFilter = nil
        store.path = [.chat(daemonNodeId: "b", sessionId: "remote")]
        let restored = AppStore(defaults: defaults)
        restored.injectCore(MockMobileCore())
        restored.daemons = store.daemons
        restored.restoreWorkingLocation()
        XCTAssertNil(restored.sessionFilter)
        XCTAssertEqual(restored.path, [.chat(daemonNodeId: "b", sessionId: "remote")])
        restored.path = []
        XCTAssertNil(restored.selectedDaemonId)
        XCTAssertNil(restored.selectedSessionId)
    }

    func testRowMutationsUseTheirOwnMachineWithoutChangingTheActiveChat() {
        store.selectSession("same", daemonNodeId: "a")
        store.daemonStatuses["b"] = .connected
        store.archiveSession("same", daemonNodeId: "b", archived: true)
        store.pinSession("same", daemonNodeId: "b", pinned: true)
        store.renameSession("same", daemonNodeId: "b", title: "Remote")
        store.deleteSession("same", daemonNodeId: "b")
        XCTAssertEqual(core.sessionMutationCalls.map { $0.nodeId }, ["b", "b", "b", "b"])
        XCTAssertEqual(core.sessionMutationCalls.map { $0.sessionId }, ["same", "same", "same", "same"])
        XCTAssertEqual(core.sessionMutationCalls.map { $0.action }, ["archive:true", "pin:true", "rename:Remote", "delete"])
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "a")
        XCTAssertEqual(store.selectedSessionId, "same")
        store.daemonStatuses["b"] = .disconnected
        store.deleteSession("same", daemonNodeId: "b")
        XCTAssertEqual(core.sessionMutationCalls.count, 4)
    }

    func testCreationFromAllUsesTheChosenMachineAndCannotFollowAFilterChange() {
        store.daemons = [daemon("a"), daemon("b")]
        store.daemonStatuses["b"] = .connected
        XCTAssertTrue(store.newSession(cwd: "/remote/project", daemonNodeId: "b"))
        XCTAssertEqual(core.newSessionTargets, ["b"])
        XCTAssertNil(store.selectedDaemonId)
        store.handleEvent(.sessionCreated(daemon: "a", sessionId: "wrong"))
        XCTAssertTrue(store.path.isEmpty)
        store.handleEvent(.sessionCreated(daemon: "b", sessionId: "new"))
        XCTAssertEqual(store.path, [.chat(daemonNodeId: "b", sessionId: "new")])
        XCTAssertNil(store.sessionFilter)
        store.path = []
        XCTAssertTrue(store.newSession(cwd: "/remote/project", daemonNodeId: "b"))
        store.sessionFilter = "a"
        store.handleEvent(.sessionCreated(daemon: "b", sessionId: "late"))
        XCTAssertTrue(store.path.isEmpty)
    }

    func testProjectReadsAndOfflineCreationNeverSwitchMachines() {
        store.daemons = [daemon("a"), daemon("b")]
        store.selectSession("current", daemonNodeId: "a")
        store.fetchProjects(daemonNodeId: "b")
        XCTAssertEqual(core.projectCalls, ["b"])
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "a")
        XCTAssertFalse(store.newSession(cwd: "/remote/project", daemonNodeId: "b"))
        XCTAssertTrue(core.newSessionTargets.isEmpty)
        store.handleEvent(.sessionCreated(daemon: "b", sessionId: "late"))
        XCTAssertEqual(store.selectedSessionId, "current")
    }

    func testRemovingAFilteredMachineReturnsTheListToAll() {
        store.daemons = [daemon("a"), daemon("b")]
        core.daemonsResult = store.daemons
        store.sessionFilter = "b"
        store.removeDaemon(daemon("b"))
        XCTAssertNil(store.sessionFilter)
        XCTAssertTrue(store.path.isEmpty)
        XCTAssertEqual(store.daemons.map(\.nodeId), ["a"])
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

            XCTAssertEqual(restored.path, [.chat(daemonNodeId: "b", sessionId: "remote")])
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

    func testMissingSavedMachineReturnsToTheCombinedList() {
        defaults.set("removed", forKey: "mew.lastDaemon")
        defaults.set(["removed": "old-chat"], forKey: "mew.lastChats")
        store.daemons = [daemon("available")]

        store.restoreWorkingLocation()

        XCTAssertTrue(store.path.isEmpty)
        XCTAssertNil(store.selectedDaemonId)
        XCTAssertNil(store.selectedSessionId)
    }

    func testFirstLaunchShowsAllChatsForOneOrSeveralMachines() {
        store.daemons = [daemon("a")]
        store.restoreWorkingLocation()
        XCTAssertTrue(store.path.isEmpty)
        XCTAssertNil(store.sessionFilter)

        defaults.removePersistentDomain(forName: suiteName)
        let multiple = AppStore(defaults: defaults)
        multiple.injectCore(MockMobileCore())
        multiple.daemons = [daemon("a"), daemon("b")]
        multiple.restoreWorkingLocation()
        XCTAssertTrue(multiple.path.isEmpty)
        XCTAssertNil(multiple.selectedDaemonId)
    }

    func testConfirmedMissingRestoredChatFallsBackToTheCombinedList() {
        defaults.set("b", forKey: "mew.lastDaemon")
        defaults.set(["b": "gone"], forKey: "mew.lastChats")
        store.daemons = [daemon("a"), daemon("b")]
        store.restoreWorkingLocation()

        store.handleEvent(.sessionList(daemon: "a", sessions: []))
        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "b", sessionId: "gone"))
        store.handleEvent(.sessionList(daemon: "b", sessions: [summary("different")]))

        XCTAssertTrue(store.path.isEmpty)
        XCTAssertNil(store.sessionFilter)
        XCTAssertNil(store.selectedDaemonId)
        XCTAssertNil(store.selectedSessionId)
        XCTAssertTrue(store.messages.isEmpty)
    }

    func testExplicitNavigationBeforeStartupRestorationWinsOverSavedLocation() {
        defaults.set("a", forKey: "mew.lastDaemon")
        defaults.set(["a": "saved"], forKey: "mew.lastChats")
        store.daemons = [daemon("a"), daemon("b")]
        store.path = [.chat(daemonNodeId: "b", sessionId: "chosen")]

        store.restoreWorkingLocation()

        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "b", sessionId: "chosen"))
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "b")
        XCTAssertEqual(store.selectedSessionId, "chosen")
    }

    func testSwitchingMachinesRestoresTheirOwnRememberedChatAcrossStoreLifetimes() {
        let first = daemon("a")
        let second = daemon("b")
        store.daemons = [first, second]
        store.path = [.chat(daemonNodeId: "a", sessionId: "local")]
        store.path = [.chat(daemonNodeId: "b", sessionId: "remote")]

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
        store.path = [.chat(daemonNodeId: "a", sessionId: "same")]
        XCTAssertEqual(store.messages.first?.parts.first?.text, "machine a history")
        store.streamingPartId = "a-stream"
        store.streamingText = "machine a live response"
        store.isStreaming = true

        store.path = [.chat(daemonNodeId: "b", sessionId: "same")]

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
        store.path = [.chat(daemonNodeId: "a", sessionId: "chat")]
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
        store.path = [.chat(daemonNodeId: "a", sessionId: "pending")]
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
        store.path = [firstChat]
        store.drafts[firstChat] = "unfinished local question"
        store.path = [secondChat]
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

    func testReturningFromANewChatRefreshesConnectedMachinesWithoutChangingTheFilter() async throws {
        store.daemons = [daemon("a"), daemon("b")]
        store.handleEvent(.daemonStatusChanged(daemon: "a", status: .connected))
        store.handleEvent(.daemonStatusChanged(daemon: "b", status: .connected))
        store.sessionFilter = nil
        let window = UIWindow(windowScene: try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene))
        window.rootViewController = UIHostingController(rootView: RootView().environmentObject(store))
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        let initialRead = expectation(for: NSPredicate { _, _ in self.core.listSessionsCalls.count >= 2 }, evaluatedWith: nil)
        await fulfillment(of: [initialRead], timeout: 3)
        core.snapshotsByDaemon["b"] = snapshot("new", text: "new chat")
        XCTAssertTrue(store.newSession(daemonNodeId: "b"))
        store.handleEvent(.sessionCreated(daemon: "b", sessionId: "new"))
        try await Task.sleep(for: .milliseconds(350))
        core.listSessionsCalls = []
        store.path = []
        let refreshed = expectation(for: NSPredicate { _, _ in Set(self.core.listSessionsCalls) == ["a", "b"] }, evaluatedWith: nil)
        await fulfillment(of: [refreshed], timeout: 3)
        store.handleEvent(.sessionList(daemon: "b", sessions: [summary("new")]))
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: nil, includeArchived: false).flatMap(\.items).map { $0.session.sessionId }, ["new"])
        XCTAssertNil(store.sessionFilter)
        XCTAssertNil(store.selectedDaemonId)
    }

    func testMissingSavedFilterFallsBackToAllWithoutLosingTheRestoredChat() {
        defaults.set("removed", forKey: "mew.sessionFilter")
        defaults.set("a", forKey: "mew.lastDaemon")
        defaults.set(["a": "chat"], forKey: "mew.lastChats")
        let restored = AppStore(defaults: defaults)
        restored.injectCore(core)
        restored.daemons = [daemon("a")]
        restored.restoreWorkingLocation()
        XCTAssertNil(restored.sessionFilter)
        XCTAssertNil(defaults.string(forKey: "mew.sessionFilter"))
        XCTAssertEqual(restored.path, [.chat(daemonNodeId: "a", sessionId: "chat")])
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
