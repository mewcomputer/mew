import XCTest
import SwiftUI
import MewMobileCore
@testable import mew

@MainActor
final class PhoneSettingsTests: XCTestCase {
    private var suite: String!
    private var defaults: UserDefaults!
    private var store: AppStore!

    override func setUp() {
        super.setUp()
        suite = "mew.phone-settings-tests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)!
        store = AppStore(defaults: defaults)
        store.isAppActive = true
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        store = nil
        defaults = nil
        super.tearDown()
    }

    func testDefaultsFollowTheSystemAndPreserveExistingReviewPresentation() {
        XCTAssertEqual(store.phoneSettings.appearance, .system)
        XCTAssertNil(store.phoneSettings.appearance.colorScheme)
        XCTAssertEqual(store.phoneSettings.lightThemeId, MewThemes.defaultLightId)
        XCTAssertEqual(store.phoneSettings.darkThemeId, MewThemes.defaultDarkId)
        XCTAssertTrue(store.phoneSettings.haptics)
        XCTAssertTrue(store.phoneSettings.completionAlerts)
        XCTAssertTrue(store.phoneSettings.errorAlerts)
        XCTAssertTrue(store.phoneSettings.requestAlerts)
        XCTAssertTrue(store.phoneSettings.reviewLineNumbers)
        XCTAssertFalse(store.phoneSettings.reviewWrapLines)
        XCTAssertEqual(store.phoneSettings.reviewTextSize, .standard)
    }

    func testPreferencesAndMachineMutesSurviveReopeningWithoutChangingNavigationOrFont() {
        store.fontChoice = .serif
        defaults.set("remote", forKey: "mew.lastDaemon")
        store.phoneSettings.appearance = .dark
        store.phoneSettings.lightThemeId = "jianghu-light"
        store.phoneSettings.darkThemeId = "jianghu-dark"
        store.phoneSettings.haptics = false
        store.phoneSettings.completionAlerts = false
        store.phoneSettings.errorAlerts = false
        store.phoneSettings.requestAlerts = false
        store.phoneSettings.reviewLineNumbers = false
        store.phoneSettings.reviewWrapLines = true
        store.phoneSettings.reviewTextSize = .large
        store.phoneSettings.mutedMachines.insert("remote")

        let reopened = AppStore(defaults: defaults)
        XCTAssertEqual(reopened.phoneSettings, store.phoneSettings)
        XCTAssertEqual(reopened.phoneSettings.appearance.colorScheme, .dark)
        XCTAssertEqual(reopened.fontChoice, .serif)
        XCTAssertEqual(defaults.string(forKey: "mew.lastDaemon"), "remote")
        XCTAssertEqual(AppStore(defaults: UserDefaults(suiteName: "\(suite!)-other")!).phoneSettings, PhoneSettings())
    }

    func testInvalidEnumsFallBackWithoutLosingOtherPreferences() {
        defaults.set("sepia", forKey: "mew.appearance")
        defaults.set("not-a-theme", forKey: "mew.lightThemeId")
        defaults.set("not-a-theme", forKey: "mew.darkThemeId")
        defaults.set("enormous", forKey: "mew.reviewTextSize")
        defaults.set("Mi Sans", forKey: "mew.fontChoice")
        defaults.set(false, forKey: "mew.haptics")
        let reopened = AppStore(defaults: defaults)
        XCTAssertEqual(reopened.phoneSettings.appearance, .system)
        XCTAssertEqual(reopened.phoneSettings.lightThemeId, MewThemes.defaultLightId)
        XCTAssertEqual(reopened.phoneSettings.darkThemeId, MewThemes.defaultDarkId)
        XCTAssertEqual(reopened.phoneSettings.reviewTextSize, .standard)
        XCTAssertEqual(reopened.fontChoice, .instrumentSans)
        XCTAssertFalse(reopened.phoneSettings.haptics)
    }

