import XCTest
@testable import GaryxMobileCore

final class GaryxRuntimePanelPagerTests: XCTestCase {
    private enum Page: Equatable {
        case main
        case model
        case thinking
    }

    // MARK: - Two-phase ordering

    func testBeginHidesContentAndCompleteSwapsAndShows() {
        var pager = GaryxRuntimePanelPager<Page>(page: .main)
        XCTAssertTrue(pager.isContentVisible)

        let token = pager.begin(to: .model)
        XCTAssertNotNil(token)
        // Exit phase: still on the old page, fully hidden — the swap must
        // not happen while anything is visible.
        XCTAssertEqual(pager.page, .main)
        XCTAssertFalse(pager.isContentVisible)

        XCTAssertTrue(pager.complete(token: token!, to: .model))
        XCTAssertEqual(pager.page, .model)
        XCTAssertTrue(pager.isContentVisible)
    }

    func testBeginToCurrentPageIsNoOp() {
        var pager = GaryxRuntimePanelPager<Page>(page: .main)
        XCTAssertNil(pager.begin(to: .main))
        XCTAssertTrue(pager.isContentVisible)
        XCTAssertEqual(pager.transitionToken, 0)
    }

    // MARK: - Latest wins

    func testRapidRequestsLatestWins() {
        var pager = GaryxRuntimePanelPager<Page>(page: .main)
        let first = pager.begin(to: .model)!
        let second = pager.begin(to: .thinking)!
        XCTAssertNotEqual(first, second)

        // The stale completion must be rejected wholesale.
        XCTAssertFalse(pager.complete(token: first, to: .model))
        XCTAssertEqual(pager.page, .main)
        XCTAssertFalse(pager.isContentVisible)

        XCTAssertTrue(pager.complete(token: second, to: .thinking))
        XCTAssertEqual(pager.page, .thinking)
        XCTAssertTrue(pager.isContentVisible)
    }

    func testCompletedTokenCannotFireTwice() {
        var pager = GaryxRuntimePanelPager<Page>(page: .main)
        let token = pager.begin(to: .model)!
        XCTAssertTrue(pager.complete(token: token, to: .model))

        // A duplicate delivery of the same continuation must not re-apply
        // after a newer request started.
        let newer = pager.begin(to: .thinking)!
        XCTAssertFalse(pager.complete(token: token, to: .model))
        XCTAssertTrue(pager.complete(token: newer, to: .thinking))
    }

    // MARK: - Collapse invalidation

    func testResetInvalidatesPendingCompletionAndShowsRoot() {
        var pager = GaryxRuntimePanelPager<Page>(page: .model)
        let token = pager.begin(to: .thinking)!

        // Scrim collapse mid-transition: back to root instantly.
        pager.reset(to: .main)
        XCTAssertEqual(pager.page, .main)
        XCTAssertTrue(pager.isContentVisible)

        // The in-flight phase-2 continuation lands afterwards and must not
        // resurrect the sub-page.
        XCTAssertFalse(pager.complete(token: token, to: .thinking))
        XCTAssertEqual(pager.page, .main)
        XCTAssertTrue(pager.isContentVisible)
    }

    func testReopenAfterResetStartsFreshFromRoot() {
        var pager = GaryxRuntimePanelPager<Page>(page: .model)
        pager.reset(to: .main)

        let token = pager.begin(to: .model)!
        XCTAssertTrue(pager.complete(token: token, to: .model))
        XCTAssertEqual(pager.page, .model)
    }

    // MARK: - Viewport metrics

    func testViewportEstimateUsesRowMetricsAndHairlines() {
        let height = GaryxRuntimeOptionsViewportMetrics.height(
            rowCount: 5,
            hairlineHeight: 1.0 / 3.0,
            measuredContentHeight: nil,
            maxHeight: 520
        )
        XCTAssertEqual(height, 5 * 44 + 4 * (1.0 / 3.0) + 16, accuracy: 0.001)
    }

    func testViewportPrefersMeasuredHeight() {
        let height = GaryxRuntimeOptionsViewportMetrics.height(
            rowCount: 5,
            hairlineHeight: 1.0 / 3.0,
            measuredContentHeight: 361,
            maxHeight: 520
        )
        XCTAssertEqual(height, 361)
    }

    func testViewportFloorsTinyContent() {
        let height = GaryxRuntimeOptionsViewportMetrics.height(
            rowCount: 1,
            hairlineHeight: 1.0 / 3.0,
            measuredContentHeight: nil,
            maxHeight: 520
        )
        XCTAssertEqual(height, 96)
    }

    func testViewportCapsLongLists() {
        let height = GaryxRuntimeOptionsViewportMetrics.height(
            rowCount: 23,
            hairlineHeight: 1.0 / 3.0,
            measuredContentHeight: nil,
            maxHeight: 520
        )
        XCTAssertEqual(height, 520)

        let measured = GaryxRuntimeOptionsViewportMetrics.height(
            rowCount: 23,
            hairlineHeight: 1.0 / 3.0,
            measuredContentHeight: 1035.33,
            maxHeight: 520
        )
        XCTAssertEqual(measured, 520)
    }
}
