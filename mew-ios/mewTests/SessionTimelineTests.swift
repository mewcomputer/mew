import XCTest
import MewMobileCore
@testable import mew

@MainActor
final class SessionTimelineTests: XCTestCase {
    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "America/Chicago")!
        return calendar
    }

    private func date(_ year: Int = 2026, _ month: Int = 10, _ day: Int = 22, _ hour: Int = 12) -> Date {
        calendar.date(from: DateComponents(year: year, month: month, day: day, hour: hour))!
    }

    private func session(_ id: String, at date: Date, state: String = "idle", pending: UInt32 = 0,
                         archived: Bool = false, pinned: Bool = false) -> SessionSummary {
        let timestamp = Int64(date.timeIntervalSince1970 * 1_000)
        return SessionSummary(sessionId: id, title: id, state: state, archived: archived, pinned: pinned,
            pendingPermissions: pending, pendingQuestions: 0, usageCost: 0, cwd: nil, scratch: false, model: nil, provider: nil,
            createdAt: timestamp - 1_000, lastMessageAt: timestamp, lastTurnFailed: false, groupId: nil,
            inputTokens: 0, outputTokens: 0, turns: 0, contextTokens: nil)
    }

    func testAllMachinesMergeByTimeWithScopedIdentityAndCachedOfflineChats() {
        let store = AppStore()
        store.daemons = ["a", "b"].map { id in
            DaemonEntry(nodeId: id, name: id, addedAt: 0, lastConnectedAt: nil,
                lastKnownVersion: nil, keepConnected: true, pairingToken: nil)
        }
        store.daemonStatuses = ["a": .connected, "b": .disconnected]
        store.sessionLists = ["a": [session("same", at: date(2026, 10, 21)), session("archived", at: date(), archived: true)],
                              "b": [session("same", at: date())],
                              "removed": [session("stale", at: date(2026, 10, 23))]]
        let items = store.sessionTimeline(daemonNodeId: nil, includeArchived: false, now: date(), calendar: calendar).flatMap(\.items)
        XCTAssertEqual(items.map(\.daemonNodeId), ["b", "a"])
        XCTAssertEqual(Set(items.map(\.id)).count, 2)
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: nil, includeArchived: true, now: date(), calendar: calendar)
            .flatMap(\.items).count, 3)
        store.handleEvent(.alert(daemon: "a", sessionId: "same", kind: "turncomplete", title: "Done", detail: nil), receivedAt: date(2026, 10, 23))
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: nil, includeArchived: false, now: date(), calendar: calendar)
            .first?.items.first?.daemonNodeId, "a")
    }

    func testRefreshingAllMachinesKeepsTheOpenChatAndSkipsOfflineMachines() {
        let store = AppStore()
        let core = MockMobileCore()
        store.injectCore(core)
        store.daemons = ["a", "b", "offline"].map { id in
            DaemonEntry(nodeId: id, name: id, addedAt: 0, lastConnectedAt: nil,
                lastKnownVersion: nil, keepConnected: true, pairingToken: nil)
        }
        store.daemonStatuses = ["a": .connected, "b": .connected, "offline": .disconnected]
        store.selectSession("current", daemonNodeId: "a")
        store.refreshSessions(daemonNodeId: nil)
        XCTAssertEqual(core.listSessionsCalls, ["a", "b"])
        XCTAssertEqual(store.selectedSessionId, "current")
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "a")
    }

    func testRecentActivityOrdersChatsInsteadOfAttentionOrPinState() {
        let store = AppStore()
        store.sessionLists = ["a": [session("old-pending", at: date(2026, 10, 20), pending: 1, pinned: true),
                                   session("new", at: date()),
                                   session("middle-running", at: date(2026, 10, 21), state: "running")]]
        let sections = store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: date(), calendar: calendar)
        XCTAssertEqual(sections.flatMap(\.items).map { $0.session.sessionId }, ["new", "middle-running", "old-pending"])
        XCTAssertEqual(sections.map(\.period), [.today, .yesterday, .previous7Days])
    }

    func testCreationFallbackTiesAndDaemonScopedIdentityAreStable() {
        let store = AppStore()
        var created = session("created", at: date())
        created.lastMessageAt = nil
        created.createdAt = Int64(date().timeIntervalSince1970 * 1_000)
        let same = session("same", at: date())
        store.sessionLists = ["b": [same], "a": [same, created]]
        let first = store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: date(), calendar: calendar).flatMap(\.items)
        let second = store.sessionTimeline(daemonNodeId: "b", includeArchived: false, now: date(), calendar: calendar).flatMap(\.items)
        let items = SessionTimelineSection.grouped(second + first, now: date(), calendar: calendar).flatMap(\.items)
        XCTAssertEqual(items.map(\.daemonNodeId), ["a", "a", "b"])
        XCTAssertEqual(items.map { $0.session.sessionId }, ["created", "same", "same"])
        XCTAssertEqual(Set(items.map(\.id)).count, 3)
    }

    func testCalendarGroupsHandleBoundariesFutureDatesAndUnknownTimes() {
        let store = AppStore()
        let now = date()
        let offsets = [0, 1, 2, 7, 8, 30, 31]
        var summaries = offsets.map { days in
            session("day-\(days)", at: calendar.date(byAdding: .day, value: -days, to: now)!)
        }
        summaries.append(session("future", at: date(2026, 10, 23)))
        var unknown = session("unknown", at: now)
        unknown.createdAt = 0
        unknown.lastMessageAt = nil
        summaries.append(unknown)
        store.sessionLists = ["a": summaries.reversed()]
        let sections = store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: now, calendar: calendar)
        XCTAssertEqual(sections.map(\.period), [.today, .yesterday, .previous7Days, .previous30Days, .older])
        XCTAssertEqual(sections.map { $0.items.map { $0.session.sessionId } },
            [["future", "day-0"], ["day-1"], ["day-2", "day-7"], ["day-8", "day-30"], ["day-31", "unknown"]])
    }

    func testYesterdayUsesCalendarDaysAcrossDaylightSaving() {
        let store = AppStore()
        let now = date(2026, 11, 2, 0)
        store.sessionLists = ["a": [session("yesterday", at: date(2026, 11, 1, 0))]]
        let sections = store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: now, calendar: calendar)
        XCTAssertEqual(sections.map(\.period), [.yesterday])
    }

    func testDaemonAndArchiveFiltersPreserveTheChronologicalOrder() {
        let store = AppStore()
        store.sessionLists = ["a": [session("archived", at: date(), archived: true), session("visible", at: date(2026, 10, 21))],
                              "b": [session("other", at: date())]]
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: date(), calendar: calendar)
            .flatMap(\.items).map { $0.session.sessionId }, ["visible"])
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: "a", includeArchived: true, now: date(), calendar: calendar)
            .flatMap(\.items).map { $0.session.sessionId }, ["archived", "visible"])
        XCTAssertTrue(store.sessionTimeline(daemonNodeId: "missing", includeArchived: true, now: date(), calendar: calendar).isEmpty)
    }

    func testNewAlertPromotesOnlyItsChatAndSurvivesAListRefresh() {
        let store = AppStore()
        let old = session("same", at: date(2020, 1, 1), pending: 1)
        let recent = session("recent", at: date(2021, 1, 1))
        store.sessionLists = ["a": [recent, old], "b": [old]]
        store.handleEvent(.alert(daemon: "a", sessionId: "same", kind: "inputneeded", title: "Question", detail: nil), receivedAt: date())
        store.handleEvent(.sessionList(daemon: "a", sessions: [recent, old]))
        let items = store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: date(), calendar: calendar).flatMap(\.items)
        XCTAssertEqual(items.first?.daemonNodeId, "a")
        XCTAssertEqual(items.first?.session.sessionId, "same")
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: "b", includeArchived: false, now: date(), calendar: calendar)
            .first?.items.first?.activityAt, old.lastMessageAt)
        XCTAssertEqual(store.sessionLists["a"]?.last?.lastMessageAt, old.lastMessageAt)
        let before = items.map(\.activityAt)
        store.handleEvent(.attentionChanged(daemon: "a", sessionId: "same", pendingPermissions: 3, pendingQuestions: 0))
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: date(), calendar: calendar)
            .flatMap(\.items).map(\.activityAt), before)
    }

    func testAlertArrivingBeforeTheSessionListStillUpdatesTheRightChat() {
        let store = AppStore()
        let core = MockMobileCore()
        store.injectCore(core)
        store.handleEvent(.alert(daemon: "a", sessionId: "late", kind: "turncomplete", title: "Done", detail: nil), receivedAt: date())
        XCTAssertEqual(core.listSessionsCalls, ["a"], "An unknown chat must fetch its metadata before it can appear")
        store.handleEvent(.sessionList(daemon: "a", sessions: [session("recent", at: date(2021, 1, 1)),
                                                               session("late", at: date(2020, 1, 1))]))
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: date(), calendar: calendar)
            .first?.items.first?.session.sessionId, "late")
    }

    func testOlderAlertDeliveryCannotMoveAChatBackwards() {
        let store = AppStore()
        store.sessionLists = ["a": [session("chat", at: date(2020, 1, 1))]]
        let alert = CoreEvent.alert(daemon: "a", sessionId: "chat", kind: "inputneeded", title: "Input", detail: nil)
        store.handleEvent(alert, receivedAt: date())
        store.handleEvent(alert, receivedAt: date(2026, 10, 21))
        XCTAssertEqual(store.sessionTimeline(daemonNodeId: "a", includeArchived: false, now: date(), calendar: calendar)
            .first?.items.first?.activityAt, Int64(date().timeIntervalSince1970 * 1_000))
    }

    func testRefreshingOneDaemonDoesNotChangeTheActiveChat() {
        let store = AppStore()
        let core = MockMobileCore()
        store.injectCore(core)
        store.selectedDaemonId = DaemonId(nodeId: "active")
        store.selectedSessionId = "current-chat"
        store.refreshSessions(daemonNodeId: "other")
        XCTAssertEqual(core.listSessionsCalls, ["other"])
        XCTAssertEqual(store.selectedDaemonId?.nodeId, "active")
        XCTAssertEqual(store.selectedSessionId, "current-chat")
    }

}
