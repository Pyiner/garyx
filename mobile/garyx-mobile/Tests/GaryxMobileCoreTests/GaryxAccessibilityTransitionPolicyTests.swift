import XCTest
@testable import GaryxMobileCore

final class GaryxAccessibilityTransitionPolicyTests: XCTestCase {
    func testAccessibilityPreferenceTruthTable() {
        let cases: [(
            reduceMotion: Bool,
            prefersCrossFadeTransitions: Bool,
            usesCrossFade: Bool,
            animatesTransition: Bool
        )] = [
            (false, false, false, true),
            (true, false, true, false),
            (false, true, true, true),
            (true, true, true, true),
        ]

        for item in cases {
            XCTAssertEqual(
                GaryxAccessibilityTransitionPolicy.usesCrossFade(
                    reduceMotion: item.reduceMotion,
                    prefersCrossFadeTransitions: item.prefersCrossFadeTransitions
                ),
                item.usesCrossFade
            )
            XCTAssertEqual(
                GaryxAccessibilityTransitionPolicy.animatesTransition(
                    reduceMotion: item.reduceMotion,
                    prefersCrossFadeTransitions: item.prefersCrossFadeTransitions
                ),
                item.animatesTransition
            )
        }
    }

    func testEscapeUsesTheTerminalVisibleActiveGate() {
        for lifecycle in GaryxRouteHostLifecyclePhase.allCases {
            for isCanonicalTop in [false, true] {
                for hasModal in [false, true] {
                    XCTAssertEqual(
                        GaryxRouteAccessibilityGate.allowsEscape(
                            isCanonicalTop: isCanonicalTop,
                            lifecycle: lifecycle,
                            hasPresentationBarrier: hasModal
                        ),
                        isCanonicalTop && lifecycle == .active && !hasModal
                    )
                }
            }
        }
    }

    func testComposerFocusRequiresInputReadyVisibleAndActive() {
        for inputReady in [false, true] {
            for visible in [false, true] {
                for active in [false, true] {
                    XCTAssertEqual(
                        GaryxRouteAccessibilityGate.allowsComposerFocus(
                            inputReady: inputReady,
                            isVisibleRoute: visible,
                            sceneIsActive: active
                        ),
                        inputReady && visible && active
                    )
                }
            }
        }
    }
}
