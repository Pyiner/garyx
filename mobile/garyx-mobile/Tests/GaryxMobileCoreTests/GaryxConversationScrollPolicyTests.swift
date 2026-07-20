import XCTest
@testable import GaryxMobileCore

final class GaryxConversationLayoutMetricsTests: XCTestCase {
    func testContentShorterThanViewportCountsAsNearBottom() {
        let metrics = GaryxConversationLayoutMetrics(
            contentTopOffset: 0,
            contentBottomOffset: 300,
            viewportHeight: 800
        )
        XCTAssertTrue(metrics.isNearBottom)
        XCTAssertFalse(metrics.hasVisibleTailGap)
    }

    func testUnmeasuredViewportCountsAsNearBottom() {
        XCTAssertTrue(GaryxConversationLayoutMetrics().isNearBottom)
    }

    func testScrolledUpContentIsNotNearBottom() {
        let metrics = GaryxConversationLayoutMetrics(
            contentTopOffset: -2_000,
            contentBottomOffset: 1_400,
            viewportHeight: 800
        )
        XCTAssertFalse(metrics.isNearBottom)
    }

    func testTailWithinThresholdIsNearBottom() {
        let metrics = GaryxConversationLayoutMetrics(
            contentTopOffset: -2_000,
            contentBottomOffset: 880,
            viewportHeight: 800
        )
        XCTAssertTrue(metrics.isNearBottom)
    }

    func testVisibleTailGapRequiresScrolledContentAndRaisedBottom() {
        let gap = GaryxConversationLayoutMetrics(
            contentTopOffset: -400,
            contentBottomOffset: 500,
            viewportHeight: 800
        )
        XCTAssertTrue(gap.hasVisibleTailGap)

        let topAligned = GaryxConversationLayoutMetrics(
            contentTopOffset: 0,
            contentBottomOffset: 500,
            viewportHeight: 800
        )
        XCTAssertFalse(topAligned.hasVisibleTailGap)

        let unmeasured = GaryxConversationLayoutMetrics(
            contentTopOffset: nil,
            contentBottomOffset: 500,
            viewportHeight: 800
        )
        XCTAssertFalse(unmeasured.hasVisibleTailGap)
    }

    func testNearLoadedHistoryStartUsesViewportScaledDistance() {
        var metrics = GaryxConversationLayoutMetrics(
            contentTopOffset: -900,
            contentBottomOffset: 5_000,
            viewportHeight: 800
        )
        XCTAssertTrue(metrics.isNearLoadedHistoryStart)

        metrics.contentTopOffset = -1_300
        XCTAssertFalse(metrics.isNearLoadedHistoryStart)

        metrics.contentTopOffset = nil
        XCTAssertFalse(metrics.isNearLoadedHistoryStart)
    }

    func testPulledPastTopRequiresIntentThreshold() {
        var metrics = GaryxConversationLayoutMetrics(
            contentTopOffset: 40,
            contentBottomOffset: 340,
            viewportHeight: 800
        )
        XCTAssertTrue(metrics.isPulledPastTop)

        // Sub-threshold jitter is not a pull.
        metrics.contentTopOffset = 10
        XCTAssertFalse(metrics.isPulledPastTop)

        // Resting top-aligned content is not a pull.
        metrics.contentTopOffset = 0
        XCTAssertFalse(metrics.isPulledPastTop)

        metrics.contentTopOffset = nil
        XCTAssertFalse(metrics.isPulledPastTop)

        metrics.contentTopOffset = 40
        metrics.viewportHeight = 0
        XCTAssertFalse(metrics.isPulledPastTop)
    }
}

final class GaryxConversationScrollStateTests: XCTestCase {
    /// The position-based browsing flip only happens after a real reader
    /// gesture; tests that browse history must first simulate one.
    private func simulateUserScroll(_ state: inout GaryxConversationScrollState) {
        _ = state.userScrollInteractionChanged(isInteracting: true)
        _ = state.userScrollInteractionChanged(isInteracting: false)
    }

    private func browsingMetrics() -> GaryxConversationLayoutMetrics {
        GaryxConversationLayoutMetrics(
            contentTopOffset: -2_000,
            contentBottomOffset: 1_600,
            viewportHeight: 800
        )
    }

    private func tailMetrics() -> GaryxConversationLayoutMetrics {
        GaryxConversationLayoutMetrics(
            contentTopOffset: -2_000,
            contentBottomOffset: 820,
            viewportHeight: 800
        )
    }

    /// Content pulled past the bottom: still near the bottom (following),
    /// but with a visible gap between the content end and the viewport.
    private func tailGapMetrics() -> GaryxConversationLayoutMetrics {
        GaryxConversationLayoutMetrics(
            contentTopOffset: -2_000,
            contentBottomOffset: 500,
            viewportHeight: 800
        )
    }

