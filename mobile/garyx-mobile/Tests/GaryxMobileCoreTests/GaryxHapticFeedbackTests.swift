import XCTest
@testable import GaryxMobileCore

final class GaryxHapticFeedbackTests: XCTestCase {
    func testSemanticEventsHaveTheExpectedPatternAndPreparationPoint() {
        let expected: [GaryxHapticEvent: GaryxHapticSpecification] = [
            .messageSendCommitted: .init(pattern: .impact(.light), preparationPoint: .touchDown),
            .threadPinChanged: .init(pattern: .selection, preparationPoint: .touchDown),
            .threadFavoriteChanged: .init(pattern: .selection, preparationPoint: .touchDown),
            .capsuleFavoriteChanged: .init(pattern: .selection, preparationPoint: .touchDown),
            .capsuleDismissCommitted: .init(
                pattern: .impact(.medium),
                preparationPoint: .gestureBegan
            ),
            .messageActionMenuPresented: .init(
                pattern: .impact(.light),
                preparationPoint: .gestureBegan
            ),
            .rowSwipeFullyRevealed: .init(
                pattern: .impact(.medium),
                preparationPoint: .gestureBegan
            ),
            .clipboardCopySucceeded: .init(
                pattern: .notification(.success),
                preparationPoint: .touchDown
            ),
            .avatarGenerationSucceeded: .init(
                pattern: .notification(.success),
                preparationPoint: .operationBegan
            ),
            .avatarGenerationFailed: .init(
                pattern: .notification(.error),
                preparationPoint: .operationBegan
            ),
            .drawerVisibilityCommitted: .init(
                pattern: .impact(.light),
                preparationPoint: .gestureBegan
            ),
            .taskTreeVisibilityCommitted: .init(
                pattern: .impact(.light),
                preparationPoint: .gestureBegan
            ),
            .pinnedOrderDropCommitted: .init(
                pattern: .selection,
                preparationPoint: .gestureBegan
            ),
        ]

        XCTAssertEqual(Set(expected.keys), Set(GaryxHapticEvent.allCases))
        for event in GaryxHapticEvent.allCases {
            XCTAssertEqual(event.specification, expected[event], event.rawValue)
        }
    }

    func testOnlyMeaningfulTerminalPatternsAreCatalogued() {
        let patterns = GaryxHapticEvent.allCases.map(\.specification.pattern)

        XCTAssertTrue(patterns.contains(.impact(.light)))
        XCTAssertTrue(patterns.contains(.impact(.medium)))
        XCTAssertTrue(patterns.contains(.selection))
        XCTAssertTrue(patterns.contains(.notification(.success)))
        XCTAssertTrue(patterns.contains(.notification(.error)))
    }
}
