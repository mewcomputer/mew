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

    func testToolDurationsUseMillisecondsAndHandleClockBoundaries() {
        let start: Int64 = 1_800_000_000_000
        let now = Date(timeIntervalSince1970: Double(start + 2_500) / 1_000)
        XCTAssertEqual(FormatHelper.toolDuration(start: start, end: start + 1_800, now: now), "1s")
        XCTAssertEqual(FormatHelper.toolDuration(start: start, end: start + 61_000, now: now), "1m 1s")
        XCTAssertEqual(FormatHelper.toolDuration(start: start, end: nil, now: now), "2s")
        XCTAssertEqual(FormatHelper.toolDuration(start: start, end: start + 999, now: now), "<1s")
        XCTAssertEqual(FormatHelper.toolDuration(start: start, end: start, now: now), "<1s")
        XCTAssertNil(FormatHelper.toolDuration(start: start, end: start - 1, now: now))
        XCTAssertNil(FormatHelper.toolDuration(start: nil, end: start, now: now))
        XCTAssertNil(FormatHelper.toolDuration(start: 0, end: start, now: now))
    }

    func testReviewRefreshKeepsLastResultUntilSuccessfulReplacement() {
        let file = ReviewFile(path: "src/main.rs", oldPath: nil, status: .modified, hunks: [], added: 1, removed: 1)
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        store.handleEvent(.reviewLoaded(daemon: "daemon-1", sessionId: "sess-1", requestId: store.review!.requestId, files: [file]))
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        XCTAssertEqual(store.review!.files.map(\.path), ["src/main.rs"])
        XCTAssertTrue(store.review!.isLoading)
        store.handleEvent(.reviewFailed(daemon: "daemon-1", sessionId: "sess-1", requestId: store.review!.requestId, message: "Connection lost"))
        XCTAssertEqual(store.review!.files.map(\.path), ["src/main.rs"])
        XCTAssertNotNil(store.review!.error)
        mock.reviewAccepted = false
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        XCTAssertEqual(store.review!.files.map(\.path), ["src/main.rs"])
        XCTAssertFalse(store.review!.isLoading)
        XCTAssertNotNil(store.review!.error)
        mock.reviewAccepted = true
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        XCTAssertNil(store.review!.error)
        store.handleEvent(.reviewLoaded(daemon: "daemon-1", sessionId: "sess-1", requestId: store.review!.requestId, files: []))
        XCTAssertTrue(store.review!.files.isEmpty)
    }

    func testWorkerRefreshKeepsTranscriptOnErrorButClearsItForAnotherWorker() {
        let messages = historySnapshot(before: nil, texts: ["worker findings"]).sessions[0].messages
        store.openWorkerHistory(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child")
        store.handleEvent(.workerHistoryLoaded(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child", requestId: store.workerHistory!.requestId, messages: messages, error: nil))
        store.openWorkerHistory(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child")
        XCTAssertEqual(store.workerHistory!.messages.map(\.id), ["worker findings"])
        store.handleEvent(.workerHistoryLoaded(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child", requestId: store.workerHistory!.requestId, messages: [], error: "Connection lost"))
        XCTAssertEqual(store.workerHistory!.messages.map(\.id), ["worker findings"])
        XCTAssertFalse(store.workerHistory!.isLoading)
        XCTAssertNotNil(store.workerHistory!.error)
        mock.detailAccepted = false
        store.openWorkerHistory(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child")
        XCTAssertEqual(store.workerHistory!.messages.map(\.id), ["worker findings"])
        XCTAssertFalse(store.workerHistory!.isLoading)
        XCTAssertNotNil(store.workerHistory!.error)
        mock.detailAccepted = true
        store.openWorkerHistory(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "other-child")
        XCTAssertTrue(store.workerHistory!.messages.isEmpty)
        store.handleEvent(.workerHistoryLoaded(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "other-child", requestId: store.workerHistory!.requestId, messages: messages, error: nil))
        store.openWorkerHistory(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "other-child")
        store.handleEvent(.workerHistoryLoaded(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "other-child", requestId: store.workerHistory!.requestId, messages: [], error: nil))
        XCTAssertTrue(store.workerHistory!.messages.isEmpty)
    }

    func testRefreshCannotReuseContentFromAnotherDaemonOrParentConversation() {
        let file = ReviewFile(path: "other/private.rs", oldPath: nil, status: .modified, hunks: [], added: 1, removed: 1)
        let messages = historySnapshot(before: nil, texts: ["other worker findings"]).sessions[0].messages
        for (daemon, session) in [("daemon-2", "sess-1"), ("daemon-1", "other-parent")] {
            store.review = WorkspaceReview(daemon: daemon, sessionId: session, requestId: "old", files: [file], isLoading: false)
            store.workerHistory = WorkerHistory(daemon: daemon, sessionId: session, childSessionId: "child", requestId: "old", messages: messages, isLoading: false)
            store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
            store.openWorkerHistory(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child")
            XCTAssertTrue(store.review!.files.isEmpty)
            XCTAssertTrue(store.workerHistory!.messages.isEmpty)
        }
    }

    func testWorkspaceReviewAcceptsOnlyTheCurrentDaemonSessionAndRequest() {
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        let request = store.review!.requestId
        XCTAssertTrue(store.review!.isLoading)
        let file = ReviewFile(path: "src/main.rs", oldPath: nil, status: .modified, hunks: [], added: 1, removed: 1)
        for (daemon, session, id) in [("daemon-2", "sess-1", request), ("daemon-1", "other", request), ("daemon-1", "sess-1", "old")] {
            store.handleEvent(.reviewLoaded(daemon: daemon, sessionId: session, requestId: id, files: [file]))
            XCTAssertTrue(store.review!.isLoading)
            XCTAssertTrue(store.review!.files.isEmpty)
        }
        store.handleEvent(.reviewLoaded(daemon: "daemon-1", sessionId: "sess-1", requestId: request, files: [file]))
        XCTAssertFalse(store.review!.isLoading)
        XCTAssertNil(store.review!.error)
        XCTAssertEqual(store.review!.files.map(\.path), ["src/main.rs"])
    }

    private func historySnapshot(before: UInt64?, texts: [String], running: Bool = false) -> DaemonSnapshot {
        let messages = texts.map { text in
            ChatMessage(id: text, role: "user", parts: [MessagePart(id: text, kind: .text, text: text,
                toolName: nil, toolState: nil, toolInput: nil, toolOutput: nil, toolError: nil,
                toolCallId: nil, toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil)], assistantMeta: nil)
        }
        return DaemonSnapshot(sessions: [SessionInfo(sessionId: "sess-1", title: "History", messages: messages,
            running: running, usageCost: 0, pendingPermissions: 0, pendingQuestions: 0,
            inputTokens: 0, outputTokens: 0, turns: 0, todos: [], historyBefore: before, subagents: [], jobs: [])],
            attachedSession: "sess-1", pendingPermissions: [], pendingAskUser: [], models: [],
            daemonVersion: nil, permissionMode: nil, currentModel: nil, currentProvider: nil,
            thinkingVariant: nil, currentPersona: nil, availablePersonas: [])
    }

    func testOlderHistoryKeepsMessagesUntilTheSelectedPageArrives() {
        mock.snapshotResult = historySnapshot(before: 24, texts: ["latest"], running: true)
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .connected))
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertEqual(store.historyBefore, 24)
        XCTAssertTrue(store.loadOlderHistory())
        XCTAssertFalse(store.loadOlderHistory())
        XCTAssertTrue(store.isLoadingOlderHistory)
        XCTAssertEqual(store.messages.map(\.id), ["latest"])
        store.handleEvent(.historyExtended(daemon: "daemon-2", sessionId: "sess-1"))
        store.handleEvent(.historyExtended(daemon: "daemon-1", sessionId: "other"))
        XCTAssertTrue(store.isLoadingOlderHistory)
        mock.snapshotResult = historySnapshot(before: nil, texts: ["older", "latest"], running: true)
        store.handleEvent(.historyExtended(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertEqual(store.messages.map(\.id), ["older", "latest"])
        XCTAssertTrue(store.isStreaming)
        XCTAssertFalse(store.isLoadingOlderHistory)
        XCTAssertNil(store.historyBefore)
        XCTAssertFalse(store.loadOlderHistory())
    }

    func testOlderHistorySendFailuresAndDisconnectsAllowRecovery() {
        mock.snapshotResult = historySnapshot(before: 24, texts: ["latest"])
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .connected))
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        mock.historyAccepted = false
        XCTAssertFalse(store.loadOlderHistory())
        XCTAssertFalse(store.isLoadingOlderHistory)
        XCTAssertNotNil(store.historyError)
        mock.historyAccepted = true
        XCTAssertTrue(store.loadOlderHistory())
        XCTAssertNil(store.historyError)
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .disconnected))
        XCTAssertFalse(store.isLoadingOlderHistory)
        XCTAssertNotNil(store.historyError)
        XCTAssertEqual(store.messages.map(\.id), ["latest"])
        store.selectSession("sess-2")
        XCTAssertNil(store.historyBefore)
        XCTAssertNil(store.historyError)
    }

    func testOlderHistoryShowsTheActiveAssistantPartOnceBetweenDeltas() {
        mock.snapshotResult = historySnapshot(before: 24, texts: ["latest"], running: true)
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        store.handleEvent(.textDelta(daemon: "daemon-1", sessionId: "sess-1", partId: "active", delta: "current"))
        var snapshot = historySnapshot(before: nil, texts: ["older", "latest"], running: true)
        let parts = [("completed", "earlier answer"), ("active", "current")].map { id, text in
            MessagePart(id: id, kind: .text, text: text, toolName: nil, toolState: nil,
                toolInput: nil, toolOutput: nil, toolError: nil, toolCallId: nil,
                toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil)
        }
        snapshot.sessions[0].messages.append(ChatMessage(id: "assistant", role: "assistant", parts: parts, assistantMeta: nil))
        mock.snapshotResult = snapshot

        store.handleEvent(.historyExtended(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertEqual(store.visibleMessages.flatMap(\.parts).compactMap(\.text), ["older", "latest", "earlier answer"])
        XCTAssertEqual(store.streamingText, "current")
        store.handleEvent(.textDelta(daemon: "daemon-1", sessionId: "sess-1", partId: "active", delta: " reply"))
        XCTAssertEqual(store.streamingText, "current reply")

        snapshot.sessions[0].messages[2].parts[1].text = "current reply"
        mock.snapshotResult = snapshot
        store.handleEvent(.partUpdated(daemon: "daemon-1", sessionId: "sess-1", partId: "active", partKind: "text", state: "completed"))
        XCTAssertEqual(store.visibleMessages.flatMap(\.parts).compactMap(\.text), ["older", "latest", "earlier answer", "current reply"])
        XCTAssertTrue(store.streamingText.isEmpty)
        XCTAssertNil(store.streamingPartId)
        XCTAssertTrue(store.isStreaming)
    }

    func testActivityAndWorkerHistoryStayWithTheirParentConversation() {
        let worker = WorkerInfo(taskId: "task", childSessionId: "child", name: "researcher", displayName: "Curie", status: "checking files", toolName: "read")
        store.handleEvent(.activityUpdated(daemon: "daemon-2", sessionId: "sess-1", subagents: [worker], jobs: []))
        XCTAssertTrue(store.subagents.isEmpty)
        store.handleEvent(.activityUpdated(daemon: "daemon-1", sessionId: "sess-1", subagents: [worker], jobs: []))
        XCTAssertEqual(store.subagents.first?.displayName, "Curie")
        store.openWorkerHistory(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child")
        let request = store.workerHistory!.requestId
        let messages = historySnapshot(before: nil, texts: ["child response"]).sessions[0].messages
        store.handleEvent(.workerHistoryLoaded(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "other", requestId: request, messages: messages, error: nil))
        XCTAssertTrue(store.workerHistory!.isLoading)
        store.handleEvent(.workerHistoryLoaded(daemon: "daemon-1", sessionId: "sess-1", childSessionId: "child", requestId: request, messages: messages, error: nil))
        XCTAssertEqual(store.workerHistory!.messages.map(\.id), ["child response"])
        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertEqual(store.selectedSessionId, "sess-1")
    }

    func testJobOutputUsesByteOffsetsAndRejectsDuplicateOrStaleChunks() {
        store.openJobOutput(daemon: "daemon-1", sessionId: "sess-1", jobId: "job")
        let first = store.jobOutput!.requestId
        let chunk = CoreEvent.jobOutputLoaded(daemon: "daemon-1", sessionId: "sess-1", jobId: "job", requestId: first,
            offset: 0, nextOffset: 3, output: "猫", done: false, incomplete: false)
        store.handleEvent(chunk)
        store.handleEvent(chunk)
        XCTAssertEqual(store.jobOutput!.output, "猫")
        XCTAssertEqual(store.jobOutput!.nextOffset, 3)
        store.loadMoreJobOutput()
        let second = store.jobOutput!.requestId
        store.handleEvent(.jobOutputFailed(daemon: "daemon-1", sessionId: "sess-1", jobId: "job", requestId: first, message: "old error"))
        XCTAssertTrue(store.jobOutput!.isLoading)
        store.handleEvent(.jobOutputLoaded(daemon: "daemon-1", sessionId: "sess-1", jobId: "job", requestId: second,
            offset: 3, nextOffset: 4, output: "!", done: true, incomplete: true))
        XCTAssertEqual(store.jobOutput!.output, "猫!")
        XCTAssertTrue(store.jobOutput!.done)
        XCTAssertTrue(store.jobOutput!.incomplete)
        XCTAssertNil(store.jobOutput!.error)
    }

    func testJobOutputErrorsAndDisconnectPreserveAlreadyReadOutput() {
        store.openJobOutput(daemon: "daemon-1", sessionId: "sess-1", jobId: "job")
        store.handleEvent(.jobOutputLoaded(daemon: "daemon-1", sessionId: "sess-1", jobId: "job", requestId: store.jobOutput!.requestId,
            offset: 0, nextOffset: 1, output: "a", done: false, incomplete: false))
        mock.detailAccepted = false
        store.loadMoreJobOutput()
        XCTAssertEqual(store.jobOutput!.output, "a")
        XCTAssertFalse(store.jobOutput!.isLoading)
        XCTAssertNotNil(store.jobOutput!.error)
        mock.detailAccepted = true
        store.loadMoreJobOutput()
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .disconnected))
        XCTAssertEqual(store.jobOutput!.output, "a")
        XCTAssertFalse(store.jobOutput!.isLoading)
        XCTAssertNotNil(store.jobOutput!.error)
        store.selectSession("other")
        XCTAssertNil(store.jobOutput)
        XCTAssertTrue(store.subagents.isEmpty)
    }

    func testJobOutputRejectsInvalidChunksWithoutLosingReadOutput() {
        store.openJobOutput(daemon: "daemon-1", sessionId: "sess-1", jobId: "job")
        store.handleEvent(.jobOutputLoaded(daemon: "daemon-1", sessionId: "sess-1", jobId: "job", requestId: store.jobOutput!.requestId,
            offset: 0, nextOffset: 3, output: "猫", done: false, incomplete: false))
        for (offset, nextOffset, output) in [(UInt64(0), UInt64(1), "x"), (3, 2, ""), (3, 4, "猫")] {
            store.loadMoreJobOutput()
            store.handleEvent(.jobOutputLoaded(daemon: "daemon-1", sessionId: "sess-1", jobId: "job", requestId: store.jobOutput!.requestId,
                offset: offset, nextOffset: nextOffset, output: output, done: true, incomplete: false))
            XCTAssertEqual(store.jobOutput!.output, "猫")
            XCTAssertEqual(store.jobOutput!.nextOffset, 3)
            XCTAssertFalse(store.jobOutput!.isLoading)
            XCTAssertFalse(store.jobOutput!.done)
            XCTAssertNotNil(store.jobOutput!.error)
        }
        store.loadMoreJobOutput()
        store.handleEvent(.jobOutputLoaded(daemon: "daemon-1", sessionId: "sess-1", jobId: "job", requestId: store.jobOutput!.requestId,
            offset: 3, nextOffset: 3, output: "", done: false, incomplete: false))
        XCTAssertEqual(store.jobOutput!.output, "猫")
        XCTAssertFalse(store.jobOutput!.isLoading)
        XCTAssertFalse(store.jobOutput!.done)
        XCTAssertNil(store.jobOutput!.error)
    }

    func testWorkspaceReviewRefreshRejectsLateSuccessAndFailure() {
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        let first = store.review!.requestId
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        let current = store.review!.requestId
        XCTAssertNotEqual(first, current)
        store.handleEvent(.reviewFailed(daemon: "daemon-1", sessionId: "sess-1", requestId: first, message: "old error"))
        store.handleEvent(.reviewLoaded(daemon: "daemon-1", sessionId: "sess-1", requestId: first, files: []))
        XCTAssertTrue(store.review!.isLoading)
        XCTAssertNil(store.review!.error)
        store.handleEvent(.reviewFailed(daemon: "daemon-1", sessionId: "sess-1", requestId: current, message: "not a git repository"))
        XCTAssertFalse(store.review!.isLoading)
        XCTAssertEqual(store.review!.error, "not a git repository")
    }

    func testEmptyWorkspaceReviewIsACompletedResult() {
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        store.handleEvent(.reviewLoaded(daemon: "daemon-1", sessionId: "sess-1", requestId: store.review!.requestId, files: []))
        XCTAssertFalse(store.review!.isLoading)
        XCTAssertTrue(store.review!.files.isEmpty)
        XCTAssertNil(store.review!.error)
    }

    func testWorkspaceReviewCannotShowResultsAfterSelectionChanges() {
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        let request = store.review!.requestId
        store.selectedDaemonId = DaemonId(nodeId: "daemon-2")
        store.handleEvent(.reviewFailed(daemon: "daemon-1", sessionId: "sess-1", requestId: request, message: "wrong workspace"))
        XCTAssertNil(store.review!.error)
        store.selectSession("sess-2")
        XCTAssertNil(store.review)
    }

    func testWorkspaceReviewDisconnectStopsLoadingAndIgnoresLateReplies() {
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        let request = store.review!.requestId
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-2", status: .disconnected))
        XCTAssertTrue(store.review!.isLoading)
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .disconnected))
        XCTAssertFalse(store.review!.isLoading)
        XCTAssertNotNil(store.review!.error)
        store.handleEvent(.reviewLoaded(daemon: "daemon-1", sessionId: "sess-1", requestId: request, files: []))
        XCTAssertNotNil(store.review!.error)
    }

    func testRejectedWorkspaceReviewShowsAnErrorAndCannotTargetAnotherSession() {
        mock.reviewAccepted = false
        store.refreshReview(daemon: "daemon-1", sessionId: "sess-1")
        XCTAssertFalse(store.review!.isLoading)
        XCTAssertNotNil(store.review!.error)
        store.refreshReview(daemon: "daemon-2", sessionId: "other")
        XCTAssertEqual(store.review!.daemon, "daemon-1")
        XCTAssertEqual(store.review!.sessionId, "sess-1")
    }

    func testRejectedPromptPreservesRetryTextAndWaitsForConfirmedAttachment() {
        mock.snapshotResult = DaemonSnapshot(sessions: [], attachedSession: "sess-1", pendingPermissions: [],
            pendingAskUser: [], models: [], daemonVersion: nil, permissionMode: nil,
            currentModel: nil, currentProvider: nil, thinkingVariant: nil,
            currentPersona: nil, availablePersonas: [])
        store.daemonStatuses["daemon-1"] = .connected
        mock.promptAccepted = false
        store.lastPrompt["sess-1"] = "previous"
        XCTAssertFalse(store.sendPrompt("new text"))
        XCTAssertEqual(store.lastPrompt["sess-1"], "previous")
        mock.promptAccepted = true
        XCTAssertTrue(store.sendPrompt("accepted"))
        XCTAssertEqual(store.lastPrompt["sess-1"], "accepted")
        store.selectedSessionId = "sess-2"
        XCTAssertFalse(store.canSendPrompt)
        XCTAssertFalse(store.sendPrompt("wrong session"))
        XCTAssertEqual(mock.promptCalls.count, 2)
        store.selectedSessionId = "sess-1"
        store.daemonStatuses["daemon-1"] = .backoff(attempt: 0, error: "offline")
        XCTAssertFalse(store.canSendPrompt)
    }

    func testRepeatedSubmissionReservesUntilAuthoritativeRunAndRecoversFromError() {
        mock.snapshotResult = DaemonSnapshot(sessions: [], attachedSession: "sess-1", pendingPermissions: [],
            pendingAskUser: [], models: [], daemonVersion: nil, permissionMode: nil,
            currentModel: nil, currentProvider: nil, thinkingVariant: nil,
            currentPersona: nil, availablePersonas: [])
        store.daemonStatuses["daemon-1"] = .connected
        XCTAssertTrue(store.sendPrompt("first"))
        XCTAssertFalse(store.isStreaming)
        XCTAssertFalse(store.canSendPrompt)
        XCTAssertFalse(store.sendPrompt("second"))
        XCTAssertEqual(mock.promptCalls.count, 1)
        store.handleEvent(.alert(daemon: "daemon-2", sessionId: "sess-1", kind: "error", title: "Other daemon", detail: nil))
        XCTAssertFalse(store.canSendPrompt)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-2", sessionId: "sess-1", running: true))
        XCTAssertFalse(store.canSendPrompt)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true))
        XCTAssertTrue(store.isStreaming)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: false))
        XCTAssertTrue(store.sendPrompt("next"))
        store.handleEvent(.alert(daemon: "daemon-1", sessionId: "sess-1", kind: "error", title: "Rejected", detail: nil))
        XCTAssertTrue(store.canSendPrompt)
        XCTAssertTrue(store.sendPrompt("retry"))
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .backoff(attempt: 0, error: "closed")))
        XCTAssertFalse(store.canSendPrompt)
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .connected))
        store.handleEvent(.sessionReady(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertTrue(store.canSendPrompt)
    }

    func testPersonaChoicesWaitForDaemonConfirmationIncludingDefault() {
        store.currentPersona["daemon-1"] = "planner"
        store.switchPersona(name: "default")
        XCTAssertEqual(mock.switchPersonaCalls.last?.name, "default")
        XCTAssertEqual(store.currentPersona["daemon-1"], "planner")
        store.switchPersona(name: "builder")
        XCTAssertEqual(store.currentPersona["daemon-1"], "planner")
        store.handleEvent(.personaList(daemon: "daemon-2", personas: []))
        XCTAssertEqual(store.currentPersona["daemon-1"], "planner")
        store.handleEvent(.personaList(daemon: "daemon-1", personas: []))
        XCTAssertNil(store.currentPersona["daemon-1"])
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
            inputTokens: 0, outputTokens: 0, turns: 0, todos: [], historyBefore: nil, subagents: [], jobs: [])
        let selected = SessionInfo(sessionId: "sess-1", title: "Selected", messages: [],
            running: true, usageCost: 0, pendingPermissions: 0, pendingQuestions: 0,
            inputTokens: 0, outputTokens: 0, turns: 0, todos: [], historyBefore: nil, subagents: [], jobs: [])
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

    func testChildQuestionRetainsIdentityAndCloseKeepsParentRequest() {
        let source = SubagentQuestionSource(parentCallId: "task", childSessionId: "child", name: "worker", displayName: "Curie")
        store.handleEvent(.askUserRequested(subagent: nil, daemon: "daemon-1", sessionId: "sess-1", requestId: "parent-q", callId: "parent-ask", questions: ["parent?"]))
        store.handleEvent(.askUserRequested(subagent: source, daemon: "daemon-1", sessionId: "sess-1", requestId: "child-q", callId: "child-ask", questions: ["child?"]))
        XCTAssertEqual(store.pendingAskUser.last?.subagent, source)
        store.cancelAskUser(requestId: "child-q")
        XCTAssertEqual(mock.askUserCancelCalls.first?.requestId, "child-q")
        XCTAssertTrue(mock.cancelCalls.isEmpty)
        store.handleEvent(.requestResolved(daemon: "daemon-1", requestId: "child-q"))
        XCTAssertEqual(store.pendingAskUser.map(\.requestId), ["parent-q"])
    }

    func testPermissionsAndQuestionsPreserveOriginWhenSelectionChanges() {
        store.handleEvent(.permissionRequested(daemon: "daemon-2", sessionId: "sess-2", requestId: "p", toolName: "shell", input: "{}"))
        store.handleEvent(.askUserRequested(subagent: nil, daemon: "daemon-2", sessionId: "sess-2", requestId: "q", callId: "call", questions: ["which file?"]))
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
            store.handleEvent(.askUserRequested(subagent: nil, daemon: daemon, sessionId: "sess-1", requestId: "q", callId: "call", questions: ["which file?"]))
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

    func testDeliveryErrorsAreVisibleAndRetainDaemonOrigin() {
        store.handleEvent(.alert(daemon: "daemon-1", sessionId: "sess-1",
            kind: "error", title: "Message delivery interrupted", detail: "Check history before retrying."))
        XCTAssertEqual(store.activeBanner?.daemon, "daemon-1")
        XCTAssertEqual(store.lastTurnFailed["sess-1"], true)
        store.lastTurnFailed["sess-1"] = false
        store.handleEvent(.alert(daemon: "daemon-2", sessionId: "sess-1",
            kind: "error", title: "Other daemon", detail: nil))
        XCTAssertEqual(store.activeBanner?.daemon, "daemon-2")
        XCTAssertEqual(store.lastTurnFailed["sess-1"], false)
    }

    func testOpeningAnAlertNavigatesOnItsOriginatingDaemon() {
        store.handleEvent(.alert(daemon: "daemon-2", sessionId: "sess-2",
            kind: "error", title: "Other daemon", detail: nil))
        let alert = store.activeBanner!
        store.openAlertSession(alert)
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "daemon-2")
        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "daemon-2", sessionId: "sess-2"))
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
            todos: [TodoItem(id: 1, content: "Task", status: "done", dependsOn: [])], historyBefore: nil, subagents: [], jobs: []
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
