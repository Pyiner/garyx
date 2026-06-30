import XCTest

@testable import GaryxMobileCore

final class GaryxRestartNoticePresentationTests: XCTestCase {
    func testParsesRestartNoticeEnvelope() {
        let parsed = GaryxRestartNoticePresentation.parse(
            "<garyx_restarted>Garyx has restarted. Continue your task.</garyx_restarted>"
        )
        XCTAssertEqual(
            parsed,
            GaryxRestartNotice(message: "Garyx has restarted. Continue your task.")
        )
    }

    func testToleratesWhitespaceAndAttributes() {
        let parsed = GaryxRestartNoticePresentation.parse(
            """

            <garyx_restarted reason="manual">
            Back online — pick up where you left off.
            </garyx_restarted>

            """
        )
        XCTAssertEqual(
            parsed,
            GaryxRestartNotice(message: "Back online — pick up where you left off.")
        )
    }

    func testEmptyBodyFallsBackToDefault() {
        let parsed = GaryxRestartNoticePresentation.parse("<garyx_restarted></garyx_restarted>")
        XCTAssertEqual(
            parsed,
            GaryxRestartNotice(message: GaryxRestartNoticePresentation.defaultMessage)
        )
    }

    func testNonRestartTextReturnsNil() {
        XCTAssertNil(GaryxRestartNoticePresentation.parse("just a normal message"))
        XCTAssertNil(
            GaryxRestartNoticePresentation.parse(
                "<garyx_task_notification></garyx_task_notification>"
            )
        )
    }
}
