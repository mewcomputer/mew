import XCTest
@testable import mew

final class HistoryAutoLoadTests: XCTestCase {
    func testPullingUpAtTheTopRequestsOnePage() {
        var autoLoad = HistoryAutoLoad()
        XCTAssertTrue(autoLoad.scrollsIntoBackfill(from: 400, to: 300))
    }

    func testOnePullLoadsOnePage() {
        var autoLoad = HistoryAutoLoad()
        XCTAssertTrue(autoLoad.scrollsIntoBackfill(from: 400, to: 300))
        // A flick keeps emitting geometry changes at the top; they must not
        // page the whole session.
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 300, to: 200))
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 200, to: 10))
    }

    func testLeavingTheTopRearmsTheNextPull() {
        var autoLoad = HistoryAutoLoad()
        XCTAssertTrue(autoLoad.scrollsIntoBackfill(from: 400, to: 300))
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 300, to: 200))
        // Scrolling back down out of the trigger zone arms another page.
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 200, to: 600))
        XCTAssertTrue(autoLoad.scrollsIntoBackfill(from: 600, to: 120))
    }

    func testDownwardScrollingNeverLoads() {
        var autoLoad = HistoryAutoLoad()
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 100, to: 500))
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 10, to: 11))
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 10, to: 10))
    }

    func testStayingInsideTheZoneWithoutMovingUpDoesNothing() {
        var autoLoad = HistoryAutoLoad()
        XCTAssertFalse(autoLoad.scrollsIntoBackfill(from: 100, to: 100))
        XCTAssertTrue(autoLoad.armed)
    }

    func testRubberBandingAboveTheTopCountsAsPullingUp() {
        // An over-scrolled transcript reports a negative distance from the top.
        var autoLoad = HistoryAutoLoad()
        XCTAssertTrue(autoLoad.scrollsIntoBackfill(from: 0, to: -60))
    }

    func testRefusedRequestRearmsSoBackfillSurvivesADisconnect() {
        var autoLoad = HistoryAutoLoad()
        XCTAssertTrue(autoLoad.scrollsIntoBackfill(from: 400, to: 300))
        // The store refused (offline, or already loading); the next pull
        // must still be able to ask.
        autoLoad.rearm()
        XCTAssertTrue(autoLoad.scrollsIntoBackfill(from: 300, to: 200))
    }
}
