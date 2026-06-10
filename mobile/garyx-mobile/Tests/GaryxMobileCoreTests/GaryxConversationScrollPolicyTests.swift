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

    func testTailGrowthFollowsWhileFollowing() {
        var state = GaryxConversationScrollState()
        let request = state.contentChanged(
            isInitialLoad: false,
            isHistoryPrepend: false,
            hasTailContent: true
        )
        XCTAssertEqual(request, .init(reason: .tailUpdate, animated: true))
        XCTAssertFalse(state.showsScrollToBottomButton)
    }

    func testTailGrowthWhileBrowsingShowsButtonInsteadOfScrolling() {
        var state = GaryxConversationScrollState()
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

        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertNil(state.thinkingIndicatorShown())
        XCTAssertTrue(state.showsScrollToBottomButton)
    }

    func testComposerFocusKeepsTailVisibleOnlyWhileFollowing() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)
        XCTAssertEqual(state.composerFocused(), .init(reason: .manual, animated: true))

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

        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertNil(state.bottomChromeChanged())
    }

    func testScrollToBottomTapResumesFollowing() {
        var state = GaryxConversationScrollState()
        _ = state.contentChanged(isInitialLoad: true, isHistoryPrepend: false, hasTailContent: true)
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

        _ = state.metricsChanged(browsingMetrics(), hasTailContent: true)
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 0, reason: .repair))
        XCTAssertFalse(state.shouldRunTailScrollAttempt(index: 1, reason: .repair))
        XCTAssertTrue(state.shouldRunTailScrollAttempt(index: 1, reason: .openingThread))
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
