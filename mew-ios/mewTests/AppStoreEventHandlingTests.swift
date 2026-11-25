import XCTest
import SwiftUI
import Combine
import MewMobileCore
@testable import mew

@MainActor
final class AppStoreEventHandlingTests: XCTestCase {

    private var store: AppStore!
    private var mock: MockMobileCore!
    private var daemonId: DaemonId!
    private var suiteName: String!

    override func setUp() {
        super.setUp()
        mock = MockMobileCore()
        suiteName = "mew.store-tests.\(UUID().uuidString)"
        store = AppStore(defaults: UserDefaults(suiteName: suiteName)!)
        store.isAppActive = true
        store.injectCore(mock)
        daemonId = DaemonId(nodeId: "daemon-1")
        store.selectedDaemonId = daemonId
        store.selectedSessionId = "sess-1"
    }

    override func tearDown() {
        UserDefaults(suiteName: suiteName)?.removePersistentDomain(forName: suiteName)
        store = nil
        mock = nil
        super.tearDown()
    }

    func testManagementReadsRequireTheirOwnDaemonAndRequestBeforePublishingDefaults() throws {
        store.daemonStatuses["daemon-2"] = .connected
        store.readDaemonManagementSettings(daemonNodeId: "daemon-2")
        let id = try XCTUnwrap(mock.managementReadCalls.first?.requestId)
        XCTAssertNil(store.managementSettings["daemon-2"])
        let settings = DaemonManagementSettings(defaults: nil, devices: nil)
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: id, settings: settings))
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-2", requestId: "old", settings: settings))
        XCTAssertNil(store.managementSettings["daemon-1"])
        XCTAssertNil(store.managementSettings["daemon-2"])
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-2", requestId: id, settings: settings))
        XCTAssertNotNil(store.managementSettings["daemon-2"])
        XCTAssertNil(store.managementSettings["daemon-2"]?.defaults)
        XCTAssertNil(store.managementSettings["daemon-2"]?.devices)
        XCTAssertNil(store.managementRequestIds["daemon-2"])
    }

    func testManagementFailureKeepsConfirmedValuesAndCanBeRetried() throws {
        store.daemonStatuses["daemon-1"] = .connected
        store.managementSettings["daemon-1"] = managementSnapshot(model: "confirmed")
        store.resetSessionDefaults(daemonNodeId: "daemon-1")
        let id = try XCTUnwrap(mock.resetDefaultsCalls.first?.requestId)
        store.handleEvent(.daemonManagementFailed(daemon: "daemon-1", requestId: "old", message: "late"))
        XCTAssertNil(store.managementErrors["daemon-1"])
        store.handleEvent(.daemonManagementFailed(daemon: "daemon-1", requestId: id, message: "Read only"))
        XCTAssertEqual(store.managementSettings["daemon-1"]?.defaults?.model, "confirmed")
        XCTAssertEqual(store.managementErrors["daemon-1"], "Read only")
        XCTAssertNil(store.managementRequestIds["daemon-1"])
        store.readDaemonManagementSettings(daemonNodeId: "daemon-1")
        XCTAssertNil(store.managementErrors["daemon-1"])
        XCTAssertEqual(mock.managementReadCalls.count, 1)
    }

    func testUnsolicitedManagementUpdatesDoNotFinishAnotherPendingMutation() throws {
        store.daemonStatuses["daemon-1"] = .connected
        store.managementSettings["daemon-1"] = managementSnapshot(model: "old")
        store.resetSessionDefaults(daemonNodeId: "daemon-1")
        let id = try XCTUnwrap(mock.resetDefaultsCalls.first?.requestId)
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: nil, settings: managementSnapshot(model: "elsewhere")))
        XCTAssertEqual(store.managementSettings["daemon-1"]?.defaults?.model, "elsewhere")
        XCTAssertEqual(store.managementRequestIds["daemon-1"], id)
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: id,
            settings: DaemonManagementSettings(defaults: nil, devices: [])))
        XCTAssertNil(store.managementRequestIds["daemon-1"])
        XCTAssertNil(store.managementSettings["daemon-1"]?.defaults)
    }

    func testDisconnectInvalidatesManagementRepliesAndQueuedMutations() throws {
        store.daemonStatuses["daemon-1"] = .connected
        store.readDaemonManagementSettings(daemonNodeId: "daemon-1")
        let id = try XCTUnwrap(mock.managementReadCalls.first?.requestId)
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .disconnected))
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: id, settings: managementSnapshot(model: "late")))
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: nil, settings: managementSnapshot(model: "late")))
        store.resetSessionDefaults(daemonNodeId: "daemon-1")
        XCTAssertNil(store.managementSettings["daemon-1"])
        XCTAssertNil(store.managementRequestIds["daemon-1"])
        XCTAssertTrue(mock.resetDefaultsCalls.isEmpty)
        XCTAssertNotNil(store.managementErrors["daemon-1"])
    }

    func testSavingDefaultsRequiresTheConfirmedChatAndWaitsForAcknowledgment() throws {
        mock.snapshotResult = historySnapshot(before: nil, texts: [])
        store.daemonStatuses["daemon-1"] = .connected
        store.managementSettings["daemon-1"] = managementSnapshot(model: "old")
        store.saveSessionDefaults(daemonNodeId: "daemon-2", sessionId: "sess-1")
        store.saveSessionDefaults(daemonNodeId: "daemon-1", sessionId: "other")
        XCTAssertTrue(mock.saveDefaultsCalls.isEmpty)
        store.saveSessionDefaults(daemonNodeId: "daemon-1", sessionId: "sess-1")
        let call = try XCTUnwrap(mock.saveDefaultsCalls.first)
        XCTAssertEqual(call.nodeId, "daemon-1")
        XCTAssertEqual(call.sessionId, "sess-1")
        XCTAssertEqual(store.managementSettings["daemon-1"]?.defaults?.model, "old")
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: call.requestId, settings: managementSnapshot(model: "saved")))
        XCTAssertEqual(store.managementSettings["daemon-1"]?.defaults?.model, "saved")
    }

    func testRevokeUsesAConfirmedDeviceOnItsOwnMachineWithoutOptimisticRemoval() throws {
        store.daemonStatuses["daemon-2"] = .connected
        store.managementSettings["daemon-2"] = managementSnapshot(model: "default")
        store.revokeDevice(daemonNodeId: "daemon-2", deviceId: "unknown")
        XCTAssertTrue(mock.revokeDeviceCalls.isEmpty)
        store.revokeDevice(daemonNodeId: "daemon-2", deviceId: "phone")
        let call = try XCTUnwrap(mock.revokeDeviceCalls.first)
        XCTAssertEqual(call.nodeId, "daemon-2")
        XCTAssertEqual(call.deviceId, "phone")
        XCTAssertEqual(store.managementSettings["daemon-2"]?.devices?.count, 1)
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-2", requestId: call.requestId,
            settings: DaemonManagementSettings(defaults: nil, devices: [])))
        XCTAssertEqual(store.managementSettings["daemon-2"]?.devices?.count, 0)
    }

    func testManagementRequestsDoNotOverlapAndRejectedSendsNeverInventSettings() {
        store.daemonStatuses["daemon-1"] = .connected
        mock.settingsAccepted = false
        store.readDaemonManagementSettings(daemonNodeId: "daemon-1")
        XCTAssertNil(store.managementRequestIds["daemon-1"])
        XCTAssertNil(store.managementSettings["daemon-1"])
        XCTAssertNotNil(store.managementErrors["daemon-1"])
        mock.settingsAccepted = true
        store.readDaemonManagementSettings(daemonNodeId: "daemon-1")
        store.readDaemonManagementSettings(daemonNodeId: "daemon-1")
        store.resetSessionDefaults(daemonNodeId: "daemon-1")
        XCTAssertEqual(mock.managementReadCalls.count, 2)
        XCTAssertTrue(mock.resetDefaultsCalls.isEmpty)
    }

    func testUnsupportedManagementRequestsTimeOutAndIgnoreLateReplies() async throws {
        store.daemonStatuses["daemon-1"] = .connected
        mock.snapshotResult = historySnapshot(before: nil, texts: [])
        XCTAssertTrue(store.sendPrompt("keep pending"))
        let timedOut = expectation(description: "Unconfirmed management request expires")
        let observation = store.$managementErrors.sink { errors in
            if errors["daemon-1"] != nil { timedOut.fulfill() }
        }
        store.readDaemonManagementSettings(daemonNodeId: "daemon-1")
        let id = try XCTUnwrap(mock.managementReadCalls.first?.requestId)
        await fulfillment(of: [timedOut], timeout: 7)
        observation.cancel()
        XCTAssertFalse(store.canSendPrompt, "Settings compatibility must not release the pending chat submission")
        XCTAssertNotEqual(store.lastTurnFailed["sess-1"], true)
        XCTAssertTrue(store.alerts.isEmpty)
        XCTAssertNil(store.managementRequestIds["daemon-1"])
        XCTAssertNil(store.managementSettings["daemon-1"])
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: id, settings: managementSnapshot(model: "late")))
        XCTAssertNil(store.managementSettings["daemon-1"])
    }

    func testDeviceDetailStaysOpenAfterConfirmedRevocationRemovesItsListRow() async throws {
        let settings = managementSnapshot(model: "saved")
        let device = try XCTUnwrap(settings.devices?.first)
        store.daemonStatuses["daemon-1"] = .connected
        store.managementSettings["daemon-1"] = settings
        let controller = UIHostingController(rootView: NavigationStack(path: .constant([device])) {
            PairedDevicesView(daemonNodeId: "daemon-1")
        }.environmentObject(store))
        let window = UIWindow(windowScene: try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene))
        window.rootViewController = controller
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        func navigationBar(in view: UIView) -> UINavigationBar? {
            if let bar = view as? UINavigationBar { return bar }
            return view.subviews.lazy.compactMap { navigationBar(in: $0) }.first
        }
        let opened = expectation(for: NSPredicate { _, _ in
            navigationBar(in: controller.view)?.topItem?.title == device.name
        }, evaluatedWith: nil)
        await fulfillment(of: [opened], timeout: 3)
        let readId = try XCTUnwrap(store.managementRequestIds["daemon-1"])
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: readId, settings: settings))
        store.revokeDevice(daemonNodeId: "daemon-1", deviceId: device.id)
        let revokeId = try XCTUnwrap(mock.revokeDeviceCalls.first?.requestId)
        store.handleEvent(.daemonManagementSettings(daemon: "daemon-1", requestId: revokeId,
            settings: DaemonManagementSettings(defaults: settings.defaults, devices: [])))
        // Allow navigation to react to removal of the source row.
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(navigationBar(in: controller.view)?.topItem?.title, device.name)
        XCTAssertEqual(store.managementSettings["daemon-1"]?.devices, [])
    }

    private func managementSnapshot(model: String) -> DaemonManagementSettings {
        DaemonManagementSettings(defaults: NewChatDefaults(model: model, provider: "test", persona: "builder",
            thinkingVariant: nil, permissionMode: "standard"),
            devices: [PairedDevice(id: "phone", name: "My phone", nodeId: "phone-node", scope: "control", pairedAt: 123)])
    }

    func testToolOnlyMessagesStayCompactInConversation() {
        let part = MessagePart(id: "tool", kind: .toolCall, text: nil, toolName: "read", toolState: "completed",
            toolInput: "{}", toolOutput: "file contents", toolError: nil, toolCallId: "call",
            toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)
        let message = ChatMessage(id: "assistant", role: "assistant", parts: [part], assistantMeta: nil, steered: false, cancelled: false)
        let controller = UIHostingController(rootView: MessageItemView(message: message, fontChoice: .system, disclosures: .constant(ToolDisclosures()))
            .environment(\.dynamicTypeSize, .large))
        let size = controller.sizeThatFits(in: CGSize(width: 393, height: 1_000))
        XCTAssertLessThanOrEqual(size.height, 50, "Tool-only messages should not inherit text-bubble padding")
        XCTAssertGreaterThanOrEqual(size.height, 44)
    }

    func testMixedToolsCollapseIntoOneActivityRow() {
        let parts = ["write", "bash", "bash"].enumerated().map { index, name in
            MessagePart(id: "tool-\(index)", kind: .toolCall, text: nil, toolName: name, toolState: "completed",
                toolInput: name == "write" ? "{\"path\":\"main.rs\"}" : "{\"command\":\"cargo test\"}",
                toolOutput: "done", toolError: nil, toolCallId: "call-\(index)",
                toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)
        }
        let message = ChatMessage(id: "assistant", role: "assistant", parts: parts, assistantMeta: nil, steered: false, cancelled: false)
        let controller = UIHostingController(rootView: MessageItemView(message: message, fontChoice: .system, disclosures: .constant(ToolDisclosures()))
            .environment(\.dynamicTypeSize, .large))
        XCTAssertLessThanOrEqual(controller.sizeThatFits(in: CGSize(width: 393, height: 1_000)).height, 50)
    }

    func testInvisibleUserTurnsStillSeparateToolActivity() {
        let part = MessagePart(id: "one", kind: .toolCall, text: nil, toolName: "bash", toolState: "completed",
            toolInput: "{}", toolOutput: nil, toolError: nil, toolCallId: "one",
            toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)
        var second = part
        second.id = "two"
        store.messages = [ChatMessage(id: "first", role: "assistant", parts: [part], assistantMeta: nil, steered: false, cancelled: false),
            ChatMessage(id: "attachment-only", role: "user", parts: [], assistantMeta: nil, steered: false, cancelled: false),
            ChatMessage(id: "second", role: "assistant", parts: [second], assistantMeta: nil, steered: false, cancelled: false)]
        let items = store.conversationItems
        XCTAssertEqual(items.count, 2, "Hidden user messages must still separate turns")
        XCTAssertEqual(items.flatMap { $0.message.parts }.map(\.id), ["one", "two"])
    }

    func testExpandedActivityAndToolDetailsSurviveOlderPageMerge() {
        let parts = (0..<3).map { index in
            MessagePart(id: "tool-\(index)", kind: .toolCall, text: nil, toolName: "bash", toolState: "completed",
                toolInput: "{\"command\":\"cargo test\"}", toolOutput: "tests passed", toolError: nil,
                toolCallId: "call-\(index)", toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)
        }
        var disclosures = ToolDisclosures()
        disclosures.setGroupExpanded(true, parts: Array(parts.suffix(2)))
        disclosures.expandedCalls.insert("tool-2")
        XCTAssertTrue(disclosures.isGroupExpanded(parts))
        let message = ChatMessage(id: "older-group", role: "assistant", parts: parts, assistantMeta: nil, steered: false, cancelled: false)
        let controller = UIHostingController(rootView: MessageItemView(message: message, fontChoice: .system,
            disclosures: .constant(disclosures)).environment(\.dynamicTypeSize, .large))
        XCTAssertGreaterThan(controller.sizeThatFits(in: CGSize(width: 393, height: 2_000)).height, 250,
            "The merged group must retain both its call list and the previously opened output")
        disclosures.setGroupExpanded(false, parts: parts)
        XCTAssertFalse(disclosures.isGroupExpanded(parts))
        disclosures.setGroupExpanded(true, parts: parts)
        XCTAssertTrue(disclosures.expandedCalls.contains("tool-2"))
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
                toolCallId: nil, toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)],
                assistantMeta: nil, steered: false, cancelled: false)
        }
        return DaemonSnapshot(sessions: [SessionInfo(sessionId: "sess-1", title: "History", messages: messages,
            running: running, usageCost: 0, pendingPermissions: 0, pendingQuestions: 0,
            inputTokens: 0, outputTokens: 0, turns: 0, todos: [], historyBefore: before, subagents: [], jobs: [])],
            attachedSession: "sess-1", pendingPermissions: [], pendingAskUser: [], models: [],
            daemonVersion: nil, permissionMode: nil, currentModel: nil, currentProvider: nil,
            thinkingVariant: nil, currentPersona: nil, availablePersonas: [], automationSettings: nil)
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
                toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)
        }
        snapshot.sessions[0].messages.append(ChatMessage(id: "assistant", role: "assistant", parts: parts, assistantMeta: nil, steered: false, cancelled: false))
        mock.snapshotResult = snapshot

        store.handleEvent(.historyExtended(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertEqual(store.conversationItems.flatMap { $0.message.parts }.compactMap(\.text), ["older", "latest", "earlier answer"])
        XCTAssertEqual(store.streamingText, "current")
        store.handleEvent(.textDelta(daemon: "daemon-1", sessionId: "sess-1", partId: "active", delta: " reply"))
        XCTAssertEqual(store.streamingText, "current reply")

        snapshot.sessions[0].messages[2].parts[1].text = "current reply"
        mock.snapshotResult = snapshot
        store.handleEvent(.partUpdated(daemon: "daemon-1", sessionId: "sess-1", partId: "active", partKind: "text", state: "completed"))
        XCTAssertEqual(store.conversationItems.flatMap { $0.message.parts }.compactMap(\.text), ["older", "latest", "earlier answer", "current reply"])
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
            currentPersona: nil, availablePersonas: [], automationSettings: nil)
        store.daemonStatuses["daemon-1"] = .connected
        mock.promptAccepted = false
        store.lastPrompt["sess-1"] = "previous"
        XCTAssertFalse(store.sendPrompt("new text"))
        XCTAssertNil(store.feedback)
        XCTAssertEqual(store.lastPrompt["sess-1"], "previous")
        mock.promptAccepted = true
        XCTAssertTrue(store.sendPrompt("accepted"))
        XCTAssertEqual(store.feedback?.kind, .sent)
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
            currentPersona: nil, availablePersonas: [], automationSettings: nil)
        store.daemonStatuses["daemon-1"] = .connected
        XCTAssertTrue(store.sendPrompt("first"))
        XCTAssertFalse(store.isStreaming)
        XCTAssertFalse(store.canSendPrompt)
        XCTAssertFalse(store.sendPrompt("second"))
        XCTAssertEqual(mock.promptCalls.count, 1)
        store.handleEvent(.alert(daemon: "daemon-2", sessionId: "sess-1", kind: "error", title: "Other daemon", detail: nil))
        XCTAssertFalse(store.canSendPrompt)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-2", sessionId: "sess-1", running: true, cancelled: false))
        XCTAssertFalse(store.canSendPrompt)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true, cancelled: false))
        XCTAssertTrue(store.isStreaming)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: false, cancelled: false))
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
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true, cancelled: false))
        store.handleEvent(.providerMessageEnded(daemon: "daemon-1", sessionId: "sess-1",
            inputTokens: 7, outputTokens: 2, cost: 0.01, failed: false, manifest: nil))
        XCTAssertTrue(store.isStreaming)
        store.streamingText = "last provider response"
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: false, cancelled: false))
        XCTAssertFalse(store.isStreaming)
        XCTAssertTrue(store.streamingText.isEmpty)
    }

    func testOtherSessionsAndDaemonsCannotEndTheSelectedRun() {
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true, cancelled: false))
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-2", sessionId: "sess-1", running: false, cancelled: false))
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-2", running: false, cancelled: false))
        store.handleEvent(.providerMessageEnded(daemon: "daemon-2", sessionId: "sess-1",
            inputTokens: 7, outputTokens: 2, cost: 0.01, failed: true, manifest: nil))
        XCTAssertTrue(store.isStreaming)
        XCTAssertNotEqual(store.lastTurnFailed["sess-1"], true)
    }

    func testPromptWaitsForAuthoritativeActivityAndLateTextCannotRestartRun() {
        store.sendPrompt("hello")
        XCTAssertFalse(store.isStreaming)
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true, cancelled: false))
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: false, cancelled: false))
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

    func testLateSlashRepliesCannotEnterAnotherConversation() {
        store.handleEvent(.slashResult(daemon: "daemon-2", sessionId: "sess-1", text: "other machine"))
        store.handleEvent(.slashResult(daemon: "daemon-1", sessionId: "sess-2", text: "previous chat"))
        XCTAssertTrue(store.messages.isEmpty)
        store.handleEvent(.slashResult(daemon: "daemon-1", sessionId: "sess-1", text: "current reply"))
        XCTAssertEqual(store.messages.flatMap(\.parts).compactMap(\.text), ["current reply"])
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
            currentPersona: nil, availablePersonas: [], automationSettings: nil)
        store.handleEvent(.sessionReloaded(daemon: "daemon-2", sessionId: "sess-1"))
        XCTAssertFalse(store.isStreaming)
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertTrue(store.isStreaming)
    }

    func testPendingNewSessionCannotBeCompletedByAnOlderHistoryReload() {
        let destination: [NavigationRoute] = []
        store.path = destination
        store.daemonStatuses["daemon-1"] = .connected
        store.newSession(cwd: "/work/new", daemonNodeId: "daemon-1")

        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "old-session"))

        XCTAssertEqual(store.path, destination)
        XCTAssertNil(store.selectedSessionId)

        store.handleEvent(.sessionCreated(daemon: "daemon-2", sessionId: "other-machine"))
        XCTAssertEqual(store.path, destination)
        store.handleEvent(.sessionCreated(daemon: "daemon-1", sessionId: "new-session"))
        XCTAssertEqual(store.path.last, .chat(daemonNodeId: "daemon-1", sessionId: "new-session"))
        XCTAssertEqual(store.selectedSessionId, "new-session")
        store.handleEvent(.sessionCreated(daemon: "daemon-1", sessionId: "duplicate"))
        XCTAssertEqual(store.selectedSessionId, "new-session")
    }

    func testCreationResultsCannotHijackNavigationAwayFromTheRequest() {
        store.path = []
        store.daemonStatuses["daemon-1"] = .connected
        store.newSession(daemonNodeId: "daemon-1")
        let destination = [NavigationRoute.chat(daemonNodeId: "daemon-1", sessionId: "chosen-chat")]
        store.path = destination

        store.handleEvent(.sessionCreated(daemon: "daemon-1", sessionId: "late-created-chat"))
        store.handleEvent(.sessionCreationFailed(daemon: "daemon-1", message: "late failure"))

        XCTAssertEqual(store.path, destination)
        XCTAssertEqual(store.selectedSessionId, "chosen-chat")
        XCTAssertTrue(store.alerts.isEmpty)
    }

    func testCreationFailureReleasesOnlyItsOwnReservationAndAllowsRetry() {
        let destination: [NavigationRoute] = []
        store.path = destination
        store.daemonStatuses["daemon-1"] = .connected
        store.newSession(cwd: "/missing", daemonNodeId: "daemon-1")
        store.handleEvent(.sessionCreationFailed(daemon: "daemon-2", message: "unrelated failure"))
        XCTAssertTrue(store.alerts.isEmpty)

        store.handleEvent(.sessionCreationFailed(daemon: "daemon-1", message: "Workspace is unavailable"))

        XCTAssertEqual(store.activeBanner?.daemon, "daemon-1")
        XCTAssertEqual(store.activeBanner?.detail, "Workspace is unavailable")
        XCTAssertEqual(store.activeBanner?.sessionId, "")
        store.handleEvent(.sessionCreated(daemon: "daemon-1", sessionId: "late-result"))
        XCTAssertEqual(store.path, destination)
        store.newSession(cwd: "/available", daemonNodeId: "daemon-1")
        store.handleEvent(.sessionCreated(daemon: "daemon-1", sessionId: "retry-result"))
        XCTAssertEqual(mock.newSessionCalls, ["/missing", "/available"])
        XCTAssertEqual(store.selectedSessionId, "retry-result")
    }

    func testDisconnectAbandonsOnlyTheOriginatingPendingCreation() {
        let destination: [NavigationRoute] = []
        store.path = destination
        store.daemonStatuses["daemon-1"] = .connected
        store.newSession(daemonNodeId: "daemon-1")
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-2", status: .disconnected))
        XCTAssertTrue(store.alerts.isEmpty)

        store.handleEvent(.daemonStatusChanged(daemon: "daemon-1", status: .backoff(attempt: 1, error: "Connection lost")))

        XCTAssertEqual(store.activeBanner?.daemon, "daemon-1")
        XCTAssertNotNil(store.activeBanner?.detail)
        store.handleEvent(.sessionCreated(daemon: "daemon-1", sessionId: "late-result"))
        XCTAssertEqual(store.path, destination)
    }

    func testAttachmentFailuresAndRetriesStayWithTheActiveConversation() {
        store.isLoadingSession = true
        store.drafts[.chat(daemonNodeId: "daemon-1", sessionId: "sess-1")] = "unsent question"
        for (daemon, session) in [("daemon-2", "sess-1"), ("daemon-1", "other")] {
            store.handleEvent(.sessionAttachmentFailed(daemon: daemon, sessionId: session, message: "unrelated failure"))
            XCTAssertTrue(store.isLoadingSession)
            XCTAssertNil(store.sessionLoadError)
        }

        store.handleEvent(.sessionAttachmentFailed(daemon: "daemon-1", sessionId: "sess-1", message: "Conversation is unavailable"))
        XCTAssertFalse(store.isLoadingSession)
        XCTAssertEqual(store.sessionLoadError, "Conversation is unavailable")
        store.retrySessionAttachment(daemonNodeId: "daemon-2", sessionId: "sess-1")
        store.retrySessionAttachment(daemonNodeId: "daemon-1", sessionId: "other")
        XCTAssertTrue(mock.attachCalls.isEmpty)

        store.retrySessionAttachment(daemonNodeId: "daemon-1", sessionId: "sess-1")

        XCTAssertTrue(store.isLoadingSession)
        XCTAssertNil(store.sessionLoadError)
        XCTAssertEqual(mock.attachCalls.count, 1)
        XCTAssertEqual(mock.attachCalls.first?.nodeId, "daemon-1")
        XCTAssertEqual(mock.attachCalls.first?.sessionId, "sess-1")
        XCTAssertEqual(store.drafts[.chat(daemonNodeId: "daemon-1", sessionId: "sess-1")], "unsent question")
    }

    func testAttachmentErrorsClearOnSuccessfulHistoryAndConversationChanges() {
        store.handleEvent(.sessionAttachmentFailed(daemon: "daemon-1", sessionId: "sess-1", message: "Try again"))
        XCTAssertNotNil(store.sessionLoadError)
        mock.snapshotResult = historySnapshot(before: nil, texts: ["restored history"])

        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))

        XCTAssertNil(store.sessionLoadError)
        XCTAssertFalse(store.isLoadingSession)
        XCTAssertEqual(store.messages.map(\.id), ["restored history"])
        store.handleEvent(.sessionAttachmentFailed(daemon: "daemon-1", sessionId: "sess-1", message: "Another failure"))
        store.selectSession("another-chat")
        XCTAssertNil(store.sessionLoadError)
        XCTAssertTrue(store.isLoadingSession)
    }

    func testSwitchingChatsClearsThePreviousTranscriptWhileAttachmentIsPending() {
        mock.snapshotResult = historySnapshot(before: 24, texts: ["previous chat"], running: true)
        store.selectSession("sess-1")
        store.streamingPartId = "old-part"
        store.streamingText = "old streaming text"
        store.selectSession("sess-2")
        XCTAssertEqual(store.selectedSessionId, "sess-2")
        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertFalse(store.isStreaming)
        XCTAssertNil(store.streamingPartId)
        XCTAssertTrue(store.streamingText.isEmpty)
        XCTAssertNil(store.historyBefore)
        XCTAssertFalse(store.canSendPrompt)
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertTrue(store.messages.isEmpty, "A delayed reload from the previous chat must not restore its content")
    }

    func testReturningFromACrossDaemonAlertRestoresTheRouteOwner() {
        store.daemons = [DaemonEntry(nodeId: "daemon-2", name: "Server", addedAt: 0, lastConnectedAt: nil, lastKnownVersion: nil, keepConnected: true, pairingToken: nil)]
        store.path = [.chat(daemonNodeId: "daemon-1", sessionId: "sess-1")]
        store.openAlertSession(AlertItem(daemon: "daemon-2", sessionId: "sess-2", kind: "question", title: "Input", detail: nil))
        store.path.removeLast()
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "daemon-1")
        XCTAssertEqual(store.selectedSessionId, "sess-1")
        XCTAssertEqual(mock.attachCalls.last?.nodeId, "daemon-1")
        XCTAssertEqual(mock.attachCalls.last?.sessionId, "sess-1")
    }

    func testPairingPreservesTheParsedInviteCredential() throws {
        let info = try MewMobileCore.parseDialInfo(payload: "mew001:{\"node_id\":\"invite-daemon\",\"name\":\"Homelab\",\"token\":\"invite-credential\"}")
        store.addDaemon(info: info, name: "My server")
        XCTAssertEqual(mock.addedDaemons.last?.nodeId, "invite-daemon")
        XCTAssertEqual(mock.addedDaemons.last?.name, "My server")
        XCTAssertEqual(mock.addedDaemons.last?.token, "invite-credential")
        XCTAssertEqual(mock.connectCalls, ["invite-daemon"])
    }

    func testLegacyPairingStillUsesTheTokenlessPath() throws {
        let info = try MewMobileCore.parseDialInfo(payload: "mew001:legacy-daemon")
        store.addDaemon(info: info, name: "Legacy")
        XCTAssertEqual(mock.addedDaemons.last?.nodeId, "legacy-daemon")
        XCTAssertNil(mock.addedDaemons.last?.token)
    }

    func testPairAgainRejectsAnInviteForAnotherMachineWithoutAddingIt() {
        let info = DialInfo(nodeId: "other-machine", name: nil, pairingToken: "credential")
        XCTAssertNotNil(store.addDaemon(info: info, name: "Saved machine", replacingDaemonNodeId: "saved-machine"))
        XCTAssertTrue(mock.addedDaemons.isEmpty)
        XCTAssertTrue(mock.connectCalls.isEmpty)
    }

    func testPairAgainReplacesTheLiveConnectionsPreviousCredential() {
        let daemon = DaemonEntry(nodeId: "saved-machine", name: "Server", addedAt: 1, lastConnectedAt: nil,
            lastKnownVersion: nil, keepConnected: true, pairingToken: "old-credential")
        store.daemons = [daemon]
        mock.daemonsResult = [daemon]
        store.path = [.chat(daemonNodeId: daemon.nodeId, sessionId: "sess-1")]
        mock.attachCalls = []
        let invite = DialInfo(nodeId: "saved-machine", name: nil, pairingToken: "new-credential")
        XCTAssertNil(store.addDaemon(info: invite, name: daemon.name, replacingDaemonNodeId: daemon.nodeId))
        XCTAssertEqual(mock.addedDaemons.last?.token, "new-credential")
        XCTAssertEqual(mock.disconnectCalls, ["saved-machine"])
        XCTAssertEqual(mock.connectCalls, ["saved-machine"])
        XCTAssertEqual(mock.attachCalls.last?.nodeId, "saved-machine")
        XCTAssertEqual(mock.attachCalls.last?.sessionId, "sess-1")
        mock.snapshotsByDaemon["saved-machine"] = historySnapshot(before: nil, texts: ["recovered"])
        store.handleEvent(.daemonStatusChanged(daemon: "saved-machine", status: .connected))
        store.handleEvent(.sessionReloaded(daemon: "saved-machine", sessionId: "sess-1"))
        XCTAssertEqual(store.messages.first?.parts.first?.text, "recovered")
        XCTAssertTrue(store.canSendPrompt)
    }

    func testOpeningDetailsCannotEraseAnAttachmentErrorWithCachedHistory() {
        store.path = [.chat(daemonNodeId: "daemon-1", sessionId: "sess-1")]
        mock.snapshotResult = historySnapshot(before: nil, texts: ["cached history"])
        store.daemonStatuses["daemon-1"] = .connected
        store.handleEvent(.sessionAttachmentFailed(daemon: "daemon-1", sessionId: "sess-1", message: "Resume failed"))
        store.path.append(.daemonDetails(daemonNodeId: "daemon-1"))
        store.path.removeLast()
        XCTAssertEqual(store.sessionLoadError, "Resume failed")
        XCTAssertFalse(store.canSendPrompt)
    }

    func testAutomationSettingsStayWithTheirDaemonAndWaitForConfirmation() {
        store.handleEvent(.automationSettings(daemon: "daemon-2", settings: AutomationSettings(autoTitleEnabled: true, autoSummaryEnabled: false)))
        XCTAssertNil(store.daemonSettings["daemon-1"])
        XCTAssertEqual(store.daemonSettings["daemon-2"]?.autoSummaryEnabled, false)
        store.setAutoSummary(daemonNodeId: "daemon-2", enabled: true)
        XCTAssertEqual(store.daemonSettings["daemon-2"]?.autoSummaryEnabled, false)
        XCTAssertEqual(mock.autoSummaryCalls.last?.nodeId, "daemon-2")
        store.handleEvent(.autoSummaryChanged(daemon: "daemon-2", enabled: true))
        XCTAssertEqual(store.daemonSettings["daemon-2"]?.autoSummaryEnabled, true)
        store.handleEvent(.daemonStatusChanged(daemon: "daemon-2", status: .disconnected))
        XCTAssertNil(store.daemonSettings["daemon-2"])
    }

    func testAutomationSettingsDoNotInventDefaultsOrReplaySessionPreferences() {
        store.readDaemonSettings(daemonNodeId: "daemon-2")
        XCTAssertNil(store.daemonSettings["daemon-2"])
        XCTAssertEqual(mock.settingsReadCalls, ["daemon-2"])
        mock.settingsAccepted = false
        store.readDaemonSettings(daemonNodeId: "daemon-2")
        XCTAssertNotNil(store.daemonSettingsErrors["daemon-2"])
        store.handleEvent(.sessionReloaded(daemon: "daemon-1", sessionId: "sess-1"))
        XCTAssertTrue(mock.autoTitleCalls.isEmpty)
        XCTAssertTrue(mock.autoSummaryCalls.isEmpty)
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
            currentPersona: nil, availablePersonas: [], automationSettings: nil
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
            kind: "permissionneeded", title: "Permission needed", detail: nil
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
        store.daemons = [DaemonEntry(nodeId: "daemon-2", name: "Server", addedAt: 0, lastConnectedAt: nil, lastKnownVersion: nil, keepConnected: true, pairingToken: nil)]
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
            kind: "permissionneeded", title: "Permission needed", detail: nil
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
            availablePersonas: [], automationSettings: nil
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

    func testQueuedMessagesTrackServerUpdatesAndForwardActions() {
        mock.snapshotResult = DaemonSnapshot(
            sessions: [], attachedSession: "sess-1", pendingPermissions: [],
            pendingAskUser: [], models: [], daemonVersion: nil, permissionMode: nil,
            currentModel: nil, currentProvider: nil, thinkingVariant: nil,
            currentPersona: nil, availablePersonas: [], automationSettings: nil
        )
        store.daemonStatuses["daemon-1"] = .connected

        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: true, cancelled: false))
        XCTAssertFalse(store.canSendPrompt)
        XCTAssertTrue(store.canEnqueuePrompt)

        XCTAssertTrue(store.enqueueMessage("follow up", steer: false))
        XCTAssertTrue(store.enqueueMessage("redirect now", steer: true))
        XCTAssertEqual(mock.enqueueCalls.count, 2)
        XCTAssertEqual(mock.enqueueCalls[0].text, "follow up")
        XCTAssertFalse(mock.enqueueCalls[0].steer)
        XCTAssertEqual(mock.enqueueCalls[1].text, "redirect now")
        XCTAssertTrue(mock.enqueueCalls[1].steer)

        let item = MobileQueuedMessage(id: "q-1", text: "follow up", steer: false)
        store.handleEvent(.queueChanged(daemon: "daemon-1", sessionId: "sess-1", items: [item]))
        store.handleEvent(.queueChanged(daemon: "daemon-2", sessionId: "sess-1", items: [
            MobileQueuedMessage(id: "q-other", text: "wrong daemon", steer: true)
        ]))
        XCTAssertEqual(store.queuedMessages["sess-1"], [item])

        XCTAssertTrue(store.promoteQueuedMessage(id: "q-1"))
        XCTAssertTrue(store.removeQueuedMessage(id: "q-1"))
        XCTAssertEqual(mock.promoteQueueCalls.count, 1)
        XCTAssertEqual(mock.promoteQueueCalls[0].messageId, "q-1")
        XCTAssertEqual(mock.removeQueueCalls.count, 1)
        XCTAssertEqual(mock.removeQueueCalls[0].messageId, "q-1")

        store.handleEvent(.queueChanged(daemon: "daemon-1", sessionId: "sess-1", items: []))
        XCTAssertEqual(store.queuedMessages["sess-1"], [])
    }

    func testSessionCompactingDeduplicatesSlashResultAndTracksActiveState() {
        store.handleEvent(.sessionCompacting(daemon: "daemon-2", sessionId: "sess-1", active: true, removed: nil, summary: nil))
        XCTAssertFalse(store.isCompacting)

        store.handleEvent(.sessionCompacting(daemon: "daemon-1", sessionId: "sess-1", active: true, removed: nil, summary: nil))
        XCTAssertTrue(store.isCompacting)

        store.handleEvent(.sessionCompacting(daemon: "daemon-1", sessionId: "sess-1", active: false, removed: 12, summary: "## Goal & Constraints\nLive recap"))
        XCTAssertFalse(store.isCompacting)
        XCTAssertEqual(store.messages.count, 1)
        XCTAssertEqual(store.messages[0].role, "system")
        XCTAssertEqual(store.messages[0].parts[0].kind, .compaction)
        XCTAssertEqual(store.messages[0].parts[0].compactionRemoved, 12)
        XCTAssertEqual(store.messages[0].parts[0].text, "## Goal & Constraints\nLive recap")

        store.handleEvent(.slashResult(daemon: "daemon-1", sessionId: "sess-1", text: "compacted 12 messages"))
        XCTAssertEqual(store.messages.count, 1)

        // Reverse order: slashResult first, then sessionCompacting replaces the text bubble.
        store.messages = []
        store.handleEvent(.slashResult(daemon: "daemon-1", sessionId: "sess-1", text: "compacted 5 messages"))
        XCTAssertEqual(store.messages.count, 1)
        store.handleEvent(.sessionCompacting(daemon: "daemon-1", sessionId: "sess-1", active: false, removed: 5, summary: nil))
        XCTAssertEqual(store.messages.count, 1)
        XCTAssertEqual(store.messages[0].role, "system")
        XCTAssertEqual(store.messages[0].parts[0].kind, .compaction)
        XCTAssertEqual(store.messages[0].parts[0].compactionRemoved, 5)
    }

    func testRetryWaitAndCancelledActivityState() {
        store.handleEvent(.retryWait(daemon: "daemon-1", sessionId: "sess-1", attempt: 1, maxAttempts: 3, delaySecs: 4, reason: "overloaded"))
        XCTAssertEqual(store.retryWait, RetryWaitInfo(attempt: 1, maxAttempts: 3, delaySecs: 4, reason: "overloaded"))

        store.handleEvent(.textDelta(daemon: "daemon-1", sessionId: "sess-1", partId: "p-1", delta: "hi"))
        XCTAssertNil(store.retryWait)

        store.handleEvent(.retryWait(daemon: "daemon-1", sessionId: "sess-1", attempt: 2, maxAttempts: 3, delaySecs: 8, reason: "timeout"))
        XCTAssertEqual(store.retryWait?.attempt, 2)

        store.messages = [
            ChatMessage(
                id: "u-1", role: "user",
                parts: [MessagePart(id: "up-1", kind: .text, text: "hello", toolName: nil, toolState: nil,
                    toolInput: nil, toolOutput: nil, toolError: nil, toolCallId: nil,
                    toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)],
                assistantMeta: nil, steered: false, cancelled: false
            )
        ]
        store.handleEvent(.sessionActivityChanged(daemon: "daemon-1", sessionId: "sess-1", running: false, cancelled: true))
        XCTAssertNil(store.retryWait)
        XCTAssertEqual(store.messages.count, 2)
        XCTAssertEqual(store.messages[1].role, "assistant")
        XCTAssertTrue(store.messages[1].cancelled)
    }

    func testSubagentIdenticonDeterminismAndHorizontalSymmetry() {
        for seed in ["child-session-1", "researcher", "coder-alpha", ""] {
            let cellsA = identiconCells(seed: seed)
            let cellsB = identiconCells(seed: seed)
            XCTAssertEqual(cellsA, cellsB)
            XCTAssertEqual(identiconColorHex(seed: seed), identiconColorHex(seed: seed))
            XCTAssertTrue(subagentIdenticonPaletteHex.contains(identiconColorHex(seed: seed)))
            XCTAssertEqual(cellsA.count, 5)
            var anyFilled = false
            for row in cellsA {
                XCTAssertEqual(row.count, 5)
                for col in 0..<5 {
                    XCTAssertEqual(row[col], row[4 - col])
                    if row[col] { anyFilled = true }
                }
            }
            XCTAssertTrue(anyFilled)
        }
        XCTAssertNotEqual(hashSeedFnv1a("agent-a"), hashSeedFnv1a("agent-b"))
    }
}
