import SwiftUI
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

    func testTransitionAdapterResolvesSpatialCrossFadeAndImmediateModes() {
        let spatial = reflectedTransition(
            GaryxMotionContext.standard.transition(.scrollLatest, moveFrom: .top)
        )
        XCTAssertTrue(spatial.contains("OpacityTransition"))
        XCTAssertTrue(spatial.contains("MoveTransition"))
        XCTAssertTrue(spatial.contains("ScaleTransition"))

        let crossFadeContext = GaryxMotionContext(
            preferences: .init(
                reduceMotion: false,
                prefersCrossFadeTransitions: true
            )
        )
        let crossFade = reflectedTransition(
            crossFadeContext.transition(.scrollLatest, moveFrom: .top)
        )
        XCTAssertTrue(crossFade.contains("OpacityTransition"))
        XCTAssertFalse(crossFade.contains("MoveTransition"))
        XCTAssertFalse(crossFade.contains("ScaleTransition"))

        let immediateContext = GaryxMotionContext(
            preferences: .init(
                reduceMotion: true,
                prefersCrossFadeTransitions: false
            )
        )
        let immediate = reflectedTransition(
            immediateContext.transition(.scrollLatest, moveFrom: .top)
        )
        XCTAssertTrue(immediate.contains("IdentityTransition"))
        XCTAssertFalse(immediate.contains("OpacityTransition"))
        XCTAssertFalse(immediate.contains("MoveTransition"))
        XCTAssertFalse(immediate.contains("ScaleTransition"))
    }

    private func reflectedTransition(_ transition: AnyTransition) -> String {
        String(reflecting: transition)
    }
}
