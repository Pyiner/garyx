import XCTest
@testable import GaryxMobileCore

final class GaryxHistoryPaginationPlannerTests: XCTestCase {
    func testRenderWindowFalseDoesNotClearCachedOlderBoundary() {
        let current = GaryxHistoryPaginationState(
            hasMoreBefore: true,
            nextBeforeIndex: 40
        )
        let cached = GaryxHistoryPaginationState(
            hasMoreBefore: true,
            nextBeforeIndex: 40
        )

        let next = GaryxHistoryPaginationPlanner.applyingRenderWindow(
            GaryxRenderWindow(floorSeq: 101, hasMoreAbove: false),
            current: current,
            cached: cached
        )

        XCTAssertEqual(next, current)
    }

    func testRenderWindowFalseWithoutCachedTruthDoesNotClearCurrentBoundary() {
        let current = GaryxHistoryPaginationState(
            hasMoreBefore: true,
            nextBeforeIndex: 40
        )

        let next = GaryxHistoryPaginationPlanner.applyingRenderWindow(
            GaryxRenderWindow(floorSeq: 101, hasMoreAbove: false),
            current: current,
            cached: nil
        )

        XCTAssertEqual(next, current)
    }

    func testRenderWindowTrueSeedsOlderBoundaryWhenCacheIsEmpty() {
        let next = GaryxHistoryPaginationPlanner.applyingRenderWindow(
            GaryxRenderWindow(floorSeq: 21, hasMoreAbove: true),
            current: .empty,
            cached: nil
        )

        XCTAssertEqual(next, GaryxHistoryPaginationState(hasMoreBefore: true, nextBeforeIndex: 20))
    }

    func testRenderWindowFalseClearsWhenCacheAlsoHasNoOlderBoundary() {
        let current = GaryxHistoryPaginationState(
            hasMoreBefore: true,
            nextBeforeIndex: 40
        )

        let next = GaryxHistoryPaginationPlanner.applyingRenderWindow(
            GaryxRenderWindow(floorSeq: 1, hasMoreAbove: false),
            current: current,
            cached: .empty
        )

        XCTAssertEqual(next, .empty)
    }

    func testLoadedOlderPageIsIdempotentForSameBeforeIndex() {
        let current = GaryxHistoryPaginationState(
            hasMoreBefore: true,
            nextBeforeIndex: 10
        )
        let duplicatePage = GaryxHistoryPaginationPage(
            hasMoreBefore: true,
            nextBeforeIndex: 10,
            oldestLoadedIndex: 11,
            latestPageStartIndex: 11
        )

        let next = GaryxHistoryPaginationPlanner.applyingTranscriptPage(
            duplicatePage,
            current: current,
            preservingLoadedOlderPages: true
        )

        XCTAssertEqual(next, current)
    }
}
