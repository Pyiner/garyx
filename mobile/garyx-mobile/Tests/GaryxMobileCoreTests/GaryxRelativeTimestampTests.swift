import XCTest
@testable import GaryxMobileCore

final class GaryxRelativeTimestampTests: XCTestCase {
    private let value = "2026-01-01T00:00:00Z"

    private var base: Date {
        // Drive offsets off the production parser so the test does not hardcode
        // an epoch; parsing is also covered by `testParsesStandardAndFractional`.
        guard let date = garyxThreadDate(from: value) else {
            fatalError("base timestamp should parse")
        }
        return date
    }

    private func label(after seconds: TimeInterval) -> String {
        garyxFormattedTaskTimestamp(value, now: base.addingTimeInterval(seconds))
    }

    func testRelativeBoundaries() {
        XCTAssertEqual(label(after: 30), "now")        // < 1 minute
        XCTAssertEqual(label(after: 60), "1m")
        XCTAssertEqual(label(after: 59 * 60), "59m")
        XCTAssertEqual(label(after: 60 * 60), "1h")
        XCTAssertEqual(label(after: 23 * 3_600), "23h")
        XCTAssertEqual(label(after: 24 * 3_600), "1d")
        XCTAssertEqual(label(after: 29 * 86_400), "29d")
        XCTAssertEqual(label(after: 30 * 86_400), "1mo")
        XCTAssertEqual(label(after: 365 * 86_400), "1y")
    }

    func testClampsNowBeforeTimestampToNow() {
        XCTAssertEqual(label(after: -120), "now")
    }

    func testEmptyOrNilValueYieldsEmpty() {
        XCTAssertEqual(garyxFormattedTaskTimestamp(nil, now: base), "")
        XCTAssertEqual(garyxFormattedTaskTimestamp("", now: base), "")
        XCTAssertEqual(garyxFormattedTaskTimestamp("   ", now: base), "")
    }

    /// The freeze bug M6 guards against: with a fixed `value`, advancing `now`
    /// must change the label (the App row re-derives it via `.everyMinute`).
    func testLabelIsNotFrozenAsNowAdvances() {
        XCTAssertEqual(label(after: 5 * 60), "5m")
        XCTAssertEqual(label(after: 90 * 60), "1h")
        XCTAssertNotEqual(label(after: 5 * 60), label(after: 90 * 60))
    }

    func testParsesStandardAndFractional() {
        XCTAssertNotNil(garyxThreadDate(from: value))
        let fractional = "2026-01-01T00:00:00.123Z"
        guard let date = garyxThreadDate(from: fractional) else {
            return XCTFail("fractional timestamp should parse")
        }
        XCTAssertEqual(
            garyxFormattedTaskTimestamp(fractional, now: date.addingTimeInterval(120)),
            "2m"
        )
    }
}
