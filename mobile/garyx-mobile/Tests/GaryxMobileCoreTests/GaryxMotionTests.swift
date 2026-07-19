import XCTest
@testable import GaryxMobileCore

final class GaryxMotionTests: XCTestCase {
    func testEveryOvershootingCurveIsGestureReleaseOnly() {
        for token in GaryxMotion.Token.allCases {
            let specification = GaryxMotion.specification(for: token)
            guard specification.curve.hasOvershootPotential else { continue }
            XCTAssertEqual(
                specification.kinetics,
                .gestureRelease,
                "\(token.rawValue) must not bounce without gesture momentum"
            )
        }
    }

    func testStateDrivenSpringTokensAreCriticallyDamped() {
        let expectations: [(GaryxMotion.Token, TimeInterval)] = [
            (.morphOpen, 0.42),
            (.morphClose, 0.32),
            (.snapBack, 0.22),
            (.cancelSnapBack, 0.28),
            (.composerPayload, 0.24),
            (.composerPanel, 0.22),
            (.imageZoom, 0.28),
        ]

        for (token, response) in expectations {
            let curve = GaryxMotion.springCurve(for: token)
            XCTAssertEqual(curve.response, response)
            XCTAssertEqual(curve.dampingRatio, 1)
        }
    }

    func testMomentumTokensPreserveCalibratedReleaseCurves() {
        let expectations: [(GaryxMotion.Token, TimeInterval, Double)] = [
            (.settle, 0.22, 0.88),
            (.rowSwipe, 0.22, 0.88),
            (.momentumSnapBack, 0.34, 0.82),
        ]

        for (token, response, dampingRatio) in expectations {
            let curve = GaryxMotion.springCurve(for: token)
            XCTAssertEqual(curve.response, response)
            XCTAssertEqual(curve.dampingRatio, dampingRatio)
            XCTAssertEqual(GaryxMotion.specification(for: token).kinetics, .gestureRelease)
        }
    }

    func testAccessibilityResolutionOwnsAnimationAndSpatialEffects() {
        let cases: [(
            preferences: GaryxMotion.Preferences,
            mode: GaryxAccessibilityTransitionMode,
            animates: Bool,
            scale: Double,
            offsetX: Double
        )] = [
            (.standard, .spatial, true, 0.98, 18),
            (
                .init(reduceMotion: false, prefersCrossFadeTransitions: true),
                .crossFade,
                true,
                1,
                0
            ),
            (
                .init(reduceMotion: true, prefersCrossFadeTransitions: false),
                .immediate,
                false,
                1,
                0
            ),
            (
                .init(reduceMotion: true, prefersCrossFadeTransitions: true),
                .crossFade,
                true,
                1,
                0
            ),
        ]

        for item in cases {
            let resolution = GaryxMotion.resolve(.rowRemoval, preferences: item.preferences)
            XCTAssertEqual(resolution.mode, item.mode)
            XCTAssertEqual(resolution.animates, item.animates)
            XCTAssertEqual(resolution.effect.scale, item.scale)
            XCTAssertEqual(resolution.effect.offsetX, item.offsetX)
            XCTAssertEqual(resolution.effect.opacity, 0)
        }
    }

    func testCrossFadeCanUseDedicatedCurveWithoutLeakingSpatialOffset() {
        let resolution = GaryxMotion.resolve(
            .runtimeDrilldownEnter,
            preferences: .init(
                reduceMotion: true,
                prefersCrossFadeTransitions: true
            )
        )

        XCTAssertEqual(resolution.mode, .crossFade)
        XCTAssertEqual(resolution.curve, .easeOut(duration: 0.12))
        XCTAssertEqual(resolution.effect, .identity)
    }

    func testPressFeedbackKeepsOpacityWhenReduceMotionRemovesScale() {
        let standard = GaryxMotion.resolve(.press, preferences: .standard)
        XCTAssertEqual(standard.effect.scale, 0.96)
        XCTAssertEqual(standard.effect.opacity, 0.78)
        XCTAssertTrue(standard.animates)

        let reduced = GaryxMotion.resolve(
            .press,
            preferences: .init(
                reduceMotion: true,
                prefersCrossFadeTransitions: false
            )
        )
        XCTAssertEqual(reduced.effect.scale, 1)
        XCTAssertEqual(reduced.effect.opacity, 0.78)
        XCTAssertFalse(reduced.animates)
    }

    func testSpatialResolutionUsesPrimaryCurveWhenCrossFadeCurveDiffers() {
        let specification = GaryxMotion.specification(for: .morphOpen)
        XCTAssertNotEqual(specification.curve, specification.crossFadeCurve)

        let spatial = GaryxMotion.resolve(.morphOpen, preferences: .standard)
        XCTAssertEqual(spatial.mode, .spatial)
        XCTAssertEqual(spatial.curve, specification.curve)

        let crossFade = GaryxMotion.resolve(
            .morphOpen,
            preferences: .init(
                reduceMotion: false,
                prefersCrossFadeTransitions: true
            )
        )
        XCTAssertEqual(crossFade.mode, .crossFade)
        XCTAssertEqual(crossFade.curve, specification.crossFadeCurve)
    }

    func testContinuousPeriodsAreCatalogOwned() {
        XCTAssertEqual(GaryxMotion.specification(for: .loadingShimmer).curve.duration, 2.4)
        XCTAssertEqual(GaryxMotion.specification(for: .thinkingShimmer).curve.duration, 2.6)
        XCTAssertEqual(GaryxMotion.specification(for: .inkSpinner).curve.duration, 1.05)
        XCTAssertEqual(GaryxMotion.specification(for: .runningOrbit).curve.duration, 1.55)
    }
}
