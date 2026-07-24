import XCTest
@testable import GaryxMobileCore

final class GaryxConversationRoutePresentationTests: XCTestCase {
    func testTranscriptTreatmentRecomputesFromLiveHistoryInputs() {
        XCTAssertEqual(
            GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: 3,
                hasRenderedSnapshot: false,
                isAwaitingInitialHistory: true
            ),
            .content
        )
        XCTAssertEqual(
            GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: 0,
                hasRenderedSnapshot: false,
                isAwaitingInitialHistory: true
            ),
            .skeleton
        )
        XCTAssertEqual(
            GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: 0,
                hasRenderedSnapshot: true,
                isAwaitingInitialHistory: true
            ),
            .content
        )
        XCTAssertEqual(
            GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: 0,
                hasRenderedSnapshot: false,
                hasTranscriptSnapshotPixels: true,
                isAwaitingInitialHistory: true
            ),
            .content
        )
        XCTAssertEqual(
            GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: 0,
                hasRenderedSnapshot: false,
                isAwaitingInitialHistory: false
            ),
            .content,
            "settled empty history belongs to the ordinary content branch"
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

    func testOpeningViewportCaptureRequiresCanonicalTailNoInteractionAndStableGeometry() throws {
        var capture = GaryxConversationOpeningViewportCaptureState(
            requiredStableSampleCount: 2
        )
        XCTAssertNil(
            capture.observe(openingSample(offsetY: 2_304, isFollowingTail: true)),
            "near-bottom state is not a substitute for the canonical opening offset"
        )
        XCTAssertNil(
            capture.observe(openingSample(offsetY: 2_400, isUserInteracting: true))
        )

        let tail = openingSample(offsetY: 2_400)
        XCTAssertNil(capture.observe(tail))
        let changedGeometry = openingSample(offsetY: 2_500, contentHeight: 3_300)
        XCTAssertNil(
            capture.observe(changedGeometry),
            "a content-size/offset change must restart the consecutive proof"
        )
        let contract = try XCTUnwrap(capture.observe(changedGeometry))
        XCTAssertEqual(contract.captureGeometry.contentOffset.y, 2_500)
        XCTAssertEqual(contract.contentSize.height, 3_300)

        var changedEpoch = changedGeometry
        changedEpoch = GaryxConversationOpeningViewportSample(
            captureGeometry: changedEpoch.captureGeometry,
            visibleViewportFrameInPage: changedEpoch.visibleViewportFrameInPage,
            contentSize: changedEpoch.contentSize,
            displayScale: changedEpoch.displayScale,
            layoutEpoch: changedEpoch.layoutEpoch + 1,
            isFollowingTail: changedEpoch.isFollowingTail,
            isUserInteracting: changedEpoch.isUserInteracting
        )
        XCTAssertNil(
            capture.observe(changedEpoch),
            "a layout epoch change must restart the capture proof"
        )
    }

    func testOpeningViewportServiceRequiresSameRevisionAndVisibleGeometry() throws {
        var capture = GaryxConversationOpeningViewportCaptureState(
            requiredStableSampleCount: 1
        )
        let sample = openingSample(offsetY: 2_400)
        let contract = try XCTUnwrap(capture.observe(sample))

        XCTAssertTrue(
            GaryxConversationOpeningViewportContractPolicy.canServe(
                contract,
                revisionMatches: true,
                visibleViewportFrameInPage: sample.visibleViewportFrameInPage
            )
        )
        XCTAssertFalse(
            GaryxConversationOpeningViewportContractPolicy.canServe(
                contract,
                revisionMatches: false,
                visibleViewportFrameInPage: sample.visibleViewportFrameInPage
            )
        )
        XCTAssertFalse(
            GaryxConversationOpeningViewportContractPolicy.canServe(
                contract,
                revisionMatches: true,
                visibleViewportFrameInPage:
                    sample.visibleViewportFrameInPage.offsetBy(dx: 0, dy: 1)
            )
        )
    }

    func testOpeningViewportCanonicalTailUsesExactDisplayPixelGrid() throws {
        let viewport = CGRect(x: 0, y: 0, width: 440, height: 956)
        let contentSize = CGSize(width: 440, height: 1_157)
        let settledTailOffset = CGFloat(1_196) / 3
        let captureGeometry =
            GaryxConversationTranscriptSnapshotCaptureGeometry(
                viewportFrameInPage: viewport,
                adjustedContentInsets: .init(
                    top: 124,
                    left: 0,
                    bottom: 197.6015625,
                    right: 0
                ),
                contentOffset: CGPoint(x: 0, y: settledTailOffset)
            )
        let sample = GaryxConversationOpeningViewportSample(
            captureGeometry: captureGeometry,
            visibleViewportFrameInPage: CGRect(x: 0, y: 0, width: 440, height: 798),
            contentSize: contentSize,
            displayScale: 3,
            layoutEpoch: 4,
            isFollowingTail: true,
            isUserInteracting: false
        )

        XCTAssertEqual(sample.canonicalTailContentOffsetY, settledTailOffset)
        XCTAssertEqual(sample.distanceFromCanonicalTail, 0)
        let contract = try XCTUnwrap(
            GaryxConversationOpeningViewportContractPolicy.captureContract(for: sample)
        )

        let coordinateRoundTripSample = GaryxConversationOpeningViewportSample(
            captureGeometry:
                GaryxConversationTranscriptSnapshotCaptureGeometry(
                    viewportFrameInPage: viewport.offsetBy(
                        dx: 0,
                        dy: -5.6843418860808015e-14
                    ),
                    adjustedContentInsets: captureGeometry.adjustedContentInsets,
                    contentOffset: captureGeometry.contentOffset
                ),
            visibleViewportFrameInPage: sample.visibleViewportFrameInPage,
            contentSize: contentSize,
            displayScale: 3,
            layoutEpoch: sample.layoutEpoch,
            isFollowingTail: true,
            isUserInteracting: false
        )
        XCTAssertEqual(
            GaryxConversationOpeningViewportContractPolicy.revealReadiness(
                for: contract,
                live: coordinateRoundTripSample,
                revisionMatches: true
            ),
            .matched,
            "coordinate conversion round-off that resolves to the same pixels is exact"
        )

        let shiftedPixelSample = GaryxConversationOpeningViewportSample(
            captureGeometry:
                GaryxConversationTranscriptSnapshotCaptureGeometry(
                    viewportFrameInPage: viewport.offsetBy(dx: 0, dy: 1.0 / 3.0),
                    adjustedContentInsets: captureGeometry.adjustedContentInsets,
                    contentOffset: captureGeometry.contentOffset
                ),
            visibleViewportFrameInPage: sample.visibleViewportFrameInPage,
            contentSize: contentSize,
            displayScale: 3,
            layoutEpoch: sample.layoutEpoch,
            isFollowingTail: true,
            isUserInteracting: false
        )
        XCTAssertEqual(
            GaryxConversationOpeningViewportContractPolicy.revealReadiness(
                for: contract,
                live: shiftedPixelSample,
                revisionMatches: true
            ),
            .pending,
            "one real display pixel remains a geometry mismatch"
        )

        let unalignedSample = GaryxConversationOpeningViewportSample(
            captureGeometry:
                GaryxConversationTranscriptSnapshotCaptureGeometry(
                    viewportFrameInPage: viewport,
                    adjustedContentInsets: captureGeometry.adjustedContentInsets,
                    contentOffset: CGPoint(x: 0, y: 398.6015625)
                ),
            visibleViewportFrameInPage: sample.visibleViewportFrameInPage,
            contentSize: contentSize,
            displayScale: 3,
            layoutEpoch: sample.layoutEpoch,
            isFollowingTail: true,
            isUserInteracting: false
        )
        XCTAssertNil(
            GaryxConversationOpeningViewportContractPolicy.captureContract(
                for: unalignedSample
            ),
            "the pixel-grid rule defines one exact target; it is not a difference tolerance"
        )
    }

    func testSnapshotRevealRequiresExactLiveTailContractAfterCadenceProof() throws {
        var capture = GaryxConversationOpeningViewportCaptureState(
            requiredStableSampleCount: 1
        )
        let capturedTail = openingSample(offsetY: 2_400)
        let contract = try XCTUnwrap(capture.observe(capturedTail))
        let changedContentGeometry = openingSample(
            offsetY: 2_500,
            contentHeight: 3_300
        )
        XCTAssertEqual(
            GaryxConversationOpeningViewportContractPolicy.revealReadiness(
                for: contract,
                live: changedContentGeometry,
                revisionMatches: true
            ),
            .pending
        )
        XCTAssertEqual(
            GaryxConversationOpeningViewportContractPolicy.revealReadiness(
                for: contract,
                live: capturedTail,
                revisionMatches: false
            ),
            .pending
        )

        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 2,
            openingViewportTimeoutFrameCount: 5
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.presentedFrame(
            interval: 1.0 / 120.0,
            openingViewportReadiness: .pending
        )
        state.presentedFrame(
            interval: 1.0 / 120.0,
            openingViewportReadiness: .pending
        )
        XCTAssertEqual(state.renderPhase, .materializingConversation)

        let matched =
            GaryxConversationOpeningViewportContractPolicy.revealReadiness(
                for: contract,
                live: capturedTail,
                revisionMatches: true
            )
        XCTAssertEqual(matched, .matched)
        XCTAssertEqual(
            state.presentedFrame(
                interval: 1.0 / 120.0,
                openingViewportReadiness: matched
            ),
            .live
        )
    }

    func testOpeningViewportProofTimeoutCannotRetainCoverForever() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1,
            openingViewportTimeoutFrameCount: 3
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 120.0)

        XCTAssertEqual(
            state.presentedFrame(
                interval: 1.0 / 120.0,
                openingViewportReadiness: .pending
            ),
            .materializingConversation
        )
        XCTAssertEqual(
            state.presentedFrame(
                interval: 1.0 / 120.0,
                openingViewportReadiness: .pending
            ),
            .materializingConversation
        )
        XCTAssertEqual(
            state.presentedFrame(
                interval: 1.0 / 120.0,
                openingViewportReadiness: .pending
            ),
            .live
        )
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
        XCTAssertEqual(
            presentation(state, input: skeletonInput),
            .openingCover(.skeleton)
        )
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
        XCTAssertEqual(
            presentation(state, input: skeletonInput),
            .openingCover(.skeleton)
        )
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
        XCTAssertEqual(
            presentation(state, input: skeletonInput),
            .live(.skeleton)
        )
        XCTAssertFalse(state.needsPresentedFrameClock)
        XCTAssertTrue(state.allowsComposerInteraction)
        XCTAssertTrue(state.allowsTranscriptInteraction)
    }

    func testS1ContentWithoutSnapshotPixelsShortCircuitsDirectlyToLive() {
        var state = GaryxConversationRoutePresentationState()
        state.apply(lifecycle: .active)

        XCTAssertEqual(
            presentation(state, input: contentWithoutPixelsInput),
            .live(.content),
            "an illegal content cover is never part of the visible composition"
        )
        XCTAssertEqual(
            state.reconcileTranscriptPresentation(contentWithoutPixelsInput),
            .live(.content)
        )
        XCTAssertEqual(state.renderPhase, .live)
        XCTAssertTrue(state.mountsLiveTranscript)
        XCTAssertTrue(state.hasBegunContentPreparation)
        XCTAssertTrue(state.hasPresentedLiveTranscript)
        XCTAssertTrue(state.allowsTranscriptInteraction)
        XCTAssertFalse(state.needsPresentedFrameClock)
    }

    func testS1SnapshotPixelsKeepAnOpaqueContentCoverLegal() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)

        XCTAssertEqual(
            state.reconcileTranscriptPresentation(contentWithPixelsInput),
            .openingCover(.snapshotPixels)
        )
        XCTAssertEqual(state.renderPhase, .openingPage)
        XCTAssertEqual(
            state.presentedFrame(interval: 1.0 / 120.0),
            .materializingConversation
        )
        XCTAssertTrue(state.mountsLiveTranscript)
        XCTAssertEqual(
            presentation(state, input: contentWithPixelsInput),
            .openingCover(.snapshotPixels)
        )
        XCTAssertTrue(
            GaryxConversationTranscriptPresentationPolicy.coverIsLegal(
                for: contentWithPixelsInput
            )
        )
    }

    func testS2SkeletonIsTheOnlyTreatmentAcrossTheNormalCoverHandoff() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 2
        )
        state.apply(lifecycle: .active)

        var observed = [presentation(state, input: skeletonInput)]
        state.presentedFrame(interval: 1.0 / 120.0)
        observed.append(presentation(state, input: skeletonInput))
        state.presentedFrame(interval: 1.0 / 120.0)
        observed.append(presentation(state, input: skeletonInput))
        state.presentedFrame(interval: 1.0 / 120.0)
        observed.append(presentation(state, input: skeletonInput))

        XCTAssertEqual(
            observed,
            [
                .openingCover(.skeleton),
                .openingCover(.skeleton),
                .openingCover(.skeleton),
                .live(.skeleton),
            ]
        )
        XCTAssertEqual(observed.map(\.treatment), Array(repeating: .skeleton, count: 4))
        XCTAssertEqual(
            presentation(state, input: contentWithoutPixelsInput),
            .live(.content),
            "the first renderable row atomically replaces the live skeleton"
        )
    }

    func testS3IncrementalLiveFramesStayOnTheContentTreatment() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.presentedFrame(interval: 1.0 / 120.0)
        XCTAssertEqual(state.renderPhase, .live)

        let incrementalInputs = [1, 2, 3].map {
            input(
                localRenderableRowCount: $0,
                hasRenderedSnapshot: true,
                isAwaitingInitialHistory: false,
                hasTranscriptSnapshotPixels: false
            )
        }
        XCTAssertEqual(
            incrementalInputs.map { presentation(state, input: $0) },
            Array(repeating: .live(.content), count: 3)
        )
    }

    func testS4RetryKeepsSkeletonOnlyWhileNoLocalContentExists() {
        var state = GaryxConversationRoutePresentationState(
            terminalOpeningFrameCount: 1,
            materializationFrameCount: 1
        )
        state.apply(lifecycle: .active)
        state.presentedFrame(interval: 1.0 / 120.0)
        state.presentedFrame(interval: 1.0 / 120.0)

        XCTAssertEqual(
            presentation(state, input: skeletonInput),
            .live(.skeleton),
            "a cold-start retry keeps the same skeleton across cover handoff"
        )
        let rowsArrivedDuringRetry = input(
            localRenderableRowCount: 1,
            hasRenderedSnapshot: true,
            isAwaitingInitialHistory: true,
            hasTranscriptSnapshotPixels: false
        )
        XCTAssertEqual(
            presentation(state, input: rowsArrivedDuringRetry),
            .live(.content),
            "retry activity cannot cover locally renderable rows with a skeleton"
        )
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
        XCTAssertEqual(
            presentation(state, input: skeletonInput),
            .live(.skeleton)
        )
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
        XCTAssertEqual(
            presentation(state, input: skeletonInput),
            .live(.skeleton)
        )
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

    private var skeletonInput: GaryxConversationTranscriptPresentationInput {
        input(
            localRenderableRowCount: 0,
            hasRenderedSnapshot: false,
            isAwaitingInitialHistory: true,
            hasTranscriptSnapshotPixels: false
        )
    }

    private var contentWithoutPixelsInput: GaryxConversationTranscriptPresentationInput {
        input(
            localRenderableRowCount: 1,
            hasRenderedSnapshot: true,
            isAwaitingInitialHistory: true,
            hasTranscriptSnapshotPixels: false
        )
    }

    private var contentWithPixelsInput: GaryxConversationTranscriptPresentationInput {
        input(
            localRenderableRowCount: 0,
            hasRenderedSnapshot: false,
            isAwaitingInitialHistory: true,
            hasTranscriptSnapshotPixels: true
        )
    }

    private func input(
        localRenderableRowCount: Int,
        hasRenderedSnapshot: Bool,
        isAwaitingInitialHistory: Bool,
        hasTranscriptSnapshotPixels: Bool
    ) -> GaryxConversationTranscriptPresentationInput {
        GaryxConversationTranscriptPresentationInput(
            treatment: GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: localRenderableRowCount,
                hasRenderedSnapshot: hasRenderedSnapshot,
                hasTranscriptSnapshotPixels: hasTranscriptSnapshotPixels,
                isAwaitingInitialHistory: isAwaitingInitialHistory
            ),
            openingViewportContractID:
                hasTranscriptSnapshotPixels ? "presentation-test-contract" : nil
        )
    }

    private func presentation(
        _ state: GaryxConversationRoutePresentationState,
        input: GaryxConversationTranscriptPresentationInput
    ) -> GaryxConversationTranscriptPresentation {
        GaryxConversationTranscriptPresentationPolicy.presentation(
            renderPhase: state.renderPhase,
            input: input
        )
    }

    private func openingSample(
        offsetY: CGFloat,
        contentHeight: CGFloat = 3_200,
        isFollowingTail: Bool = true,
        isUserInteracting: Bool = false
    ) -> GaryxConversationOpeningViewportSample {
        let viewport = CGRect(x: 0, y: 120, width: 440, height: 800)
        return GaryxConversationOpeningViewportSample(
            captureGeometry: GaryxConversationTranscriptSnapshotCaptureGeometry(
                viewportFrameInPage: viewport,
                adjustedContentInsets: .init(top: 0, left: 0, bottom: 0, right: 0),
                contentOffset: CGPoint(x: 0, y: offsetY)
            ),
            visibleViewportFrameInPage: viewport,
            contentSize: CGSize(width: viewport.width, height: contentHeight),
            displayScale: 3,
            layoutEpoch: 9,
            isFollowingTail: isFollowingTail,
            isUserInteracting: isUserInteracting
        )
    }
}