    func testDisabledAlertsStillRecordActivity() {
        store.phoneSettings.completionAlerts = false
        alert("turncomplete")
        XCTAssertNil(store.activeBanner)
        XCTAssertNil(store.feedback)
        XCTAssertEqual(store.alerts.count, 1)
        XCTAssertEqual(store.alerts.first?.sessionId, "chat")
    }

    func testEachWireAlertKindUsesItsOwnPreference() {
        store.phoneSettings.completionAlerts = false
        store.phoneSettings.requestAlerts = false
        for kind in ["turncomplete", "permissionneeded", "inputneeded"] {
            alert(kind)
            XCTAssertNil(store.activeBanner, kind)
        }
        for kind in ["turnfailed", "error"] {
            alert(kind)
            XCTAssertEqual(store.activeBanner?.kind, kind)
        }
        store.activeBanner = nil
        store.phoneSettings.errorAlerts = false
        alert("turnfailed")
        XCTAssertNil(store.activeBanner)
        store.phoneSettings.requestAlerts = true
        alert("permissionneeded")
        XCTAssertEqual(store.activeBanner?.kind, "permissionneeded")
        alert("inputneeded")
        XCTAssertEqual(store.activeBanner?.kind, "inputneeded")
    }

    func testMachineMuteAffectsPresentationOnlyAndIsScopedToItsOrigin() {
        store.phoneSettings.mutedMachines.insert("a")
        alert("error", daemon: "a")
        XCTAssertNil(store.activeBanner)
        XCTAssertNil(store.feedback)
        alert("error", daemon: "b")
        XCTAssertEqual(store.activeBanner?.daemon, "b")
        XCTAssertEqual(store.alerts.count, 2)
    }

    func testBackgroundEventsNeverPresentOrReplayForegroundFeedback() {
        store.isAppActive = false
        alert("error")
        XCTAssertNil(store.activeBanner)
        XCTAssertNil(store.feedback)
        XCTAssertEqual(store.alerts.count, 1)
        store.isAppActive = true
        XCTAssertNil(store.activeBanner)
        XCTAssertNil(store.feedback)
        alert("inputneeded")
        XCTAssertNotNil(store.feedback)
        store.isAppActive = false
        XCTAssertNil(store.activeBanner)
        XCTAssertNil(store.feedback)
    }

    func testHapticsCanBeDisabledWithoutDisablingBanners() {
        store.phoneSettings.haptics = false
        alert("turncomplete")
        XCTAssertNotNil(store.activeBanner)
        XCTAssertNil(store.feedback)
    }

    func testTurningOffAnAlertCategoryDismissesItsCurrentBanner() {
        alert("turncomplete")
        store.phoneSettings.completionAlerts = false
        XCTAssertNil(store.activeBanner)
        XCTAssertEqual(store.alerts.count, 1)
    }

    func testDisabledDeliveryAlertsStillReleasePendingSubmissionAndRetainFailure() {
        let core = MockMobileCore()
        core.snapshotResult = DaemonSnapshot(sessions: [], attachedSession: "chat", pendingPermissions: [], pendingAskUser: [],
            models: [], daemonVersion: nil, permissionMode: nil, currentModel: nil, currentProvider: nil,
            thinkingVariant: nil, currentPersona: nil, availablePersonas: [], automationSettings: nil)
        store.injectCore(core)
        store.selectedDaemonId = DaemonId(nodeId: "remote")
        store.selectedSessionId = "chat"
        store.daemonStatuses["remote"] = .connected
        store.phoneSettings.errorAlerts = false
        XCTAssertTrue(store.sendPrompt("hello"))
        XCTAssertFalse(store.canSendPrompt)
        alert("error")
        XCTAssertTrue(store.canSendPrompt)
        XCTAssertEqual(store.lastTurnFailed["chat"], true)
        XCTAssertNil(store.activeBanner)
    }

    func testCurrentConversationCompletesWithFeedbackWithoutCoveringItWithABanner() {
        store.selectedDaemonId = DaemonId(nodeId: "remote")
        store.selectedSessionId = "chat"
        alert("turncomplete")
        XCTAssertNil(store.activeBanner)
        XCTAssertEqual(store.feedback?.kind, .completed)
    }

