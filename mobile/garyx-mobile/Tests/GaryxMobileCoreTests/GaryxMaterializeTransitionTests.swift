import XCTest
@testable import GaryxMobileCore

final class GaryxMaterializeTransitionTests: XCTestCase {
    func testStandardMotionMaterializesBlurScaleAndOpacityTogether() {
        XCTAssertEqual(
            activeState(preferences: .standard, reduceTransparency: false),
            .init(opacity: 0, scale: 0.965, blurRadius: 12)
        )
    }

    func testReduceTransparencyRemovesBlurButKeepsSpatialArrival() {
        XCTAssertEqual(
            activeState(preferences: .standard, reduceTransparency: true),
            .init(opacity: 0, scale: 0.965, blurRadius: 0)
        )
    }

    func testCrossFadeAlwaysRemovesBlurAndScale() {
        let preferences = GaryxMotion.Preferences(
            reduceMotion: false,
            prefersCrossFadeTransitions: true
        )
        for reduceTransparency in [false, true] {
            XCTAssertEqual(
                activeState(
                    preferences: preferences,
                    reduceTransparency: reduceTransparency
                ),
                .init(opacity: 0, scale: 1, blurRadius: 0)
            )
        }
    }

    func testReduceMotionIsImmediateWithoutCrossFadePreference() {
        let preferences = GaryxMotion.Preferences(
            reduceMotion: true,
            prefersCrossFadeTransitions: false
        )
        for reduceTransparency in [false, true] {
            XCTAssertEqual(
                activeState(
                    preferences: preferences,
                    reduceTransparency: reduceTransparency
                ),
                .identity
            )
        }
    }

    func testReduceMotionStillCrossFadesWhenExplicitlyPreferred() {
        let preferences = GaryxMotion.Preferences(
            reduceMotion: true,
            prefersCrossFadeTransitions: true
        )
        XCTAssertEqual(
            activeState(preferences: preferences, reduceTransparency: false),
            .init(opacity: 0, scale: 1, blurRadius: 0)
        )
    }

    func testInvalidVisualInputsAreClamped() {
        XCTAssertEqual(
            GaryxMaterializeTransitionPolicy.activeState(
                transitionMode: .spatial,
                reduceTransparency: false,
                initialScale: -1,
                initialBlurRadius: -8
            ),
            .init(opacity: 0, scale: 0, blurRadius: 0)
        )
    }

    private func activeState(
        preferences: GaryxMotion.Preferences,
        reduceTransparency: Bool
    ) -> GaryxMaterializeTransitionVisualState {
        let resolution = GaryxMotion.resolve(.threadMenu, preferences: preferences)
        return GaryxMaterializeTransitionPolicy.activeState(
            transitionMode: resolution.mode,
            reduceTransparency: reduceTransparency,
            initialScale: 0.965,
            initialBlurRadius: 12
        )
    }
}
