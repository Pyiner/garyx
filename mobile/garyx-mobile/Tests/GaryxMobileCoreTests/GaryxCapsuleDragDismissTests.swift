import XCTest
@testable import GaryxMobileCore

/// #TASK-1470 — the iOS capsule full-screen detail gains a Photos-style
/// pull-to-dismiss that must coexist with the inner web view's scroll. These
/// guard the pure decision logic: engage only from the top + downward, lock the
/// decision for the whole drag, and dismiss on threshold or predicted flick.
final class GaryxCapsuleDragDismissTests: XCTestCase {
    func testEngagesOnlyWhenAtTopAndDraggingDown() {
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.decideInitialPhase(atTop: true, translationY: 10),
            .engaged
        )
    }

    func testIgnoresWhenNotAtTop() {
        // A drag started while the page is scrolled down belongs to web scroll,
        // even if it is downward.
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.decideInitialPhase(atTop: false, translationY: 40),
            .ignored
        )
    }

    func testIgnoresUpwardDragEvenAtTop() {
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.decideInitialPhase(atTop: true, translationY: -30),
            .ignored
        )
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.decideInitialPhase(atTop: false, translationY: -30),
            .ignored
        )
    }

    /// The phase is decided once and the caller locks it; an `.ignored` drag must
    /// never move the content, which is what keeps web scrolling unaffected and
    /// prevents a mid-drag "snap to dismiss" when the page later reaches the top.
    func testIgnoredPhaseNeverOffsetsContent() {
        XCTAssertEqual(GaryxCapsuleDragDismiss.resolvedOffset(phase: .ignored, translationY: 300), 0)
        XCTAssertEqual(GaryxCapsuleDragDismiss.resolvedOffset(phase: .idle, translationY: 300), 0)
    }

    func testEngagedOffsetFollowsDownwardTranslationAndClampsUp() {
        XCTAssertEqual(GaryxCapsuleDragDismiss.resolvedOffset(phase: .engaged, translationY: 80), 80)
        // Overscroll up while engaged clamps to 0 (no upward content jump).
        XCTAssertEqual(GaryxCapsuleDragDismiss.resolvedOffset(phase: .engaged, translationY: -25), 0)
    }

    func testDragProgressIsClampedAndMonotonic() {
        XCTAssertEqual(GaryxCapsuleDragDismiss.dragProgress(offset: 0), 0, accuracy: 0.0001)
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.dragProgress(offset: 120, fullPullDistance: 240),
            0.5,
            accuracy: 0.0001
        )
        XCTAssertEqual(GaryxCapsuleDragDismiss.dragProgress(offset: 600, fullPullDistance: 240), 1)
        // Never negative, even for an over-clamped/odd offset.
        XCTAssertEqual(GaryxCapsuleDragDismiss.dragProgress(offset: -50), 0)
    }

    func testShouldDismissOnOffsetThreshold() {
        XCTAssertTrue(
            GaryxCapsuleDragDismiss.shouldDismiss(phase: .engaged, offset: 130, predictedTranslationY: 130)
        )
        XCTAssertFalse(
            GaryxCapsuleDragDismiss.shouldDismiss(phase: .engaged, offset: 119, predictedTranslationY: 119)
        )
    }

    func testShouldDismissOnPredictedFlickEvenWhenOffsetShort() {
        // A quick flick: small live offset but large predicted end translation.
        XCTAssertTrue(
            GaryxCapsuleDragDismiss.shouldDismiss(phase: .engaged, offset: 40, predictedTranslationY: 300)
        )
    }

    func testNonEngagedPhaseNeverDismisses() {
        XCTAssertFalse(
            GaryxCapsuleDragDismiss.shouldDismiss(phase: .ignored, offset: 999, predictedTranslationY: 999)
        )
        XCTAssertFalse(
            GaryxCapsuleDragDismiss.shouldDismiss(phase: .idle, offset: 999, predictedTranslationY: 999)
        )
    }
}
