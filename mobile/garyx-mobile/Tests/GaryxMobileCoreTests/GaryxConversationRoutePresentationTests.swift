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
        XCTAssertFalse(state.hasBegunContentPreparation)

        state.apply(lifecycle: .appeared)

        XCTAssertTrue(state.presentsConversationPage)
        XCTAssertFalse(state.showsFullScreenPlaceholder)
        XCTAssertTrue(state.showsOpeningPage)
        XCTAssertFalse(state.hasBegunContentPreparation)
    }

    func testActiveRouteWaitsForFirstDeliveredFrameBeforeContentPreparation() {
        var state = GaryxConversationRoutePresentationState()

        state.apply(lifecycle: .appeared)
        XCTAssertFalse(state.needsPresentedFrameClock)
        state.apply(lifecycle: .active)
        XCTAssertFalse(state.hasBegunContentPreparation)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertFalse(state.mountsLiveConversation)
        XCTAssertTrue(state.needsPresentedFrameClock)

        XCTAssertEqual(state.presentedFrame(interval: nil), .openingPage)
        XCTAssertTrue(state.hasBegunContentPreparation)
        XCTAssertFalse(state.mountsLiveConversation)
        state.apply(lifecycle: .active)
    }

    func testLocalDraftReproductionBypassesGatewayThreadOpeningStateMachine() {
        XCTAssertFalse(
            GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
                threadId: nil,
                historyLoaded: false,
                liveRenderSnapshot: nil,
                cachedTranscript: nil
            ),
            "a local draft has no selected thread history to await"
        )

        let plan = GaryxConversationRoutePresentationPolicy.plan(
            for: .conversationDraft(draftID: "draft-local")
        )

        XCTAssertEqual(plan, .directLocal)
        XCTAssertTrue(plan?.mountsFinalChromeOnFirstFrame == true)
        XCTAssertFalse(plan?.usesOpeningMaterializationStateMachine == true)
    }

    func testExistingThreadRetainsStagedOpeningAndMaterializationStateMachine() {
        let plan = GaryxConversationRoutePresentationPolicy.plan(
            for: .conversation(threadID: "thread-existing")
        )

        XCTAssertEqual(plan, .stagedGatewayThread)
        XCTAssertFalse(plan?.mountsFinalChromeOnFirstFrame == true)
        XCTAssertTrue(plan?.usesOpeningMaterializationStateMachine == true)
        XCTAssertEqual(
            GaryxConversationRoutePresentationState.defaultTerminalOpeningFrameCount,
            2
        )
        XCTAssertEqual(
            GaryxConversationRoutePresentationState.defaultMaterializationFrameCount,
            12
        )
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
        XCTAssertTrue(state.allowsLiveConversationInteraction)
    }

    func testLiveImplementationHandoffWaitsForStableMaterializationFrame() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1
        )
        state.apply(lifecycle: .active)

        XCTAssertFalse(state.hasBegunContentPreparation)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .materializingConversation)
        XCTAssertTrue(state.hasBegunContentPreparation)
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
        XCTAssertTrue(state.allowsLiveConversationInteraction)
    }

    func testInFlightHistoryRefreshCannotOwnTranscriptInteractionHandoff() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)

        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )

        // An initial history refresh remains in flight, so no network
        // completion signal is delivered. Once the live graph has survived
        // the normal stability gate, it must still take interaction ownership
        // from the pixel-continuity cover.
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
        XCTAssertTrue(state.hasPresentedLiveConversation)
        XCTAssertTrue(state.allowsLiveConversationInteraction)
        XCTAssertFalse(state.showsOpeningPage)
    }

    func testMissedMaterializationFrameRestartsStabilityProof() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 2,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: nil)
        state.presentedFrame(interval: 1.0 / 120.0)

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

    func testMaterializationEstablishesReferenceAfterNilOpeningSamples() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 2,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)

        // Terminal delivery can begin without an interval sample. Entering
        // materialization must establish a new reference and still complete
        // the consecutive-frame proof.
        state.presentedFrame(interval: nil)
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

        state.apply(lifecycle: .inactive)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertFalse(state.hasBegunContentPreparation)
        state.apply(lifecycle: .active)
        XCTAssertFalse(state.hasBegunContentPreparation)
        state.presentedFrame(interval: 1.0 / 120.0)
        XCTAssertTrue(state.hasBegunContentPreparation)
    }

    func testLivePredecessorRetainsItsPreparedConversation() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.presentedFrame(interval: 1.0 / 120.0)

        state.apply(lifecycle: .inactive)

        XCTAssertTrue(state.hasBegunContentPreparation)
        XCTAssertEqual(state.renderPhase, .live)
        XCTAssertTrue(state.mountsLiveConversation)
        XCTAssertFalse(state.showsOpeningPage)
        state.apply(lifecycle: .active)
        XCTAssertEqual(state.renderPhase, .live)
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
