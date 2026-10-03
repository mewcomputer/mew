import XCTest
import MewMobileCore
@testable import mew

@MainActor
final class AppStoreEventHandlingTests: XCTestCase {

    private var store: AppStore!
    private var mock: MockMobileCore!
    private var daemonId: DaemonId!

    override func setUp() {
        super.setUp()
        mock = MockMobileCore()
        store = AppStore()
        store.injectCore(mock)
        daemonId = DaemonId(nodeId: "daemon-1")
        store.selectedDaemonId = daemonId
        store.selectedSessionId = "sess-1"
    }

    func testProviderCompletionKeepsRunBusyUntilDaemonActivityEnds() {
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true))
        store.handleEvent(.providerMessageEnded(daemon: "daemon-1", sessionId: "sess-1",
            inputTokens: 7, outputTokens: 2, cost: 0.01, failed: false, manifest: nil))
        XCTAssertTrue(store.isStreaming)
        store.streamingText = "last provider response"
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: false))
        XCTAssertFalse(store.isStreaming)
        XCTAssertTrue(store.streamingText.isEmpty)
    }

    func testOtherSessionsAndDaemonsCannotEndTheSelectedRun() {
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true))
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-2", sessionId: "sess-1", running: false))
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-2", running: false))
        store.handleEvent(.providerMessageEnded(daemon: "daemon-2", sessionId: "sess-1",
            inputTokens: 7, outputTokens: 2, cost: 0.01, failed: true, manifest: nil))
        XCTAssertTrue(store.isStreaming)
        XCTAssertNotEqual(store.lastTurnFailed["sess-1"], true)
    }

    func testPromptWaitsForAuthoritativeActivityAndLateTextCannotRestartRun() {
        store.sendPrompt("hello")
        XCTAssertFalse(store.isStreaming)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true))
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: false))
        store.handleEvent(.textDelta(daemon: "daemon-1", sessionId: "sess-1", partId: "late", delta: "late text"))
        XCTAssertFalse(store.isStreaming)
    }

    func testOtherSessionsAndDaemonsCannotChangeStreamingText() {
        store.handleEvent(.textDelta(daemon: "daemon-1", sessionId: "sess-1", partId: "own", delta: "own text"))
        store.handleEvent(.textDelta(daemon: "daemon-2", sessionId: "sess-1", partId: "other", delta: "wrong daemon"))
        store.handleEvent(.textDelta(daemon: "daemon-1", sessionId: "sess-2", partId: "other", delta: "wrong session"))
        XCTAssertEqual(store.streamingText, "own text")
        XCTAssertEqual(store.streamingPartId, "own")
    }

    func testSnapshotOnlyRestoresRunningForTheSelectedSession() {
        let other = SessionInfo(sessionId: "sess-2", title: "Other", messages: [],
            running: false, usageCost: 0, pendingPermissions: 0, pendingQuestions: 0,
            inputTokens: 0, outputTokens: 0, turns: 0, todos: [])
        let selected = SessionInfo(sessionId: "sess-1", title: "Selected", messages: [],
            running: true, usageCost: 0, pendingPermissions: 0, pendingQuestions: 0,
            inputTokens: 0, outputTokens: 0, turns: 0, todos: [])
        mock.snapshotResult = DaemonSnapshot(sessions: [other, selected], attachedSession: "sess-1",
            pendingPermissions: [], pendingAskUser: [], models: [], daemonVersion: nil,
            permissionMode: nil, currentModel: nil, currentProvider: nil, thinkingVariant: nil,
            currentPersona: nil, availablePersonas: [])
        store.handleEvent(.sessionReloaded(daemon: "daemon-2", sessionId: "sess-1"))
        XCTAssertFalse(store.isStreaming)
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertTrue(store.isStreaming)
    }

    func testGoalProposalPreservesIdentityAndRespondsOnOriginatingDaemon() {
        store.handleEvent(.goalProposed(daemon: "daemon-1", sessionId: "sess-1",
            requestId: "goal-1", callId: "call-1", objective: "finish the feature"))
        XCTAssertEqual(store.pendingGoals.count, 1)
        XCTAssertEqual(store.pendingGoals.first?.daemon, "daemon-1")
        XCTAssertEqual(store.pendingGoals.first?.sessionId, "sess-1")
        store.respondToGoal(requestId: "goal-1", accepted: true)
        XCTAssertEqual(mock.goalResponseCalls.first?.nodeId, "daemon-1")
        XCTAssertEqual(mock.goalResponseCalls.first?.requestId, "goal-1")
        XCTAssertEqual(mock.goalResponseCalls.first?.accepted, true)
    }

    func testPermissionsAndQuestionsPreserveOriginWhenSelectionChanges() {
        store.handleEvent(.permissionRequested(daemon: "daemon-2", sessionId: "sess-2", requestId: "p", toolName: "shell", input: "{}"))
        store.handleEvent(.askUserRequested(daemon: "daemon-2", sessionId: "sess-2", requestId: "q", callId: "call", questions: ["which file?"]))
        XCTAssertEqual(store.pendingPermissions.first?.daemon, "daemon-2")
        XCTAssertEqual(store.pendingPermissions.first?.sessionId, "sess-2")
        XCTAssertEqual(store.pendingAskUser.first?.daemon, "daemon-2")
        XCTAssertEqual(store.pendingAskUser.first?.sessionId, "sess-2")
        store.respondPermission(requestId: "p", decision: .allowOnce)
        store.respondAskUser(requestId: "q", answers: ["file.swift"])
        XCTAssertTrue(mock.permissionResponseCalls.isEmpty)
        XCTAssertTrue(mock.askUserResponseCalls.isEmpty)
        store.selectedDaemonId = DaemonId(nodeId: "daemon-2")
        store.selectedSessionId = "sess-2"
        store.respondPermission(requestId: "p", decision: .allowOnce)
        store.respondAskUser(requestId: "q", answers: ["file.swift"])
        XCTAssertEqual(mock.permissionResponseCalls.first?.nodeId, "daemon-2")
        XCTAssertEqual(mock.askUserResponseCalls.first?.nodeId, "daemon-2")
        XCTAssertEqual(mock.askUserResponseCalls.first?.answers, ["file.swift"])
    }

    func testReadyClearsOnlyOriginActionsWhileHistoryReloadPreservesReplay() {
        func replay(_ daemon: String) {
            store.handleEvent(.permissionRequested(daemon: daemon, sessionId: "sess-1", requestId: "p", toolName: "shell", input: "{}"))
            store.handleEvent(.askUserRequested(daemon: daemon, sessionId: "sess-1", requestId: "q", callId: "call", questions: ["which file?"]))
            store.handleEvent(.goalProposed(daemon: daemon, sessionId: "sess-1", requestId: "g", callId: "goal", objective: "finish"))
        }
        for daemon in ["daemon-1", "daemon-2"] { replay(daemon); replay(daemon) }
        XCTAssertEqual(store.pendingPermissions.count, 2)
        XCTAssertEqual(store.pendingAskUser.count, 2)
        XCTAssertEqual(store.pendingGoals.count, 2)
        store.handleEvent(.sessionReady(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertEqual(store.pendingPermissions.map(\.daemon), ["daemon-2"])
        XCTAssertEqual(store.pendingAskUser.map(\.daemon), ["daemon-2"])
        XCTAssertEqual(store.pendingGoals.map(\.daemon), ["daemon-2"])
        replay("daemon-1")
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertEqual(store.pendingPermissions.count, 2)
        XCTAssertEqual(store.pendingAskUser.count, 2)
        XCTAssertEqual(store.pendingGoals.count, 2)
        store.handleEvent(.requestResolved(daemon: "daemon-1", requestId: "p"))
        store.handleEvent(.requestResolved(daemon: "daemon-1", requestId: "q"))
        XCTAssertEqual(store.pendingPermissions.map(\.daemon), ["daemon-2"])
        XCTAssertEqual(store.pendingAskUser.map(\.daemon), ["daemon-2"])
    }

    func testAttentionCountsUpdateOnlyTheOriginatingDaemonSession() {
        let summary = SessionSummary(sessionId: "sess-1", title: "Test", state: "running",
            archived: false, pinned: false, pendingPermissions: 0, pendingQuestions: 0,
            usageCost: 0, cwd: nil, model: nil, provider: nil, createdAt: 0,
            lastMessageAt: nil, lastTurnFailed: false, groupId: nil,
            inputTokens: 0, outputTokens: 0, turns: 0, contextTokens: nil)
        store.sessionLists = ["daemon-1": [summary], "daemon-2": [summary]]
        store.handleEvent(.attentionChanged(daemon: "daemon-1", sessionId: "sess-1", pendingPermissions: 1, pendingQuestions: 2))
        store.handleEvent(.attentionChanged(daemon: "daemon-1", sessionId: "sess-2", pendingPermissions: 7, pendingQuestions: 8))
        XCTAssertEqual(store.sessionLists["daemon-1"]?.first?.pendingPermissions, 1)
        XCTAssertEqual(store.sessionLists["daemon-1"]?.first?.pendingQuestions, 2)
        XCTAssertEqual(store.sessionLists["daemon-2"]?.first?.pendingPermissions, 0)
        XCTAssertEqual(store.sessionLists["daemon-2"]?.first?.pendingQuestions, 0)
    }

    func testGoalProposalCannotBeAnsweredFromAnotherSession() {
        store.handleEvent(.goalProposed(daemon: "daemon-1", sessionId: "sess-1",
            requestId: "goal-1", callId: "call-1", objective: "finish the feature"))
        store.selectedSessionId = "sess-2"
        store.respondToGoal(requestId: "goal-1", accepted: false)
        XCTAssertTrue(mock.goalResponseCalls.isEmpty)
        XCTAssertEqual(store.pendingGoals.count, 1)
    }

    func testGoalProposalCanBeRejectedAndReplayDoesNotDuplicateIt() {
        let event = CoreEvent.goalProposed(daemon: "daemon-1", sessionId: "sess-1",
            requestId: "goal-1", callId: "call-1", objective: "finish the feature")
        store.handleEvent(event)
        store.handleEvent(event)
        XCTAssertEqual(store.pendingGoals.count, 1)
        store.respondToGoal(requestId: "goal-1", accepted: false)
        XCTAssertEqual(mock.goalResponseCalls.first?.accepted, false)
    }

    func testGoalProposalCannotBeAnsweredFromAnotherDaemon() {
        store.handleEvent(.goalProposed(daemon: "daemon-1", sessionId: "sess-1",
            requestId: "goal-1", callId: "call-1", objective: "finish the feature"))
        store.selectedDaemonId = DaemonId(nodeId: "daemon-2")
        store.respondToGoal(requestId: "goal-1", accepted: true)
        XCTAssertTrue(mock.goalResponseCalls.isEmpty)
    }

    func testGoalResolutionOnlyDismissesRequestsForItsDaemon() {
        for daemon in ["daemon-1", "daemon-2"] {
            store.handleEvent(.goalProposed(daemon: daemon, sessionId: "sess-1",
                requestId: "goal-1", callId: "call-1", objective: "finish the feature"))
        }
        store.handleEvent(.requestResolved(daemon: "daemon-1", requestId: "goal-1"))
        XCTAssertEqual(store.pendingGoals.map(\.daemon), ["daemon-2"])
    }

    // MARK: - AC.1: SlashResult renders as synthetic message

    func testSlashResultAppendsSyntheticMessage() {
        let initialCount = store.messages.count
        store.handleEvent(.slashResult(daemon: "daemon-1", sessionId: "sess-1", text: "context cleared"))
        XCTAssertEqual(store.messages.count, initialCount + 1)
        let msg = store.messages.last!
        XCTAssertEqual(msg.role, "assistant")
        XCTAssertEqual(msg.parts.first?.text, "context cleared")
        XCTAssertEqual(msg.parts.first?.kind, .text)
    }

    // MARK: - AC.2: PermissionModeChanged updates state

    func testPermissionModeChangedUpdatesState() {
        store.handleEvent(.permissionModeChanged(daemon: "daemon-1", mode: "dangerous"))
        XCTAssertEqual(store.permissionMode["daemon-1"], "dangerous")
    }

    func testPermissionModeChangedFromSessionReadySeedsSnapshot() {
        let snap = DaemonSnapshot(
            sessions: [], attachedSession: nil,
            pendingPermissions: [], pendingAskUser: [],
            models: [], daemonVersion: nil,
            permissionMode: "standard",
            currentModel: nil, currentProvider: nil, thinkingVariant: nil,
            currentPersona: nil, availablePersonas: []
        )
        mock.snapshotResult = snap
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertEqual(store.permissionMode["daemon-1"], "standard")
    }

    // MARK: - AC.3: ThinkingVariantChanged updates state

    func testThinkingVariantChangedUpdatesState() {
        store.handleEvent(.thinkingVariantChanged(daemon: "daemon-1", variant: "high"))
        XCTAssertEqual(store.thinkingVariant["daemon-1"], "high")
    }

    func testThinkingVariantChangedNoneClearsState() {
        store.handleEvent(.thinkingVariantChanged(daemon: "daemon-1", variant: "high"))
        store.handleEvent(.thinkingVariantChanged(daemon: "daemon-1", variant: nil))
        XCTAssertNil(store.thinkingVariant["daemon-1"] ?? nil)
    }

    // MARK: - AC.4: TodosUpdated stores payload

    func testTodosUpdatedStoresItems() {
        let todos = [
            TodoItem(id: 1, content: "Task A", status: "done", dependsOn: []),
            TodoItem(id: 2, content: "Task B", status: "pending", dependsOn: [1]),
        ]
        store.handleEvent(.todosUpdated(daemon: "daemon-1", sessionId: "sess-1", todos: todos))
        XCTAssertEqual(store.todos["sess-1"]?.count, 2)
        XCTAssertEqual(store.todos["sess-1"]?[0].content, "Task A")
        XCTAssertEqual(store.todos["sess-1"]?[1].dependsOn, [1])
    }

    func testTodosUpdatedReplacesOldList() {
        let todos1 = [TodoItem(id: 1, content: "A", status: "pending", dependsOn: [])]
        let todos2 = [TodoItem(id: 1, content: "A", status: "done", dependsOn: []),
                       TodoItem(id: 2, content: "B", status: "pending", dependsOn: [])]
        store.handleEvent(.todosUpdated(daemon: "daemon-1", sessionId: "sess-1", todos: todos1))
        store.handleEvent(.todosUpdated(daemon: "daemon-1", sessionId: "sess-1", todos: todos2))
        XCTAssertEqual(store.todos["sess-1"]?.count, 2)
        XCTAssertEqual(store.todos["sess-1"]?[0].status, "done")
    }

    // MARK: - AC.5: ProviderMessageEnded captures usage

    func testProviderMessageEndedCapturesUsage() {
        store.handleEvent(.providerMessageEnded(
            daemon: "daemon-1", sessionId: "sess-1",
            inputTokens: 500, outputTokens: 200,
            cost: 0.0123, failed: false, manifest: nil
        ))
        let usage = store.sessionUsage["sess-1"]
        XCTAssertNotNil(usage)
        XCTAssertEqual(usage?.inputTokens, 500)
        XCTAssertEqual(usage?.outputTokens, 200)
        XCTAssertEqual(usage?.cost ?? -1, 0.0123, accuracy: 0.0001)
        XCTAssertEqual(usage?.turns, 1) // first turn (no snapshot)
    }

    func testProviderMessageEndedSetsFailedFlag() {
        store.handleEvent(.providerMessageEnded(
            daemon: "daemon-1", sessionId: "sess-1",
            inputTokens: 0, outputTokens: 0,
            cost: 0.0, failed: true, manifest: nil
        ))
        XCTAssertEqual(store.lastTurnFailed["sess-1"], true)
    }

    // MARK: - AC.6: ModelSwitched updates state

    func testModelSwitchedUpdatesState() {
        store.handleEvent(.modelSwitched(daemon: "daemon-1", provider: "anthropic", model: "claude-4"))
        XCTAssertEqual(store.currentProvider["daemon-1"], "anthropic")
        XCTAssertEqual(store.currentModel["daemon-1"], "claude-4")
    }

    // MARK: - AC.7: Alert sets activeBanner for non-active session

    func testAlertSetsBannerForOtherSession() {
        store.selectedSessionId = "sess-1"
        store.handleEvent(.alert(
            daemon: "daemon-1", sessionId: "sess-2",
            kind: "permission", title: "Permission needed", detail: nil
        ))
        XCTAssertNotNil(store.activeBanner)
        XCTAssertEqual(store.activeBanner?.sessionId, "sess-2")
        XCTAssertEqual(store.activeBanner?.title, "Permission needed")
    }

    func testAlertDoesNotSetBannerForActiveSession() {
        store.selectedSessionId = "sess-1"
        store.handleEvent(.alert(
            daemon: "daemon-1", sessionId: "sess-1",
            kind: "permission", title: "Permission needed", detail: nil
        ))
        XCTAssertNil(store.activeBanner)
    }

    // MARK: - applySnapshot seeds all new state

    func testApplySnapshotSeedsUsageAndTodos() {
        let session = SessionInfo(
            sessionId: "sess-1", title: "Test", messages: [],
            running: false, usageCost: 0.05,
            pendingPermissions: 0, pendingQuestions: 0,
            inputTokens: 1000, outputTokens: 500, turns: 3,
            todos: [TodoItem(id: 1, content: "Task", status: "done", dependsOn: [])]
        )
        let snap = DaemonSnapshot(
            sessions: [session], attachedSession: "sess-1",
            pendingPermissions: [], pendingAskUser: [],
            models: [], daemonVersion: "1.0",
            permissionMode: "standard",
            currentModel: "gpt-4", currentProvider: "openai",
            thinkingVariant: "high", currentPersona: nil,
            availablePersonas: []
        )
        mock.snapshotResult = snap
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))

        XCTAssertEqual(store.sessionUsage["sess-1"]?.inputTokens, 1000)
        XCTAssertEqual(store.sessionUsage["sess-1"]?.turns, 3)
        XCTAssertEqual(store.todos["sess-1"]?.count, 1)
        XCTAssertEqual(store.permissionMode["daemon-1"], "standard")
        XCTAssertEqual(store.currentModel["daemon-1"], "gpt-4")
        XCTAssertEqual(store.currentProvider["daemon-1"], "openai")
        XCTAssertEqual(store.thinkingVariant["daemon-1"], "high")
    }
}
