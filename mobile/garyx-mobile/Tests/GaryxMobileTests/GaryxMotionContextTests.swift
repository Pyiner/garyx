import XCTest
@testable import GaryxMobile

final class GaryxMotionContextTests: XCTestCase {
    func testCrossFadeStripsSpatialEffectAndPreservesOpacityFeedback() {
        let context = GaryxMotionContext(
            preferences: .init(
                reduceMotion: false,
                prefersCrossFadeTransitions: true
            )
        )

        XCTAssertEqual(context.scale(.press, active: true), 1)
        XCTAssertEqual(context.offset(.rowRemoval, active: true), .zero)
        XCTAssertEqual(context.opacity(.press, active: true), 0.78)
        XCTAssertTrue(context.animates(.press))
        XCTAssertFalse(context.animatesSpatially(.press))
    }

    func testImmediateModeDisablesAnimations() {
        let context = GaryxMotionContext(
            preferences: .init(
                reduceMotion: true,
                prefersCrossFadeTransitions: false
            )
        )

        XCTAssertFalse(context.animates(.toast))
        XCTAssertNil(context.animation(.toast))
        XCTAssertNil(context.spatialAnimation(.scrollToTail))
    }

    func testContinuousMotionPausesOutsideSpatialMode() {
        XCTAssertFalse(GaryxMotionContext.standard.pausesContinuousMotion(.loadingShimmer))

        let crossFade = GaryxMotionContext(
            preferences: .init(
                reduceMotion: false,
                prefersCrossFadeTransitions: true
            )
        )
        XCTAssertTrue(crossFade.pausesContinuousMotion(.loadingShimmer))
    }
}
