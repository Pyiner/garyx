import XCTest
@testable import GaryxMobileCore

final class GaryxSendAnchorFillerStateTests: XCTestCase {
    func testSessionFillerShrinksWithReplyGrowthAndNeverRegrows() {
        var state = GaryxSendAnchorFillerState()

        XCTAssertEqual(
            state.begin(
                anchorRowId: "user_turn:origin:send-1",
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 200
            ),
            576
        )
        XCTAssertEqual(state.anchorRowId, "user_turn:origin:send-1")
        XCTAssertTrue(state.hasMeasuredViewport)

        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 300
            ),
            476
        )
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 100
            ),
            476,
            "a shorter atomic replacement must not regrow run space"
        )
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 900
            ),
            0
        )
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 50
            ),
            0,
            "zero is terminal within one send-anchored session"
        )
    }

    func testNewSendRebuildsRunSpaceAndResetClearsSession() {
        var state = GaryxSendAnchorFillerState()
        _ = state.begin(
            anchorRowId: "user_turn:origin:send-1",
            viewportHeight: 800,
            bottomChromeClearance: 24,
            contentBelowAnchorHeight: 900
        )
        XCTAssertEqual(state.height, 0)

        XCTAssertEqual(
            state.begin(
                anchorRowId: "user_turn:origin:send-2",
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 120
            ),
            656,
            "a follow-up send starts a fresh monotonic session"
        )
        XCTAssertEqual(state.anchorRowId, "user_turn:origin:send-2")

        state.reset()
        XCTAssertNil(state.anchorRowId)
        XCTAssertEqual(state.height, 0)
        XCTAssertFalse(state.hasMeasuredViewport)
    }

    func testFirstValidViewportEstablishesAnInitiallyUnmeasuredSession() {
        var state = GaryxSendAnchorFillerState()
        XCTAssertEqual(
            state.begin(
                anchorRowId: "user_turn:origin:send",
                viewportHeight: 0,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 180
            ),
            0
        )
        XCTAssertFalse(state.hasMeasuredViewport)

        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 180
            ),
            596
        )
        XCTAssertTrue(state.hasMeasuredViewport)
    }

    func testInvalidMeasurementsAreClampedWithoutBreakingMonotonicity() {
        var state = GaryxSendAnchorFillerState()
        XCTAssertEqual(
            state.begin(
                anchorRowId: "user_turn:origin:send",
                viewportHeight: 800,
                bottomChromeClearance: -20,
                contentBelowAnchorHeight: .nan
            ),
            800
        )
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: .infinity,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 10
            ),
            800
        )
    }
}
