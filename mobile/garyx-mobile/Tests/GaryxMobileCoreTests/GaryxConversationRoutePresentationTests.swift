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

    func testWarmReentrySnapshotAlignsFirstRowWithLiveTranscript() {
        let capture = GaryxConversationTranscriptSnapshotCaptureGeometry(
            viewportFrameInPage: CGRect(x: 0, y: 0, width: 402, height: 874),
            adjustedContentInsets: .init(top: 124, left: 0, bottom: 0, right: 0),
            contentOffset: CGPoint(x: 0, y: -124)
        )
        let openingContainerFrame = CGRect(x: 0, y: 124, width: 402, height: 593)
        let installationFrame = GaryxConversationTranscriptSnapshotGeometry.installationFrame(
            capture: capture,
            containerFrameInPage: openingContainerFrame
        )

        // The captured short-thread row begins 34 pt into content. With the
        // measured -124 content offset it is at y=158 in the live full-page
        // scroll view. Installing those same pixels in the transcript-only
        // container must preserve that page coordinate through the handoff.
        let firstRowInSnapshot = capture.snapshotPoint(
            forContentPoint: CGPoint(x: 16, y: 34)
        )
        let liveFirstRowY = capture.viewportFrameInPage.minY + firstRowInSnapshot.y
        let openingFirstRowY = openingContainerFrame.minY
            + installationFrame.minY
            + firstRowInSnapshot.y

        XCTAssertEqual(capture.contentOffset.y, -capture.adjustedContentInsets.top)
        XCTAssertEqual(liveFirstRowY, 158, accuracy: 0.001)
        XCTAssertEqual(
            openingFirstRowY,
            liveFirstRowY,
            accuracy: 0.001,
            "the opening snapshot and live scroll view must put the first row at the same page y"
        )
        XCTAssertEqual(installationFrame.minY, -124, accuracy: 0.001)
        XCTAssertEqual(installationFrame.size, CGSize(width: 402, height: 874))
    }

    func testConversationPageIsTheOnlyFullScreenSurfaceFromMount() {
        var state = GaryxConversationRoutePresentationState()

        XCTAssertTrue(state.presentsConversationPage)
        XCTAssertFalse(state.showsFullScreenPlaceholder)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertFalse(state.mountsLiveTranscript)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertFalse(state.allowsTranscriptInteraction)
        XCTAssertFalse(state.hasBegunContentPreparation)

        state.apply(lifecycle: .appeared)

        XCTAssertTrue(state.presentsConversationPage)
        XCTAssertFalse(state.showsFullScreenPlaceholder)
        XCTAssertTrue(state.showsOpeningTranscriptCover)
        XCTAssertFalse(state.hasBegunContentPreparation)
    }

    func testActiveRouteWaitsForFirstDeliveredFrameBeforeContentPreparation() {
        var state = GaryxConversationRoutePresentationState()

        state.apply(lifecycle: .appeared)
        XCTAssertFalse(state.needsPresentedFrameClock)
        state.apply(lifecycle: .active)
        XCTAssertFalse(state.hasBegunContentPreparation)
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertFalse(state.mountsLiveTranscript)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertFalse(state.allowsTranscriptInteraction)
        XCTAssertTrue(state.needsPresentedFrameClock)

        XCTAssertEqual(state.presentedFrame(interval: nil), .openingPage)
        XCTAssertTrue(state.hasBegunContentPreparation)
        XCTAssertFalse(state.mountsLiveTranscript)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertFalse(state.allowsTranscriptInteraction)
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
        XCTAssertFalse(state.mountsLiveTranscript)
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertTrue(state.mountsLiveTranscript)
        XCTAssertTrue(state.showsOpeningTranscriptCover)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertFalse(state.allowsTranscriptInteraction)
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
        XCTAssertTrue(state.hasPresentedLiveTranscript)
        XCTAssertFalse(state.showsOpeningTranscriptCover)
        XCTAssertFalse(state.needsPresentedFrameClock)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertTrue(state.allowsTranscriptInteraction)
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
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertTrue(state.allowsTranscriptInteraction)
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
        XCTAssertTrue(state.hasPresentedLiveTranscript)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertTrue(state.allowsTranscriptInteraction)
        XCTAssertFalse(state.showsOpeningTranscriptCover)
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
        XCTAssertTrue(state.mountsLiveTranscript)
        XCTAssertFalse(state.showsOpeningTranscriptCover)
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

    func testComposerInteractionIsIndependentOfEveryTranscriptRenderPhase() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1
        )
        state.apply(lifecycle: .active)

        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertFalse(state.allowsTranscriptInteraction)

        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertFalse(state.allowsTranscriptInteraction)

        XCTAssertEqual(state.presentedFrame(interval: 1.0 / 120.0), .live)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertTrue(state.allowsTranscriptInteraction)
    }

}
