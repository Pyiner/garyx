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
}
