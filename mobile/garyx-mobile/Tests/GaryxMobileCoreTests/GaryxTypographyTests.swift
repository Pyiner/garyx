import XCTest
@testable import GaryxMobileCore

final class GaryxTypographyTests: XCTestCase {
    func testReadingRoleTablePinsSystemStylesAndBaselineSizes() {
        let expected: [(GaryxTypographyRole, GaryxTypographyTextStyle, Double)] = [
            (.largeTitle, .largeTitle, 34),
            (.title, .title, 28),
            (.title2, .title2, 22),
            (.title3, .title3, 20),
            (.headline, .headline, 17),
            (.body, .body, 17),
            (.callout, .callout, 16),
            (.subheadline, .subheadline, 15),
            (.footnote, .footnote, 13),
            (.caption, .caption, 12),
            (.caption2, .caption2, 11),
        ]

        XCTAssertEqual(expected.map(\.0), GaryxTypographyRole.allCases)
        for (role, textStyle, size) in expected {
            XCTAssertEqual(role.specification.textStyle, textStyle)
            XCTAssertEqual(role.specification.basePointSize, size)
            XCTAssertEqual(role.specification.scalePolicy, .unbounded)
        }
    }

    func testOpticalIntentTightensDisplayAndOpensSmallReadingText() {
        for role in [GaryxTypographyRole.largeTitle, .title, .title2, .title3] {
            XCTAssertEqual(role.specification.tracking, .tightened)
            XCTAssertEqual(role.specification.leading, .tight)
        }

        for role in [GaryxTypographyRole.body, .callout, .subheadline] {
            XCTAssertEqual(role.specification.tracking, .neutral)
            XCTAssertEqual(role.specification.leading, .relaxed)
        }

        for role in [GaryxTypographyRole.footnote, .caption, .caption2] {
            XCTAssertEqual(role.specification.tracking, .opened)
            XCTAssertEqual(role.specification.leading, .relaxed)
        }
    }

    func testOnlyReadingSurfaceIsUncappedAndEveryChromeBoundaryExplainsItsCap() {
        XCTAssertNil(GaryxTypographyScaleBoundary.readingSurface.maximumCategory)

        for boundary in GaryxTypographyScaleBoundary.allCases where boundary != .readingSurface {
            XCTAssertEqual(boundary.maximumCategory, .extraExtraLarge)
            XCTAssertFalse(boundary.rationale.isEmpty)
        }
    }
}
