import XCTest
@testable import GaryxMobileCore

final class GaryxSendAnchorFillerStateTests: XCTestCase {
    func testFillerTracksFloorMinusContentExactly() {
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
        XCTAssertEqual(state.runSpaceFloor, 776)

        // Reply growth consumes run space one-for-one: the total below the
        // anchor stays pinned at the floor, so the anchored row never moves.
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 300
            ),
            476
        )
        // A shorter atomic replacement (thinking label collapsing into the
        // committed reply) restores the same total: still no anchor motion,
        // and no blank-space deficit either.
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 100
            ),
            676
        )
        XCTAssertFalse(state.isExhausted)

        // Content reaching the floor leaves no run space and marks the
        // session exhausted: the reply is growing below the screen and the
        // view hands the session off to tail following.
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 900
            ),
            0
        )
        XCTAssertTrue(state.isExhausted)
    }

    func testUnmeasuredSessionCannotExhaust() {
        var state = GaryxSendAnchorFillerState()
        _ = state.begin(
            anchorRowId: "user_turn:origin:send",
            viewportHeight: 0,
            bottomChromeClearance: 24,
            contentBelowAnchorHeight: 500
        )
        XCTAssertFalse(
            state.isExhausted,
            "a zero floor means no viewport was measured; content cannot exhaust it"
        )
    }

    func testFloorRisesWithViewportGrowthAndNeverFalls() {
        var state = GaryxSendAnchorFillerState()

        // Send with the keyboard up: the session starts from the reduced
        // viewport.
        _ = state.begin(
            anchorRowId: "user_turn:origin:send",
            viewportHeight: 500,
            bottomChromeClearance: 24,
            contentBelowAnchorHeight: 120
        )
        XCTAssertEqual(state.runSpaceFloor, 476)
        XCTAssertEqual(state.height, 356)

        // Keyboard dismissal grows the viewport: the floor rises and the
        // filler is allowed to grow back, keeping the anchor-to-top scroll
        // reachable (v1 clamped short here).
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 120
            ),
            656
        )
        XCTAssertEqual(state.runSpaceFloor, 776)

        // Keyboard reappearing must not lower the floor: shrinking run space
        // under a settled anchor would clamp the scroll position upward.
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 500,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 120
            ),
            656
        )
        XCTAssertEqual(state.runSpaceFloor, 776)
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
            "a follow-up send starts a fresh session floor"
        )
        XCTAssertEqual(state.anchorRowId, "user_turn:origin:send-2")

        state.reset()
        XCTAssertNil(state.anchorRowId)
        XCTAssertEqual(state.height, 0)
        XCTAssertEqual(state.runSpaceFloor, 0)
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
        XCTAssertEqual(state.runSpaceFloor, 0)

        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 180
            ),
            596
        )
        XCTAssertEqual(state.runSpaceFloor, 776)
    }

    func testInvalidMeasurementsAreClampedWithoutRaisingTheFloor() {
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
            790,
            "an infinite viewport sample is treated as zero and cannot raise the floor"
        )
        XCTAssertEqual(state.runSpaceFloor, 800)
    }

    func testAnchorTopInsetReducesRequiredRunSpace() {
        var state = GaryxSendAnchorFillerState()
        // The anchored row sits `anchorTopInset` below the viewport top
        // (v2.1 breathing room under the title capsule), so exactly that
        // much less run space is required underneath it.
        XCTAssertEqual(
            state.begin(
                anchorRowId: "user_turn:origin:send",
                viewportHeight: 800,
                bottomChromeClearance: 24,
                anchorTopInset: 16,
                contentBelowAnchorHeight: 200
            ),
            560
        )
        XCTAssertEqual(state.runSpaceFloor, 760)
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                anchorTopInset: 16,
                contentBelowAnchorHeight: 760
            ),
            0
        )
        XCTAssertTrue(state.isExhausted)
    }

    func testGestureRetirementShrinkWrapsInsteadOfClamping() {
        var state = GaryxSendAnchorFillerState()
        _ = state.begin(
            anchorRowId: "user_turn:origin:send",
            viewportHeight: 800,
            bottomChromeClearance: 24,
            anchorTopInset: 16,
            contentBelowAnchorHeight: 120
        )
        let heightAtExit = state.height
        XCTAssertEqual(heightAtExit, 640)

        state.beginRetiring()
        XCTAssertTrue(state.isRetiring)

        // Floor reconciliation must not regrow a retiring spacer.
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 900,
                bottomChromeClearance: 24,
                anchorTopInset: 16,
                contentBelowAnchorHeight: 120
            ),
            heightAtExit
        )

        // Upward reading motion trims exactly the scrollable excess; a
        // bottom rubber-band (negative excess) trims nothing.
        XCTAssertEqual(state.trim(scrollableExcessBelowViewport: 25), 615)
        XCTAssertEqual(state.trim(scrollableExcessBelowViewport: -40), 615)
        XCTAssertEqual(state.trim(scrollableExcessBelowViewport: 0), 615)

        // Consuming the rest clears the session entirely.
        XCTAssertEqual(state.trim(scrollableExcessBelowViewport: 10_000), 0)
        XCTAssertNil(state.anchorRowId)
        XCTAssertFalse(state.isRetiring)
        XCTAssertEqual(state.runSpaceFloor, 0)
    }

    func testReconcileWithoutSessionStaysEmpty() {
        var state = GaryxSendAnchorFillerState()
        XCTAssertEqual(
            state.reconcile(
                viewportHeight: 800,
                bottomChromeClearance: 24,
                contentBelowAnchorHeight: 100
            ),
            0
        )
        XCTAssertNil(state.anchorRowId)
        XCTAssertEqual(state.runSpaceFloor, 0)
    }
}