    func testThreadOpenedResetsAndJumpsToTail() {
        var state = GaryxConversationScrollState()
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        _ = state.contentChanged(isInitialLoad: false, isHistoryPrepend: false, hasTailContent: true)

        let request = state.threadOpened()
        XCTAssertEqual(request, .init(reason: .openingThread, animated: false))
        XCTAssertTrue(state.isFollowingTail)
        XCTAssertFalse(state.showsScrollToBottomButton)
        XCTAssertFalse(state.hasMovedTowardOlderHistory)
    }

    func testThreadOpenedKeepsMeasuredViewport() {
        var state = GaryxConversationScrollState()
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)

        _ = state.threadOpened()
        XCTAssertEqual(state.metrics.viewportHeight, browsingMetrics().viewportHeight)
        XCTAssertNil(state.metrics.contentTopOffset)
        XCTAssertEqual(state.metrics.contentBottomOffset, 0)
    }

    func testInitialLoadJumpsToTailWithoutAnimation() {
        var state = GaryxConversationScrollState()
        let request = state.contentChanged(
            isInitialLoad: true,
            isHistoryPrepend: false,
            hasTailContent: true
        )
        XCTAssertEqual(request, .init(reason: .openingThread, animated: false))
    }

    func testTailGrowthFollowsWhileFollowingWithoutAnimatedScroll() {
        var state = GaryxConversationScrollState()
        let request = state.contentChanged(
            isInitialLoad: false,
            isHistoryPrepend: false,
            hasTailContent: true
        )
        XCTAssertEqual(request, .init(reason: .tailUpdate, animated: false))
        XCTAssertFalse(state.showsScrollToBottomButton)
    }

    func testTailGrowthWhileBrowsingShowsButtonInsteadOfScrolling() {
        var state = GaryxConversationScrollState()
        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)

        let request = state.contentChanged(
            isInitialLoad: false,
            isHistoryPrepend: false,
            hasTailContent: true
        )
        XCTAssertNil(request)
        XCTAssertTrue(state.showsScrollToBottomButton)
    }

    func testHistoryPrependNeverScrolls() {
        var state = GaryxConversationScrollState()
        let request = state.contentChanged(
            isInitialLoad: false,
            isHistoryPrepend: true,
            hasTailContent: true
        )
        XCTAssertNil(request)
    }

    func testEmptyTranscriptProducesNoScrollAndNoButton() {
        var state = GaryxConversationScrollState()
        let request = state.contentChanged(
            isInitialLoad: true,
            isHistoryPrepend: false,
            hasTailContent: false
        )
        XCTAssertNil(request)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: false)
        XCTAssertFalse(state.showsScrollToBottomButton)
    }

    func testMetricsDriveAnchoringTransitions() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertEqual(state.anchoring, .browsingHistory)
        XCTAssertTrue(state.hasMovedTowardOlderHistory)
        XCTAssertTrue(state.showsScrollToBottomButton)

        _ = state.metricsChanged(tailMetrics(), hasTailContent: true)
        XCTAssertEqual(state.anchoring, .followingTail)
        XCTAssertFalse(state.showsScrollToBottomButton)
        XCTAssertTrue(state.hasMovedTowardOlderHistory)
    }

    func testVisibleTailGapWhileFollowingRequestsRepair() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        let request = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -400,
                contentBottomOffset: 500,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertEqual(request, .init(reason: .repair, animated: false))
    }

    func testThinkingIndicatorFollowsOnlyWhileFollowing() {
        var state = GaryxConversationScrollState()
        XCTAssertEqual(
            state.thinkingIndicatorShown(),
            .init(reason: .tailUpdate, animated: false)
        )

        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertNil(state.thinkingIndicatorShown())
        XCTAssertTrue(state.showsScrollToBottomButton)
    }

    func testComposerFocusKeepsTailVisibleOnlyWhileFollowing() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)
        XCTAssertEqual(state.composerFocused(), .init(reason: .manual, animated: true))

        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertNil(state.composerFocused())
    }

    func testComposerFocusWithoutContentDoesNotScroll() {
        var state = GaryxConversationScrollState()
        XCTAssertNil(state.composerFocused())
    }

    func testBottomChromeChangeRepairsOnlyWhileFollowing() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)
        XCTAssertEqual(state.bottomChromeChanged(), .init(reason: .repair, animated: false))

        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertNil(state.bottomChromeChanged())
    }

    func testScrollToBottomTapResumesFollowing() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)
        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertTrue(state.showsScrollToBottomButton)

        let request = state.scrollToBottomTapped()
        XCTAssertEqual(request, .init(reason: .manual, animated: false))
        XCTAssertTrue(state.isFollowingTail)
        XCTAssertFalse(state.showsScrollToBottomButton)
    }

    func testRepairRetriesStopAfterReaderLeavesTail() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 0, reason: .repair))
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 1, reason: .repair))

        // Before the first gesture, repair retries keep chasing late layout
        // settling even while the measured position is far from the bottom.
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 1, reason: .repair))

        // Once the reader scrolls away themselves, retries stop.
        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 0, reason: .repair))
        XCTAssertFalse(state.shouldRunTailScrollAttempt(index: 1, reason: .repair))
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 1, reason: .openingThread))
    }

    func testTailUpdateRetriesStopAfterReaderLeavesTail() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 1, reason: .tailUpdate))

        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 0, reason: .tailUpdate))
        XCTAssertFalse(state.shouldRunTailScrollAttempt(index: 1, reason: .tailUpdate))
    }

    func testCrossScopeMessageTailUpdateCannotCancelOpeningRetryChain() throws {
        var state = GaryxConversationScrollState()
        var scheduler = GaryxConversationTailScrollScheduler()
        let opening = scheduler.schedule(reason: state.threadOpened().reason)
        let switchUpdate = state.messagesChanged(
            previousIds: ["history:5"],
            currentIds: ["history:5"],
            previousScopeIdentity: "thread:a",
            currentScopeIdentity: "thread:b",
            hasTailContent: true
        )

        XCTAssertEqual(switchUpdate?.reason, .tailUpdate)
        let switchToken = scheduler.schedule(reason: try XCTUnwrap(switchUpdate).reason)

        XCTAssertTrue(
            scheduler.isCurrent(opening),
            "A switch callback must not truncate the opening chain's late settling retries."
        )
        XCTAssertTrue(scheduler.isCurrent(switchToken))
    }

    func testCachedThinkingTailUpdateCannotCancelOpeningRetryChain() throws {
        var state = GaryxConversationScrollState()
        var scheduler = GaryxConversationTailScrollScheduler()
        let opening = scheduler.schedule(reason: state.threadOpened().reason)
        let thinkingReveal = try XCTUnwrap(state.thinkingIndicatorShown())

        XCTAssertEqual(thinkingReveal.reason, .tailUpdate)
        let thinkingToken = scheduler.schedule(reason: thinkingReveal.reason)

        XCTAssertTrue(
            scheduler.isCurrent(opening),
            "A cached thinking reveal must not truncate the opening chain's late settling retries."
        )
        XCTAssertTrue(scheduler.isCurrent(thinkingToken))
    }

    func testTailScrollSchedulerCoalescesWithinHorizonAndLongChainSupersedesAll() {
        var scheduler = GaryxConversationTailScrollScheduler()
        let opening = scheduler.schedule(reason: .openingThread)
        let firstTailUpdate = scheduler.schedule(reason: .tailUpdate)
        let latestTailUpdate = scheduler.schedule(reason: .tailUpdate)

        XCTAssertTrue(scheduler.isCurrent(opening))
        XCTAssertFalse(scheduler.isCurrent(firstTailUpdate))
        XCTAssertTrue(scheduler.isCurrent(latestTailUpdate))

        let repair = scheduler.schedule(reason: .repair)
        XCTAssertFalse(scheduler.isCurrent(opening))
        XCTAssertFalse(scheduler.isCurrent(latestTailUpdate))
        XCTAssertTrue(scheduler.isCurrent(repair))
    }

    func testPersistentTailGapRepairsOnlyOnRisingEdge() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        XCTAssertEqual(
            state.metricsChanged(tailGapMetrics(), hasTailContent: true),
            .init(reason: .repair, animated: false)
        )
        // The gap persists (lazy layout estimation could not close it):
        // later frames must not regenerate the repair, or the reader can
        // never scroll away.
        XCTAssertNil(state.metricsChanged(tailGapMetrics(), hasTailContent: true))
        XCTAssertNil(state.metricsChanged(tailGapMetrics(), hasTailContent: true))

        // Once the gap closes and later reappears, the repair fires again.
        _ = state.metricsChanged(tailMetrics(), hasTailContent: true)
        XCTAssertEqual(
            state.metricsChanged(tailGapMetrics(), hasTailContent: true),
            .init(reason: .repair, animated: false)
        )
    }

    func testNoProgrammaticScrollWhileUserGestureIsActive() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        XCTAssertNil(state.userScrollInteractionChanged(isInteracting: true))
        XCTAssertNil(state.metricsChanged(tailGapMetrics(), hasTailContent: true))
        XCTAssertFalse(state.shouldRunTailScrollAttempt(index: 0, reason: .repair))
        XCTAssertFalse(state.shouldRunTailScrollAttempt(index: 0, reason: .tailUpdate))
        XCTAssertFalse(state.shouldRunTailScrollAttempt(index: 1, reason: .openingThread))
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 0, reason: .manual))
    }

    func testGestureEndOverTailGapRepairsOnce() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        _ = state.userScrollInteractionChanged(isInteracting: true)
        _ = state.metricsChanged(tailGapMetrics(), hasTailContent: true)

        XCTAssertEqual(
            state.userScrollInteractionChanged(isInteracting: false),
            .init(reason: .repair, animated: false)
        )
        // Repeated end events without a new interaction change nothing.
        XCTAssertNil(state.userScrollInteractionChanged(isInteracting: false))
    }

    func testGestureEndAwayFromTailDoesNotScroll() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        _ = state.userScrollInteractionChanged(isInteracting: true)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertNil(state.userScrollInteractionChanged(isInteracting: false))
        XCTAssertTrue(state.showsScrollToBottomButton)
    }

    func testLayoutDriftBeforeFirstScrollRepinsTail() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        // Late layout settling (heavy markdown, async thumbnails) pushed the
        // tail away before the reader ever touched the scroll view: stay
        // following and pull the tail back.
        let request = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertEqual(request, .init(reason: .repair, animated: false))
        XCTAssertTrue(state.isFollowingTail)
        XCTAssertFalse(state.showsScrollToBottomButton)
        XCTAssertFalse(state.hasMovedTowardOlderHistory)
    }

    func testFirstGestureDisablesDriftRepin() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        // While the reader drags away, the viewport is theirs.
        _ = state.userScrollInteractionChanged(isInteracting: true)
        XCTAssertNil(state.metricsChanged(browsingMetrics(), hasTailContent: true))
        XCTAssertEqual(state.anchoring, .browsingHistory)

        // And after the gesture, drifting positions never re-pin again.
        _ = state.userScrollInteractionChanged(isInteracting: false)
        XCTAssertNil(state.metricsChanged(browsingMetrics(), hasTailContent: true))
        XCTAssertEqual(state.anchoring, .browsingHistory)
        XCTAssertTrue(state.showsScrollToBottomButton)
    }

    /// The two edge emitters (top sentinel, bottom anchor) contribute halves
    /// of one preference value; merge must assemble the atomic frame in
    /// either reduce order and let later contributions win their side.
    func testContentEdgesMergeAssemblesAtomicFrame() {
        let topHalf = GaryxConversationContentEdges(top: -120)
        let bottomHalf = GaryxConversationContentEdges(bottom: 900)

        XCTAssertEqual(
            topHalf.merging(bottomHalf),
            GaryxConversationContentEdges(top: -120, bottom: 900)
        )
        XCTAssertEqual(
            bottomHalf.merging(topHalf),
            GaryxConversationContentEdges(top: -120, bottom: 900)
        )
        // A later contribution for the same side wins.
        XCTAssertEqual(
            topHalf.merging(GaryxConversationContentEdges(top: -80)),
            GaryxConversationContentEdges(top: -80, bottom: nil)
        )
    }

    /// Regression shape for #TASK-2073 P2: the retired split-key adapter
    /// delivered top and bottom through separate callbacks, so every scroll
    /// step showed a phantom content-height change. The stable-layout guard
    /// must keep rejecting that shape — atomic frames are the only way
    /// upward travel may accumulate, which is exactly why the view merges
    /// both edges into one preference value.
    func testSplitEdgeDeliveryNeverAccumulatesReaderTravel() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        var top: CGFloat = -2_200
        var bottom: CGFloat = 820
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: top,
                contentBottomOffset: bottom,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        // Scroll up 10pt per step, but deliver top first and bottom second
        // as two separate metrics updates (the old two-key adapter shape).
        for _ in 1...40 {
            top += 10
            _ = state.metricsChanged(
                GaryxConversationLayoutMetrics(
                    contentTopOffset: top,
                    contentBottomOffset: bottom,
                    viewportHeight: 800
                ),
                hasTailContent: true
            )
            bottom += 10
            _ = state.metricsChanged(
                GaryxConversationLayoutMetrics(
                    contentTopOffset: top,
                    contentBottomOffset: bottom,
                    viewportHeight: 800
                ),
                hasTailContent: true
            )
        }
        XCTAssertFalse(
            state.hasUserScrolledSinceOpen,
            "Half-updated frames show phantom height changes and must never count as reader travel."
        )
    }

    /// Pre-iOS 18 there is no scroll-phase API: sustained upward movement
    /// across stable-layout frames must count as the reader's scroll, flip
    /// the anchoring, and arm automatic history paging.
    func testUpwardReadingTravelActsAsReaderScrollWithoutPhaseEvents() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        // At the tail; content height stays 3020 through every frame.
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -2_200,
                contentBottomOffset: 820,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertFalse(state.hasUserScrolledSinceOpen)

        // Slow upward drag in 10pt steps: same layout, top offset rising.
        for step in 1...12 {
            _ = state.metricsChanged(
                GaryxConversationLayoutMetrics(
                    contentTopOffset: -2_200 + CGFloat(step) * 10,
                    contentBottomOffset: 820 + CGFloat(step) * 10,
                    viewportHeight: 800
                ),
                hasTailContent: true
            )
        }
        XCTAssertTrue(state.hasUserScrolledSinceOpen)
        XCTAssertEqual(state.anchoring, .browsingHistory)
        XCTAssertTrue(state.hasMovedTowardOlderHistory)

        // Continue to the loaded start: automatic paging fires.
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -400,
                contentBottomOffset: 2_620,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertTrue(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )
    }

    /// Layout settling (streaming markdown, async thumbnails) grows the
    /// content height, so it must never masquerade as reader travel — the
    /// pre-first-gesture drift repin has to stay armed.
    func testLayoutSettlingDoesNotCountAsReaderTravel() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -100,
                contentBottomOffset: 900,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        // Tail keeps growing while the top edge also shifts up: height changes,
        // so no travel accumulates.
        let request = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -60,
                contentBottomOffset: 1_400,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertFalse(state.hasUserScrolledSinceOpen)
        XCTAssertFalse(state.hasMovedTowardOlderHistory)
        XCTAssertEqual(request, .init(reason: .repair, animated: false))
    }

    /// Keyboard / bottom chrome changes resize the viewport; top-offset
    /// movement caused by them must not count as reader travel.
    func testViewportResizeDoesNotCountAsReaderTravel() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -300,
                contentBottomOffset: 700,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -260,
                contentBottomOffset: 740,
                viewportHeight: 500
            ),
            hasTailContent: true
        )
        XCTAssertFalse(state.hasUserScrolledSinceOpen)
    }

    /// Downward movement (including programmatic tail repairs) resets the
    /// accumulator: travel must be sustained, not netted across reversals.
    func testDownwardMovementResetsUpwardTravel() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        var top: CGFloat = -2_200
        func frame(_ delta: CGFloat) {
            top += delta
            _ = state.metricsChanged(
                GaryxConversationLayoutMetrics(
                    contentTopOffset: top,
                    contentBottomOffset: top + 3_020,
                    viewportHeight: 800
                ),
                hasTailContent: true
            )
        }
        frame(0)
        frame(16)
        frame(-6)
        frame(16)
        XCTAssertFalse(state.hasUserScrolledSinceOpen)
        frame(16)
        XCTAssertTrue(state.hasUserScrolledSinceOpen)
    }

    func testHistoryPrefetchRequiresMovementAndProximity() {
        var state = GaryxConversationScrollState()
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )

        simulateUserScroll(&state)
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -400,
                contentBottomOffset: 2_600,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertTrue(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: false,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: true,
                hasPendingPrefetch: false
            )
        )
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: true
            )
        )

        // Distance gate: browsing metrics put the loaded start 2000pt above
        // the viewport, beyond the 1.5x viewport prefetch distance.
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )
    }

    func testSmallWindowAutoPagesAfterScrollTowardHistory() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)
        _ = state.userScrollInteractionChanged(isInteracting: true)
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -80,
                contentBottomOffset: 980,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        _ = state.userScrollInteractionChanged(isInteracting: false)

        XCTAssertTrue(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            ),
            "Scrolling up in a barely scrollable window is a reach for older history and must auto-page — no manual load button exists."
        )
    }

    func testShortTranscriptTopPullArmsAutomaticHistoryPaging() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        // Content shorter than the viewport can never flip the anchoring to
        // browsing; before the pull, nothing may auto-page.
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: 0,
                contentBottomOffset: 300,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )

        // The reader rubber-bands the top past the intent threshold.
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: 40,
                contentBottomOffset: 340,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertTrue(state.hasMovedTowardOlderHistory)

        // The pull settles back; the armed intent persists and paging fires.
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: 0,
                contentBottomOffset: 300,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertTrue(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )
    }

    /// A cold mount can begin with cached messages before the messages
    /// onChange lifecycle has ever run. A short transcript's top pull still
    /// pages older history while following the tail, so misclassifying the
    /// first prepend as tail growth visibly jumps back to the newest message.
    func testColdMountMessagePrependAfterShortTranscriptTopPullDoesNotRequestTailScroll() {
        var state = GaryxConversationScrollState()

        _ = state.threadOpened()
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: 0,
                contentBottomOffset: 300,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: 40,
                contentBottomOffset: 340,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: 0,
                contentBottomOffset: 300,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertTrue(state.isFollowingTail)
        XCTAssertTrue(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )

        let previousIds = ["history:3", "history:4", "history:5"]
        let currentIds = ["history:0", "history:1", "history:2"] + previousIds
        let request = state.messagesChanged(
            previousIds: previousIds,
            currentIds: currentIds,
            previousScopeIdentity: "thread:a",
            currentScopeIdentity: "thread:a",
            hasTailContent: true
        )

        XCTAssertNil(
            request,
            "An older-page prepend requested by a short top pull must not schedule a tail scroll."
        )
    }

    func testUntouchedThreadNeverAutoPagesFromTopRowAppearance() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)

        // Cold open of a short transcript puts the loaded start on screen
        // immediately (top-row onAppear fires); without any reader gesture
        // no automatic paging may start.
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: 0,
                contentBottomOffset: 300,
                viewportHeight: 800
            ),
            hasTailContent: true
        )
        XCTAssertFalse(state.hasMovedTowardOlderHistory)
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            )
        )
    }

    func testTopRowAppearanceStillHonorsDistanceFromLoadedHistoryStart() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)
        simulateUserScroll(&state)
        _ = state.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -2_000,
                contentBottomOffset: 2_600,
                viewportHeight: 800
            ),
            hasTailContent: true
        )

        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false
            ),
            "Top-row onAppear is a row-materialization signal, not permission to bypass scroll distance."
        )
    }

    func testRenderRowPrependPreservesScrollWhenMessagesDidNotChange() {
        let transcriptMessages = [
            transcriptMessage(index: 0, role: .user, text: "Older question"),
            transcriptMessage(index: 2, role: .user, text: "Current question"),
        ]
        let previousSnapshot = renderSnapshot(
            rows: [
                userTurn(id: "turn:seq3", seq: 3),
            ],
            floorSeq: 3
        )
        let currentSnapshot = renderSnapshot(
            rows: [
                userTurn(id: "turn:seq1", seq: 1),
                userTurn(id: "turn:seq3", seq: 3),
            ],
            floorSeq: 1
        )

        let previousRowIds = GaryxMobileRenderStateMapper.rows(
            snapshot: previousSnapshot,
            messages: [],
            transcriptMessages: transcriptMessages
        ).map(\.id)
        let currentRowIds = GaryxMobileRenderStateMapper.rows(
            snapshot: currentSnapshot,
            messages: [],
            transcriptMessages: transcriptMessages
        ).map(\.id)

        XCTAssertEqual(transcriptMessages.map(\.id), ["history:0", "history:2"])
        XCTAssertEqual(previousRowIds, ["turn:seq3"])
        XCTAssertEqual(currentRowIds, ["turn:seq1", "turn:seq3"])

        var state = GaryxConversationScrollState()
        _ = state.threadOpened()
        let restore = state.renderRowsChanged(
            previousIds: previousRowIds,
            currentIds: currentRowIds,
            previousScopeIdentity: "thread:a",
            currentScopeIdentity: "thread:a",
            hasTailContent: true
        )

        // A prepend must hand the view a reading-anchor restore for the
        // pre-prepend first row — SwiftUI alone keeps the offset relative to
        // the content top, which would park the viewport over the freshly
        // loaded oldest rows.
        XCTAssertEqual(
            restore,
            GaryxConversationScrollState.ReadingAnchorRestore(anchorRowId: "turn:seq3")
        )
        XCTAssertTrue(state.hasTailContent)
    }

    /// #TASK-2088 regression — the exact restore is the anchor row's
    /// displacement in the transcript CONTENT coordinate space. It depends
    /// only on how much height was inserted above the anchor, never on where
    /// the reader was (T = 0, 640pt, or a full prefetch distance away) and
    /// never on concurrent tail growth below the anchor — neither quantity
    /// appears in the formula.
    func testHistoryPrependTopGrowthIsAnchorDisplacement() {
        // A 2400pt network page lays out above the anchor row.
        XCTAssertEqual(
            GaryxConversationScrollState.historyPrependTopGrowth(
                capturedAnchorMinY: 35,
                currentAnchorMinY: 2_435
            ),
            2_400
        )
        // A 60-row in-memory window reveal works the same way.
        XCTAssertEqual(
            GaryxConversationScrollState.historyPrependTopGrowth(
                capturedAnchorMinY: 0,
                currentAnchorMinY: 812.5
            ),
            812.5
        )
    }

    /// The restore waits for observable growth: missing geometry, unchanged
    /// layout, and upward movement (not a prepend) all return nil so the
    /// caller retries or falls back instead of teleporting the reader.
    func testHistoryPrependTopGrowthRequiresObservableGrowth() {
        XCTAssertNil(
            GaryxConversationScrollState.historyPrependTopGrowth(
                capturedAnchorMinY: nil,
                currentAnchorMinY: 2_435
            )
        )
        XCTAssertNil(
            GaryxConversationScrollState.historyPrependTopGrowth(
                capturedAnchorMinY: 35,
                currentAnchorMinY: nil
            )
        )
        XCTAssertNil(
            GaryxConversationScrollState.historyPrependTopGrowth(
                capturedAnchorMinY: 35,
                currentAnchorMinY: 35
            )
        )
        XCTAssertNil(
            GaryxConversationScrollState.historyPrependTopGrowth(
                capturedAnchorMinY: 35,
                currentAnchorMinY: 35.4
            )
        )
        XCTAssertNil(
            GaryxConversationScrollState.historyPrependTopGrowth(
                capturedAnchorMinY: 2_435,
                currentAnchorMinY: 35
            )
        )
    }

    /// Only a genuine prepend restores the reading anchor: tail appends,
    /// unchanged rows, and thread switches must not scroll anywhere.
    func testRenderRowsChangedOnlyRestoresForGenuinePrepends() {
        var state = GaryxConversationScrollState()
        _ = state.threadOpened()

        // Tail append: no restore.
        XCTAssertNil(
            state.renderRowsChanged(
                previousIds: ["a", "b"],
                currentIds: ["a", "b", "c"],
                previousScopeIdentity: "thread:a",
                currentScopeIdentity: "thread:a",
                hasTailContent: true
            )
        )
        // Unchanged rows: no restore.
        XCTAssertNil(
            state.renderRowsChanged(
                previousIds: ["a", "b"],
                currentIds: ["a", "b"],
                previousScopeIdentity: "thread:a",
                currentScopeIdentity: "thread:a",
                hasTailContent: true
            )
        )
        // Thread switch: another thread's rows replay the same ids (row ids are
        // message-reference based, so "a" recurs at index 1), yet a
        // cross-thread change must never restore.
        XCTAssertNil(
            state.renderRowsChanged(
                previousIds: ["a", "b"],
                currentIds: ["x", "a", "b"],
                previousScopeIdentity: "thread:a",
                currentScopeIdentity: "thread:b",
                hasTailContent: true
            )
        )
        // Genuine prepend within the now-current thread anchors to the
        // pre-prepend first row, and the anchoring state is untouched (restore
        // is not a reader gesture).
        let restore = state.renderRowsChanged(
            previousIds: ["a", "b"],
            currentIds: ["x", "y", "a", "b"],
            previousScopeIdentity: "thread:b",
            currentScopeIdentity: "thread:b",
            hasTailContent: true
        )
        XCTAssertEqual(restore?.anchorRowId, "a")
        XCTAssertTrue(state.isFollowingTail)
        XCTAssertFalse(state.hasUserScrolledSinceOpen)
    }

    /// #TASK-2488 — a cold mount whose cached render rows predate the view's
    /// first appearance never fires a row-id change before the first real
    /// prepend. The old/new observation scopes must therefore travel with the
    /// row ids themselves; view-local lifecycle memory has never been seeded at
    /// this point and would reject the genuine same-thread prepend.
    func testColdMountCachedRowsPrependArmsRestore() {
        var state = GaryxConversationScrollState()
        // onAppear opens the thread over rows that were already cached before
        // the view first appeared (turns 12...23).
        _ = state.threadOpened()

        // The reader scrolled up from the tail (Response 14 at the viewport
        // top). Anchoring does not gate the restore; this only makes the test
        // faithful to the reproduction.
        _ = state.userScrollInteractionChanged(isInteracting: true)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        _ = state.userScrollInteractionChanged(isInteracting: false)

        // The first real content change is the older-history prepend: 12 cached
        // rows (turns 12...23) grow to 24 (turns 0...23), all in the same
        // thread.
        let cached = (12...23).map { "user_turn:history:\($0)" }
        let prepended = (0...23).map { "user_turn:history:\($0)" }
        let restore = state.renderRowsChanged(
            previousIds: cached,
            currentIds: prepended,
            previousScopeIdentity: "thread:a",
            currentScopeIdentity: "thread:a",
            hasTailContent: true
        )
        XCTAssertEqual(
            restore,
            GaryxConversationScrollState.ReadingAnchorRestore(
                anchorRowId: "user_turn:history:12"
            ),
            "A cold-mount prepend must arm the reading-anchor restore for the pre-prepend first row."
        )
    }

    /// The thread-switch guard must not depend on whether the open event or the
    /// row change is delivered first. The old/new scope identities are an
    /// atomic part of the row observation, so a cross-thread row replay is
    /// rejected directly in either ordering (#TASK-2488).
    func testThreadSwitchRejectsRestoreRegardlessOfEventOrder() {
        func openAndSettleThreadA() -> GaryxConversationScrollState {
            var state = GaryxConversationScrollState()
            _ = state.threadOpened()
            _ = state.renderRowsChanged(
                previousIds: [],
                currentIds: ["user_turn:history:5"],
                previousScopeIdentity: "thread:a",
                currentScopeIdentity: "thread:a",
                hasTailContent: true
            )
            return state
        }

        // Order 1: the open(thread-b) event lands before the thread-a→b rows.
        var openFirst = openAndSettleThreadA()
        _ = openFirst.threadOpened()
        XCTAssertNil(
            openFirst.renderRowsChanged(
                previousIds: ["user_turn:history:5"],
                currentIds: ["user_turn:history:1", "user_turn:history:5"],
                previousScopeIdentity: "thread:a",
                currentScopeIdentity: "thread:b",
                hasTailContent: true
            ),
            "open-before-rows switch must not restore across threads"
        )

        // Order 2: the thread-a→b row change lands before open(thread-b).
        var rowsFirst = openAndSettleThreadA()
        XCTAssertNil(
            rowsFirst.renderRowsChanged(
                previousIds: ["user_turn:history:5"],
                currentIds: ["user_turn:history:1", "user_turn:history:5"],
                previousScopeIdentity: "thread:a",
                currentScopeIdentity: "thread:b",
                hasTailContent: true
            ),
            "rows-before-open switch must not restore across threads"
        )
        _ = rowsFirst.threadOpened()

        // And once thread-b is the established current thread, a genuine
        // in-thread prepend still restores.
        let restore = rowsFirst.renderRowsChanged(
            previousIds: ["user_turn:history:5"],
            currentIds: ["user_turn:history:2", "user_turn:history:5"],
            previousScopeIdentity: "thread:b",
            currentScopeIdentity: "thread:b",
            hasTailContent: true
        )
        XCTAssertEqual(restore?.anchorRowId, "user_turn:history:5")
    }

    /// Row ids are message-reference based and recur across threads. Including
    /// route scope in the observed value makes an identical-id switch visible;
    /// after that rejected replay, B's first real in-thread prepend restores.
    func testThreadSwitchWithIdenticalCachedRowIdsPreservesFirstInThreadPrepend() {
        var state = GaryxConversationScrollState()
        let cached = ["user_turn:history:5"]
        let threadA = GaryxConversationScrollObservation(
            scopeIdentity: "thread:a",
            value: cached
        )
        let threadB = GaryxConversationScrollObservation(
            scopeIdentity: "thread:b",
            value: cached
        )

        XCTAssertNotEqual(threadA, threadB)

        _ = state.threadOpened()
        XCTAssertNil(
            state.renderRowsChanged(
                previousIds: threadA.value,
                currentIds: threadB.value,
                previousScopeIdentity: threadA.scopeIdentity,
                currentScopeIdentity: threadB.scopeIdentity,
                hasTailContent: true
            ),
            "Identical ids from another thread are replay, never a prepend."
        )
        let restore = state.renderRowsChanged(
            previousIds: threadB.value,
            currentIds: ["user_turn:history:1"] + cached,
            previousScopeIdentity: threadB.scopeIdentity,
            currentScopeIdentity: threadB.scopeIdentity,
            hasTailContent: true
        )
        XCTAssertEqual(
            restore,
            GaryxConversationScrollState.ReadingAnchorRestore(
                anchorRowId: "user_turn:history:5"
            )
        )
    }

    func testPreservesScrollForPrependedHistory() {
        XCTAssertTrue(
            GaryxConversationScrollState.preservesScrollForPrependedHistory(
                previousIds: ["b", "c"],
                currentIds: ["a", "b", "c"],
                threadUnchanged: true
            )
        )
        XCTAssertFalse(
            GaryxConversationScrollState.preservesScrollForPrependedHistory(
                previousIds: ["b", "c"],
                currentIds: ["a", "b", "c"],
                threadUnchanged: false
            )
        )
        XCTAssertFalse(
            GaryxConversationScrollState.preservesScrollForPrependedHistory(
                previousIds: ["b", "c"],
                currentIds: ["b", "c", "d"],
                threadUnchanged: true
            )
        )
        XCTAssertFalse(
            GaryxConversationScrollState.preservesScrollForPrependedHistory(
                previousIds: [],
                currentIds: ["a"],
                threadUnchanged: true
            )
        )
        XCTAssertFalse(
            GaryxConversationScrollState.preservesScrollForPrependedHistory(
                previousIds: ["b", "c"],
                currentIds: ["a", "x"],
                threadUnchanged: true
            )
        )
    }

    private func renderSnapshot(rows: [GaryxRenderRow], floorSeq: Int) -> GaryxRenderSnapshot {
        GaryxRenderSnapshot(
            basedOnSeq: 4,
            rows: rows,
            window: GaryxRenderWindow(floorSeq: floorSeq, hasMoreAbove: floorSeq > 1)
        )
    }

    private func userTurn(id: String, seq: Int) -> GaryxRenderRow {
        .userTurn(GaryxRenderUserTurnRow(
            id: id,
            user: GaryxRenderMessageRef(id: "seq:\(seq)", seq: seq, role: "user"),
            activity: []
        ))
    }

    private func transcriptMessage(
        index: Int,
        role: GaryxTranscriptRole,
        text: String
    ) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(index: index, role: role, text: text)
    }
}

