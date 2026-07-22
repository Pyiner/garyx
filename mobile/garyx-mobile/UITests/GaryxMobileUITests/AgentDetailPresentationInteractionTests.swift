import XCTest

final class AgentDetailPresentationInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testEditRemainsPresentedBeyondFormerDismissWindow() throws {
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

        let uploadAvatar = app.buttons["Upload avatar"]
        let generateAvatar = app.buttons["Generate avatar"]
        let removeAvatar = app.buttons["Remove avatar"]
        XCTAssertTrue(uploadAvatar.waitForExistence(timeout: 5))
        XCTAssertTrue(generateAvatar.waitForExistence(timeout: 5))
        XCTAssertTrue(removeAvatar.waitForExistence(timeout: 5))

        let avatarActionFrames = [
            uploadAvatar.frame,
            generateAvatar.frame,
            removeAvatar.frame,
        ]
        let expectedHeight = try XCTUnwrap(avatarActionFrames.first?.height)
        for frame in avatarActionFrames.dropFirst() {
            XCTAssertEqual(
                frame.height,
                expectedHeight,
                accuracy: 1,
                "Horizontal avatar actions must remain single-line and equal-height"
            )
        }
        let expectedMidY = try XCTUnwrap(avatarActionFrames.first?.midY)
        for frame in avatarActionFrames.dropFirst() {
            XCTAssertEqual(
                frame.midY,
                expectedMidY,
                accuracy: 1,
                "Horizontal avatar actions must share one row"
            )
        }

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
