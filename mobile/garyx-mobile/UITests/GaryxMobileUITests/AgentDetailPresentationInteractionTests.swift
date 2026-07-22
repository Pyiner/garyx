import XCTest

final class AgentDetailPresentationInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testEditRemainsPresentedBeyondFormerDismissWindow() {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "agents"
        app.launch()

        let reviewer = app.staticTexts["Reviewer"].firstMatch
        XCTAssertTrue(reviewer.waitForExistence(timeout: 10))
        reviewer.tap()

        let detailNavigationBar = app.navigationBars["Agent Detail"]
        XCTAssertTrue(detailNavigationBar.waitForExistence(timeout: 5))
        let editButton = app.buttons["Edit Agent"]
        for _ in 0..<8 where !editButton.exists {
            app.swipeUp()
        }
        XCTAssertTrue(editButton.waitForExistence(timeout: 5))
        editButton.tap()

        let editNavigationBar = app.navigationBars["Edit Agent"]
        XCTAssertTrue(editNavigationBar.waitForExistence(timeout: 5))
        Thread.sleep(forTimeInterval: 5)
        XCTAssertTrue(
            editNavigationBar.exists,
            "Edit must remain presented beyond the former automatic-dismiss window"
        )

        app.buttons["Cancel"].tap()
        XCTAssertTrue(detailNavigationBar.waitForExistence(timeout: 5))
        app.buttons["Done"].tap()
        let detailDidDismiss = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"),
            object: detailNavigationBar
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [detailDidDismiss], timeout: 5),
            .completed
        )
        XCTAssertTrue(reviewer.isHittable)
    }
}
