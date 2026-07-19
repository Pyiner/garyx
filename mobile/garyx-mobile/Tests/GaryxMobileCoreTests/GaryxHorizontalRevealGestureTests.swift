import XCTest
@testable import GaryxMobileCore

final class GaryxHorizontalRevealGestureTests: XCTestCase {
    func testFastShortFlickOpensWhileSlowMiddleReleaseCancels() throws {
        let extent: CGFloat = 330

        var flick = GaryxHorizontalRevealState(position: .closed, extent: extent)
        flick.beginDrag(extent: extent)
        flick.updateDrag(logicalTranslation: extent * 0.18, extent: extent)
        let flickSettle = try XCTUnwrap(flick.release(
            logicalVelocity: 300,
            extent: extent,
            projection: .fullScreenNavigation
        ))
        XCTAssertEqual(flickSettle.target, .open)
        XCTAssertEqual(flickSettle.initialVelocity, 300)

        var slow = GaryxHorizontalRevealState(position: .closed, extent: extent)
        slow.beginDrag(extent: extent)
        slow.updateDrag(logicalTranslation: extent * 0.40, extent: extent)
        let slowSettle = try XCTUnwrap(slow.release(
            logicalVelocity: 0,
            extent: extent,
            projection: .fullScreenNavigation
        ))
        XCTAssertEqual(slowSettle.target, .closed)
    }

    func testDragUsesSignedRubberBandBeyondBothEndpoints() {
        let extent: CGFloat = 320
        var closed = GaryxHorizontalRevealState(position: .closed, extent: extent)
        closed.beginDrag(extent: extent)
        let below = closed.updateDrag(logicalTranslation: -80, extent: extent)
        XCTAssertLessThan(below, 0)
        XCTAssertGreaterThan(below, -80)

        var open = GaryxHorizontalRevealState(position: .open, extent: extent)
        open.beginDrag(extent: extent)
        let above = open.updateDrag(logicalTranslation: 80, extent: extent)
        XCTAssertGreaterThan(above, extent)
        XCTAssertLessThan(above, extent + 80)
    }

    func testSettleCanBeInterruptedWithoutAnOpeningGate() throws {
        let extent: CGFloat = 400
        var state = GaryxHorizontalRevealState(position: .closed, extent: extent)
        state.beginDrag(extent: extent)
        state.updateDrag(logicalTranslation: 90, extent: extent)
        XCTAssertEqual(
            try XCTUnwrap(state.release(
                logicalVelocity: 500,
                extent: extent,
                projection: .fullScreenNavigation
            )).target,
            .open
        )
        XCTAssertTrue(state.isSettling)

        state.beginDrag(interruptedReveal: 172, extent: extent)
        XCTAssertEqual(state.phase, .dragging)
        XCTAssertEqual(state.reveal, 172)
        state.updateDrag(logicalTranslation: -120, extent: extent)
        let reversal = try XCTUnwrap(state.release(
            logicalVelocity: -300,
            extent: extent,
            projection: .fullScreenNavigation
        ))
        XCTAssertEqual(reversal.target, .closed)
        XCTAssertEqual(reversal.initialReveal, 52)
        XCTAssertEqual(reversal.initialVelocity, -300)
    }

    func testDragAndProgrammaticSettleUseAnInvisibleHitTestingFreeze() throws {
        let extent: CGFloat = 330
        var state = GaryxHorizontalRevealState(position: .closed, extent: extent)

        XCTAssertTrue(state.phase.allowsSurfaceHitTesting)

        state.beginDrag(extent: extent)
        XCTAssertFalse(
            state.phase.allowsSurfaceHitTesting,
            "an active drag blocks taps without disabling the rendered controls"
        )

        state.synchronize(to: .closed, extent: extent)
        _ = try XCTUnwrap(state.beginProgrammaticSettle(
            to: .open,
            initialVelocity: 0,
            extent: extent
        ))
        XCTAssertFalse(
            state.phase.allowsSurfaceHitTesting,
            "a programmatic settle must not become a visually disabled content state"
        )

        state.updateSettle(sampledReveal: extent, extent: extent)
        XCTAssertEqual(state.finishSettle(extent: extent), .open)
        XCTAssertTrue(state.phase.allowsSurfaceHitTesting)
    }

    func testCancellationAfterRegrabResumesInterruptedTarget() throws {
        let extent: CGFloat = 300
        var state = GaryxHorizontalRevealState(position: .closed, extent: extent)
        state.beginDrag(extent: extent)
        state.updateDrag(logicalTranslation: 200, extent: extent)
        XCTAssertEqual(
            try XCTUnwrap(state.release(
                logicalVelocity: 0,
                extent: extent,
                projection: .fullScreenNavigation
            )).target,
            .open
        )

        state.beginDrag(interruptedReveal: 230, extent: extent)
        state.updateDrag(logicalTranslation: -40, extent: extent)
        XCTAssertEqual(try XCTUnwrap(state.cancelDrag(extent: extent)).target, .open)
    }

    func testShortTravelProjectionIsUsedWithoutPredictedEndTranslation() throws {
        let extent: CGFloat = 106
        var state = GaryxHorizontalRevealState(position: .closed, extent: extent)
        state.beginDrag(extent: extent)
        state.updateDrag(logicalTranslation: 18, extent: extent)
        let settle = try XCTUnwrap(state.release(
            logicalVelocity: 220,
            extent: extent,
            projection: .shortTravelDismiss
        ))
        XCTAssertEqual(settle.target, .open)
        XCTAssertEqual(settle.initialVelocity, 220)
    }
}
