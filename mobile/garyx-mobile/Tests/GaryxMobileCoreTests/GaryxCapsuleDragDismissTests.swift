import XCTest
@testable import GaryxMobileCore

final class GaryxCapsuleDragDismissTests: XCTestCase {
    private let edgeX: CGFloat = 12
    private let outsideX: CGFloat = 25

    func testWaitsForDecisionDistanceThenLocksHorizontalAtLeadingEdge() {
        XCTAssertEqual(classify(x: edgeX, dx: 13, dy: 0, atTop: false), .pending)
        XCTAssertEqual(classify(x: edgeX, dx: 14, dy: 0, atTop: false), .horizontalDismiss)
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.classify(
                currentPhase: .horizontalDismiss,
                startX: edgeX,
                translation: CGSize(width: 15, height: 100),
                webAtTop: true,
                panelPresented: false
            ),
            .horizontalDismiss,
            "an owned drag never changes axis"
        )
    }

    func testOutsideLeadingEdgeHorizontalDragIsIgnored() {
        XCTAssertEqual(classify(x: outsideX, dx: 80, dy: 4, atTop: true), .ignored)
        XCTAssertEqual(classify(x: outsideX, dx: 80, dy: 4, atTop: false), .ignored)
    }

    func testHorizontalRequiresRightwardAndOnePointFiveDominance() {
        XCTAssertEqual(classify(x: edgeX, dx: -80, dy: 2, atTop: true), .verticalDismiss)
        XCTAssertEqual(classify(x: edgeX, dx: 30, dy: 20, atTop: false), .horizontalDismiss)
        XCTAssertEqual(classify(x: edgeX, dx: 29.9, dy: 20, atTop: false), .ignored)
    }

    func testVerticalDismissRequiresTopAndDownwardNonHorizontalIntent() {
        XCTAssertEqual(classify(x: outsideX, dx: 5, dy: 30, atTop: true), .verticalDismiss)
        XCTAssertEqual(classify(x: outsideX, dx: 5, dy: 30, atTop: false), .ignored)
        XCTAssertEqual(classify(x: outsideX, dx: 5, dy: -30, atTop: true), .ignored)
    }

    func testDiagonalCompetitionChoosesExactlyOneAxis() {
        XCTAssertEqual(classify(x: edgeX, dx: 45, dy: 30, atTop: true), .horizontalDismiss)
        XCTAssertEqual(classify(x: edgeX, dx: 44, dy: 30, atTop: true), .verticalDismiss)
    }

    func testPanelPresentationSuppressesBothAxes() {
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.classify(
                startX: edgeX,
                translation: CGSize(width: 100, height: 0),
                webAtTop: true,
                panelPresented: true
            ),
            .ignored
        )
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.classify(
                startX: outsideX,
                translation: CGSize(width: 0, height: 100),
                webAtTop: true,
                panelPresented: true
            ),
            .ignored
        )
    }

    func testResolvedTranslationMovesOnlyOwnedPositiveAxis() {
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.resolvedTranslation(
                phase: .horizontalDismiss,
                translation: CGSize(width: 80, height: 40)
            ),
            CGSize(width: 80, height: 0)
        )
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.resolvedTranslation(
                phase: .verticalDismiss,
                translation: CGSize(width: 80, height: 40)
            ),
            CGSize(width: 0, height: 40)
        )
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.resolvedTranslation(
                phase: .horizontalDismiss,
                translation: CGSize(width: -20, height: 0)
            ),
            .zero
        )
    }

    func testHorizontalThresholdUsesContainerThirdWith260Clamp() {
        XCTAssertEqual(GaryxCapsuleDragDismiss.horizontalDismissThreshold(containerWidth: 390), 130)
        XCTAssertEqual(GaryxCapsuleDragDismiss.horizontalDismissThreshold(containerWidth: 1_200), 260)
        XCTAssertEqual(GaryxCapsuleDragDismiss.horizontalDismissThreshold(containerWidth: 0), 0)
    }

    func testReleaseUsesTranslationAndVelocityProjectionForBothAxes() {
        XCTAssertTrue(
            GaryxCapsuleDragDismiss.shouldDismiss(
                phase: .horizontalDismiss,
                translation: CGSize(width: 60, height: 0),
                velocity: CGSize(width: 400, height: 0),
                containerWidth: 390
            )
        )
        XCTAssertFalse(
            GaryxCapsuleDragDismiss.shouldDismiss(
                phase: .horizontalDismiss,
                translation: CGSize(width: 60, height: 0),
                velocity: CGSize(width: 100, height: 0),
                containerWidth: 390
            )
        )
        XCTAssertTrue(
            GaryxCapsuleDragDismiss.shouldDismiss(
                phase: .verticalDismiss,
                translation: CGSize(width: 0, height: 40),
                velocity: CGSize(width: 0, height: 500),
                containerWidth: 390
            )
        )
    }

    func testReducerCancelResetsOwnedState() {
        var state = GaryxCapsuleDragDismissState()
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.reduce(
                state: &state,
                event: .changed(
                    startX: edgeX,
                    translation: CGSize(width: 80, height: 2),
                    webAtTop: false,
                    panelPresented: false
                )
            ),
            .none
        )
        XCTAssertEqual(state.phase, .horizontalDismiss)
        XCTAssertEqual(state.translation.width, 80)
        XCTAssertEqual(GaryxCapsuleDragDismiss.reduce(state: &state, event: .cancelled), .none)
        XCTAssertEqual(state, GaryxCapsuleDragDismissState())
    }

    func testReducerReleaseDismissesOrSnapsBackAndAlwaysResets() {
        var state = GaryxCapsuleDragDismissState(
            phase: .horizontalDismiss,
            translation: CGSize(width: 130, height: 0)
        )
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.reduce(
                state: &state,
                event: .released(velocity: .zero, containerWidth: 390)
            ),
            .dismiss
        )
        XCTAssertEqual(state, GaryxCapsuleDragDismissState())

        state = GaryxCapsuleDragDismissState(
            phase: .verticalDismiss,
            translation: CGSize(width: 0, height: 30)
        )
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.reduce(
                state: &state,
                event: .released(velocity: .zero, containerWidth: 390)
            ),
            .snapBack
        )
        XCTAssertEqual(state, GaryxCapsuleDragDismissState())
    }

    func testProgressIsClampedForBothAxes() {
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.dragProgress(
                phase: .horizontalDismiss,
                translation: CGSize(width: 120, height: 0)
            ),
            0.5,
            accuracy: 0.0001
        )
        XCTAssertEqual(
            GaryxCapsuleDragDismiss.dragProgress(
                phase: .verticalDismiss,
                translation: CGSize(width: 0, height: 600)
            ),
            1
        )
    }

    private func classify(
        x: CGFloat,
        dx: CGFloat,
        dy: CGFloat,
        atTop: Bool
    ) -> GaryxCapsuleDragPhase {
        GaryxCapsuleDragDismiss.classify(
            startX: x,
            translation: CGSize(width: dx, height: dy),
            webAtTop: atTop,
            panelPresented: false
        )
    }
}
