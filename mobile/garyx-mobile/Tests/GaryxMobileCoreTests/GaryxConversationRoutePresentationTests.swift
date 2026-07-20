import XCTest
@testable import GaryxMobileCore

final class GaryxConversationRoutePresentationTests: XCTestCase {
    func testOpeningTranscriptUsesLocalMessagesWhileRefreshRuns() {
        XCTAssertEqual(
            GaryxConversationOpeningTranscriptPolicy.presentation(localRenderableRowCount: 3),
            .localMessages
        )
        XCTAssertEqual(
            GaryxConversationOpeningTranscriptPolicy.presentation(localRenderableRowCount: 0),
            .loading
        )
        XCTAssertEqual(
            GaryxConversationOpeningTranscriptPolicy.presentation(
                localRenderableRowCount: 0,
                hasRenderedSnapshot: true
            ),
            .localMessages
        )
    }

    func testConversationPageIsTheOnlyFullScreenSurfaceFromMount() {
        var state = GaryxConversationRoutePresentationState()

        XCTAssertTrue(state.presentsConversationPage)
        XCTAssertFalse(state.showsFullScreenPlaceholder)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertFalse(state.mountsLiveConversation)
        XCTAssertEqual(state.messagePhase, .waitingForActivation)

        state.apply(lifecycle: .appeared)

        XCTAssertTrue(state.presentsConversationPage)
        XCTAssertFalse(state.showsFullScreenPlaceholder)
        XCTAssertTrue(state.showsOpeningPage)
        XCTAssertEqual(state.messagePhase, .waitingForActivation)
    }

    func testActiveRouteWaitsForFirstDeliveredFrameBeforeMessagePreparation() {
        var state = GaryxConversationRoutePresentationState()

        XCTAssertEqual(state.apply(lifecycle: .appeared), .none)
        XCTAssertFalse(state.needsPresentedFrameClock)
        XCTAssertEqual(state.apply(lifecycle: .active), .none)
        XCTAssertEqual(state.messagePhase, .waitingForActivation)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertFalse(state.mountsLiveConversation)
        XCTAssertTrue(state.needsPresentedFrameClock)

        XCTAssertEqual(state.presentedFrame(interval: nil), .openingPage)
        XCTAssertEqual(state.messagePhase, .loading)
        XCTAssertFalse(state.mountsLiveConversation)
        XCTAssertEqual(state.apply(lifecycle: .active), .none)
    }

    func testLocalDraftReproductionTraversesLoadingOpeningPageDespiteImmediateReadiness() {
        XCTAssertFalse(
            GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
                threadId: nil,
                historyLoaded: false,
                liveRenderSnapshot: nil,
                cachedTranscript: nil
            ),
            "a local draft has no selected thread history to await"
        )

        var state = GaryxConversationRoutePresentationState()
        state.apply(lifecycle: .active)

        // This is the production draft sequence: the shared route driver waits
        // for a delivered frame, enters `.loading`, and only then asks the
        // route stack to prepare content. The route stack immediately reports
        // a non-thread destination ready, because a local draft has no history
        // request to perform.
        XCTAssertEqual(state.presentedFrame(interval: nil), .openingPage)
        XCTAssertEqual(state.messagePhase, .loading)
        state.messageContentDidBecomeReady()
        XCTAssertEqual(state.messagePhase, .ready)
        XCTAssertTrue(state.showsOpeningPage)

        // The registry resets its frame clock when readiness arrives. Even
        // though the draft is already ready, the generic conversation handoff
        // keeps the opening page on top through the terminal opening gate and
        // a complete materialization stability proof.
        XCTAssertEqual(state.presentedFrame(interval: nil), .materializingConversation)
        var materializationFrames = 0
        while state.renderPhase != .live, materializationFrames < 20 {
            materializationFrames += 1
            state.presentedFrame(interval: 1.0 / 120.0)
        }

        XCTAssertEqual(materializationFrames, 13)
        XCTAssertEqual(2 + materializationFrames, 15)
        XCTAssertEqual(state.renderPhase, .live)
        XCTAssertFalse(state.showsOpeningPage)
    }

    func testTerminalFramesMaterializeThenRevealTheLivePage() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 2,
            materializationFrameCount: 3
        )
        state.apply(lifecycle: .appeared)
        state.apply(lifecycle: .active)

        XCTAssertEqual(state.presentedFrame(interval: nil), .openingPage)
        XCTAssertFalse(state.mountsLiveConversation)
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertTrue(state.mountsLiveConversation)
        XCTAssertTrue(state.showsOpeningPage)
        XCTAssertFalse(state.needsPresentedFrameClock)

        state.messageContentDidBecomeReady()
        XCTAssertTrue(state.needsPresentedFrameClock)

        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
        XCTAssertTrue(state.hasPresentedLiveConversation)
        XCTAssertFalse(state.showsOpeningPage)
        XCTAssertFalse(state.needsPresentedFrameClock)
        XCTAssertEqual(state.messagePhase, .ready)
    }

    func testLiveImplementationHandoffWaitsForMessageReadiness() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1
        )
        state.apply(lifecycle: .active)

        XCTAssertEqual(state.messagePhase, .waitingForActivation)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .materializingConversation)
        XCTAssertEqual(state.messagePhase, .loading)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .materializingConversation)
        XCTAssertTrue(state.showsOpeningPage)

        state.messageContentDidBecomeReady()
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
    }

    func testMissedMaterializationFrameRestartsStabilityProof() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 2,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: nil)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.messageContentDidBecomeReady()

        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 30.0),
            .materializingConversation
        )
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
    }

    func testMaterializationReestablishesReferenceAfterContentClockReset() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 2,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)

        // The first delivered frame starts content preparation. Content
        // readiness resets the registry clock, so the next delivered frame
        // may also carry no interval while entering materialization.
        state.presentedFrame(interval: nil)
        state.messageContentDidBecomeReady()
        XCTAssertEqual(state.presentedFrame(interval: nil), .materializingConversation)

        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
    }

    func testInterruptedMaterializationRestartsForReactivatedOccurrence() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 60.0)
        state.messageContentDidBecomeReady()

        XCTAssertEqual(state.apply(lifecycle: .inactive), .none)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertEqual(state.messagePhase, .waitingForActivation)
        XCTAssertEqual(state.apply(lifecycle: .active), .none)
        XCTAssertEqual(state.messagePhase, .waitingForActivation)
        state.presentedFrame(interval: 1.0 / 120.0)
        XCTAssertEqual(state.messagePhase, .loading)
    }

    func testLivePredecessorRetainsItsPreparedConversation() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.messageContentDidBecomeReady()
        state.presentedFrame(interval: 1.0 / 120.0)

        state.apply(lifecycle: .inactive)

        XCTAssertEqual(state.messagePhase, .ready)
        XCTAssertEqual(state.renderPhase, .live)
        XCTAssertTrue(state.mountsLiveConversation)
        XCTAssertFalse(state.showsOpeningPage)
        XCTAssertEqual(state.apply(lifecycle: .active), .none)
    }

    func testRenderPrewarmRequiresConsecutiveStableFrames() {
        var state = GaryxConversationRenderPrewarmState(requiredStableFrameCount: 2)
        state.begin()

        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0, frameBudget: 1.0 / 120.0),
            .materializing
        )
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 30.0, frameBudget: 1.0 / 120.0),
            .materializing
        )
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0, frameBudget: 1.0 / 120.0),
            .materializing
        )
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0, frameBudget: 1.0 / 120.0),
            .ready
        )
        XCTAssertFalse(state.rendersWarmupSurface)
    }

}
