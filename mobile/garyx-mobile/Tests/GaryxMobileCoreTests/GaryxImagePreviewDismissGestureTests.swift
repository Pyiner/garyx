import XCTest
@testable import GaryxMobileCore

final class GaryxImagePreviewDismissGestureTests: XCTestCase {
    func testWaitsForDecisionDistanceThenLocksDownwardIntent() {
        XCTAssertEqual(classify(dx: 0, dy: 9), .pending)
        XCTAssertEqual(classify(dx: 1, dy: 10), .downwardDismiss)
        XCTAssertEqual(
            GaryxImagePreviewDismissGesture.classify(
                currentPhase: .downwardDismiss,
                translation: CGSize(width: 100, height: 11)
            ),
            .downwardDismiss
        )
    }

    func testHorizontalAndUpwardDragsAreRejectedForPagerOwnership() {
        XCTAssertEqual(classify(dx: 20, dy: 10), .rejected)
        XCTAssertEqual(classify(dx: 0, dy: -20), .rejected)
        XCTAssertFalse(GaryxImagePreviewDismissGesture.isDownwardIntent(CGSize(width: 30, height: 20)))
    }

    func testVisibleOffsetOnlyTracksOwnedDownwardDrag() {
        XCTAssertEqual(
            GaryxImagePreviewDismissGesture.visibleOffset(
                phase: .downwardDismiss,
                translation: CGSize(width: 4, height: 72)
            ),
            72
        )
        XCTAssertEqual(
            GaryxImagePreviewDismissGesture.visibleOffset(
                phase: .rejected,
                translation: CGSize(width: 4, height: 72)
            ),
            0
        )
    }

    func testDismissRequiresDistanceAndOnePointTwoFiveVerticalDominance() {
        XCTAssertFalse(shouldDismiss(dx: 0, dy: 88))
        XCTAssertTrue(shouldDismiss(dx: 0, dy: 89))
        XCTAssertTrue(shouldDismiss(dx: 70, dy: 89))
        XCTAssertFalse(shouldDismiss(dx: 72, dy: 89))
        XCTAssertFalse(
            GaryxImagePreviewDismissGesture.shouldDismiss(
                phase: .rejected,
                translation: CGSize(width: 0, height: 200)
            )
        )
    }

    private func classify(dx: CGFloat, dy: CGFloat) -> GaryxImagePreviewDragPhase {
        GaryxImagePreviewDismissGesture.classify(translation: CGSize(width: dx, height: dy))
    }

    private func shouldDismiss(dx: CGFloat, dy: CGFloat) -> Bool {
        GaryxImagePreviewDismissGesture.shouldDismiss(
            phase: .downwardDismiss,
            translation: CGSize(width: dx, height: dy)
        )
    }
}
