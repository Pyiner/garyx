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

    func testHistoryPrefetchRequiresMovementAndProximity() {
        var state = GaryxConversationScrollState()
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false,
                ignoreDistance: true
            )
        )

        simulateUserScroll(&state)
        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertTrue(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false,
                ignoreDistance: true
            )
        )
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: false,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false,
                ignoreDistance: true
            )
        )
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: true,
                hasPendingPrefetch: false,
                ignoreDistance: true
            )
        )
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: true,
                ignoreDistance: true
            )
        )

        // Distance gate: browsing metrics put the loaded start 2000pt above
        // the viewport, beyond the 1.5x viewport prefetch distance.
        XCTAssertFalse(
            state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: true,
                isLoadingOlderHistory: false,
                hasPendingPrefetch: false,
                ignoreDistance: false
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