    func testRetryingASentMessageProducesFeedbackOnlyAfterItIsAccepted() {
        let core = MockMobileCore()
        core.snapshotResult = DaemonSnapshot(sessions: [], attachedSession: "chat", pendingPermissions: [], pendingAskUser: [],
            models: [], daemonVersion: nil, permissionMode: nil, currentModel: nil, currentProvider: nil,
            thinkingVariant: nil, currentPersona: nil, availablePersonas: [], automationSettings: nil)
        store.injectCore(core)
        store.selectedDaemonId = DaemonId(nodeId: "remote")
        store.selectedSessionId = "chat"
        store.daemonStatuses["remote"] = .connected
        store.lastPrompt["chat"] = "try again"
        store.lastTurnFailed["chat"] = true
        core.promptAccepted = false
        store.retryLastTurn()
        XCTAssertNil(store.feedback)
        XCTAssertEqual(store.lastTurnFailed["chat"], true)
        core.promptAccepted = true
        store.retryLastTurn()
        XCTAssertEqual(store.feedback?.kind, .sent)
        XCTAssertEqual(store.lastTurnFailed["chat"], false)
    }

    func testMutingAlertsDoesNotDiscardPermissionRequestsOrQuestions() {
        store.phoneSettings.mutedMachines.insert("remote")
        store.handleEvent(.permissionRequested(daemon: "remote", sessionId: "chat", requestId: "permission",
            toolName: "bash", input: "{}"))
        store.handleEvent(.askUserRequested(subagent: nil, daemon: "remote", sessionId: "chat", requestId: "question",
            callId: "call", questions: []))
        alert("permissionneeded")
        alert("inputneeded")
        XCTAssertNil(store.activeBanner)
        XCTAssertEqual(store.pendingPermissions.map(\.requestId), ["permission"])
        XCTAssertEqual(store.pendingAskUser.map(\.requestId), ["question"])
        XCTAssertEqual(store.alerts.count, 2)
    }

    func testWrappedReviewLinesUseAvailableWidthAtAccessibilitySizes() {
        let line = ReviewLine(kind: .addition, oldLine: nil, newLine: 100,
            text: "let message = \"This long line should remain readable without horizontal scrolling on a phone.\"")
        let wrapped = reviewLineSize(line, wrap: true)
        let unwrapped = reviewLineSize(line, wrap: false)
        XCTAssertEqual(wrapped.width, 320, accuracy: 0.5, "Allow sub-point pixel rounding by UIKit")
        XCTAssertGreaterThan(wrapped.height, unwrapped.height)
    }

    private func reviewLineSize(_ line: ReviewLine, wrap: Bool) -> CGSize {
        let view = ReviewLineView(line: line, wrapLines: wrap, showLineNumbers: true, fontSize: 26, gutterWidth: 52)
        return UIHostingController(rootView: view).sizeThatFits(in: CGSize(width: 320, height: 2_000))
    }

    func testDiagnosticExportUsesAnAllowlistEvenWhenErrorsAndNamesContainSecrets() {
        let secret = "secret-token-/Users/private/prompt"
        store.daemons = [DaemonEntry(nodeId: secret, name: secret, addedAt: 0, lastConnectedAt: nil,
            lastKnownVersion: "1.2.3", keepConnected: true, pairingToken: secret)]
        store.daemonStatuses[secret] = .backoff(attempt: 4, error: secret)
        store.daemonVersions[secret] = secret
        store.drafts[.chat(daemonNodeId: secret, sessionId: secret)] = secret
        let report = store.connectionDiagnostics
        XCTAssertFalse(report.contains(secret))
        XCTAssertFalse(report.contains("/Users"))
        XCTAssertTrue(report.contains("retrying"))
        XCTAssertTrue(report.contains("unknown"))
        XCTAssertTrue(report.contains("Machine 1"))
    }

    private func alert(_ kind: String, daemon: String = "remote") {
        store.handleEvent(.alert(daemon: daemon, sessionId: "chat", kind: kind, title: "Update", detail: nil))
    }

}
