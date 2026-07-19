import XCTest
@testable import GaryxMobileCore

final class GaryxConversationRoutePresentationTests: XCTestCase {
    func testLiveContentNeverMountsInsideTransitionLifecycle() {
        var state = GaryxConversationRoutePresentationState()

        XCTAssertEqual(state.renderPhase, .transitionPlaceholder)
        XCTAssertFalse(state.mountsLiveContent)
        XCTAssertFalse(state.needsPresentedFrameClock)

        state.apply(lifecycle: .appeared)
        for _ in 0..<8 {
            state.presentedFrame(interval: 1.0 / 120.0)
        }

        XCTAssertEqual(state.renderPhase, .transitionPlaceholder)
        XCTAssertFalse(state.mountsLiveContent)
        XCTAssertFalse(state.needsPresentedFrameClock)
    }

    func testTerminalFramesStagePreparationThenReveal() {
        var state = GaryxConversationRoutePresentationState()
        state.apply(lifecycle: .appeared)
        state.apply(lifecycle: .active)

        XCTAssertTrue(state.needsPresentedFrameClock)
        XCTAssertEqual(state.presentedFrame(interval: nil), .transitionPlaceholder)
        XCTAssertFalse(state.mountsLiveContent)

        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .preparingLiveContent)
        XCTAssertTrue(state.mountsLiveContent)
        XCTAssertTrue(state.showsPlaceholder)
        XCTAssertFalse(state.needsPresentedFrameClock)

        state.contentDidBecomeReady()
        XCTAssertTrue(state.needsPresentedFrameClock)
        XCTAssertEqual(state.presentedFrame(interval: nil), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
        XCTAssertTrue(state.hasPresentedLiveContent)
        XCTAssertFalse(state.showsPlaceholder)
        XCTAssertFalse(state.needsPresentedFrameClock)
    }

    func testInterruptedPreparationRestartsFromPlaceholder() {
        var state = GaryxConversationRoutePresentationState(
            terminalPlaceholderFrameCount: 1,
            livePreparationFrameCount: 2
        )
        state.apply(lifecycle: .appeared)
        state.apply(lifecycle: .active)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 60.0), .preparingLiveContent)
        state.contentDidBecomeReady()

        state.apply(lifecycle: .inactive)
        XCTAssertEqual(state.renderPhase, .transitionPlaceholder)
        XCTAssertFalse(state.mountsLiveContent)

        state.apply(lifecycle: .active)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 60.0), .preparingLiveContent)
        XCTAssertFalse(state.needsPresentedFrameClock)
        state.contentDidBecomeReady()
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 60.0), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 60.0), .live)
    }

    func testPreparedPredecessorKeepsLiveSurfaceWhileInactive() {
        var state = GaryxConversationRoutePresentationState(
            terminalPlaceholderFrameCount: 1,
            livePreparationFrameCount: 1
        )
        state.apply(lifecycle: .appeared)
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.contentDidBecomeReady()
        state.presentedFrame(interval: 1.0 / 120.0)
        XCTAssertEqual(state.renderPhase, .live)

        state.apply(lifecycle: .inactive)
        XCTAssertEqual(state.renderPhase, .live)
        XCTAssertTrue(state.mountsLiveContent)
        XCTAssertFalse(state.showsPlaceholder)

        state.apply(lifecycle: .active)
        XCTAssertEqual(state.renderPhase, .live)
        XCTAssertFalse(state.needsPresentedFrameClock)
    }

    func testMissedPreparationFrameRestartsStabilityBudget() {
        var state = GaryxConversationRoutePresentationState(
            terminalPlaceholderFrameCount: 2,
            livePreparationFrameCount: 2
        )
        state.apply(lifecycle: .appeared)
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: nil)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.contentDidBecomeReady()

        XCTAssertEqual(state.presentedFrame(interval: nil), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 60.0), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .preparingLiveContent)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
    }
}