final class GaryxTailThinkingPresentationStateTests: XCTestCase {
    func testThinkingShorterThanDelayNeverBecomesVisible() {
        var state = GaryxTailThinkingPresentationState()
        XCTAssertFalse(state.update(isThinking: true, now: 1.0, delay: 0.2))
        XCTAssertEqual(state.nextVisibilityCheck(now: 1.0, delay: 0.2) ?? -1, 0.2, accuracy: 0.001)

        XCTAssertFalse(state.update(isThinking: false, now: 1.12, delay: 0.2))
        XCTAssertNil(state.nextVisibilityCheck(now: 1.12, delay: 0.2))

        XCTAssertFalse(state.update(isThinking: false, now: 1.25, delay: 0.2))
    }

    func testThinkingLongerThanDelayAppearsThenHidesWhenTextArrives() {
        var state = GaryxTailThinkingPresentationState()
        XCTAssertFalse(state.update(isThinking: true, now: 10.0, delay: 0.2))
        XCTAssertFalse(state.update(isThinking: true, now: 10.19, delay: 0.2))
        XCTAssertTrue(state.update(isThinking: true, now: 10.21, delay: 0.2))
        XCTAssertNil(state.nextVisibilityCheck(now: 10.21, delay: 0.2))

        XCTAssertFalse(state.update(isThinking: false, now: 10.3, delay: 0.2))
    }

    func testThinkingDelayRestartsAfterCancellation() {
        var state = GaryxTailThinkingPresentationState()
        XCTAssertFalse(state.update(isThinking: true, now: 2.0, delay: 0.2))
        XCTAssertFalse(state.update(isThinking: false, now: 2.1, delay: 0.2))

        XCTAssertFalse(state.update(isThinking: true, now: 3.0, delay: 0.2))
        XCTAssertEqual(state.nextVisibilityCheck(now: 3.05, delay: 0.2) ?? -1, 0.15, accuracy: 0.001)
        XCTAssertTrue(state.update(isThinking: true, now: 3.21, delay: 0.2))
    }
}
