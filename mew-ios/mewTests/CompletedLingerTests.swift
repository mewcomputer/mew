import XCTest
@testable import mew

final class CompletedLingerTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000)
    private var afterGrace: Date { start.addingTimeInterval(CompletedLinger.grace + 1) }

    func testFinishedWorkStaysForTheGraceWindow() {
        var linger = CompletedLinger()
        linger.observe(finished: ["job-1"], now: start)
        XCTAssertTrue(linger.isVisible("job-1", now: start))
        XCTAssertTrue(linger.isVisible("job-1", now: start.addingTimeInterval(CompletedLinger.grace - 1)))
    }

    func testFinishedWorkLeavesAfterTheGraceWindow() {
        var linger = CompletedLinger()
        linger.observe(finished: ["job-1"], now: start)
        XCTAssertFalse(linger.isVisible("job-1", now: afterGrace))
    }

    func testRunningWorkIsAlwaysVisible() {
        var linger = CompletedLinger()
        linger.observe(finished: [], now: start)
        XCTAssertTrue(linger.isVisible("job-1", now: afterGrace))
    }

    func testRestartedWorkIsVisibleAgain() {
        var linger = CompletedLinger()
        linger.observe(finished: ["job-1"], now: start)
        XCTAssertFalse(linger.isVisible("job-1", now: afterGrace))
        // The job runs again, then finishes again: a fresh grace window.
        linger.observe(finished: [], now: afterGrace)
        XCTAssertTrue(linger.isVisible("job-1", now: afterGrace))
        linger.observe(finished: ["job-1"], now: afterGrace)
        XCTAssertTrue(linger.isVisible("job-1", now: afterGrace))
        XCTAssertFalse(linger.isVisible("job-1", now: afterGrace.addingTimeInterval(CompletedLinger.grace + 1)))
    }

    func testForgettingKeysStopsSchedulingPrunes() {
        var linger = CompletedLinger()
        linger.observe(finished: ["job-1"], now: start)
        XCTAssertTrue(linger.hasFinishedWorkInsideGrace(now: start))
        XCTAssertFalse(linger.hasFinishedWorkInsideGrace(now: afterGrace))
        linger.observe(finished: [], now: afterGrace)
        XCTAssertFalse(linger.hasFinishedWorkInsideGrace(now: afterGrace))
    }

    func testSeveralJobsExpireIndependently() {
        var linger = CompletedLinger()
        linger.observe(finished: ["job-1"], now: start)
        let later = start.addingTimeInterval(CompletedLinger.grace / 2)
        linger.observe(finished: ["job-1", "job-2"], now: later)
        XCTAssertFalse(linger.isVisible("job-1", now: afterGrace))
        XCTAssertTrue(linger.isVisible("job-2", now: afterGrace))
    }
}
