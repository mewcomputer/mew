import SwiftUI
import XCTest
@testable import mew

final class ThinkingGaugeTests: XCTestCase {
    func testNeedleFollowsTheCanonicalEffortOrder() {
        XCTAssertEqual(ThinkingGauge.position(for: nil), 0)
        XCTAssertEqual(ThinkingGauge.position(for: "none"), 0)
        XCTAssertEqual(ThinkingGauge.position(for: " off "), 0)
        XCTAssertEqual(ThinkingGauge.position(for: "ultra"), 1)

        let ordered = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
        let positions = ordered.map { ThinkingGauge.position(for: $0) }
        XCTAssertEqual(positions, positions.sorted(), "Higher effort must never sit lower on the dial")
        XCTAssertEqual(Set(positions).count, ordered.count, "Every level needs its own needle position")
    }

    /// A model's subset of levels is irrelevant: the dial is absolute, so `high`
    /// reads the same whether the model also offers `xhigh` or stops there.
    func testUnknownLevelsReadAsEngagedRatherThanIdle() {
        XCTAssertEqual(ThinkingGauge.position(for: "turbo"), 1)
        XCTAssertEqual(ThinkingGauge.position(for: "HIGH"), ThinkingGauge.position(for: "high"))
    }

    @MainActor
    func testGaugeStaysInsideItsFrame() {
        for size in [13.0, 52.0] {
            let view = ThinkingGauge(variant: "high", size: size, tint: .accentColor, showsTicks: true)
                .environment(\.mewTheme, MewThemes.palette(id: MewThemes.defaultDarkId, fallbackDark: true))
            let host = UIHostingController(rootView: view)
            let fitted = host.sizeThatFits(in: CGSize(width: size * 4, height: size * 4))
            XCTAssertLessThanOrEqual(fitted.height, size + 1, "The dial must not outgrow the badge it sits in")
            XCTAssertLessThanOrEqual(fitted.width, size + 1)
        }
    }
}
