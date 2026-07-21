import XCTest

final class TaskNotificationInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testCollapsedCardPreservesInteractionsAndPresentsCompleteBody() {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_TASK_NOTIFICATION_FIXTURE"] = "1"
        app.launch()

        let card = app.descendants(matching: .any)["garyx-task-notification-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        let expand = app.buttons["garyx-task-notification-expand"]
        XCTAssertTrue(expand.waitForExistence(timeout: 3))
        XCTAssertTrue(expand.isHittable)
        attachScreenshot(named: "task-notification-collapsed")

        let link = app.links["Open validation file"]
        XCTAssertTrue(link.waitForExistence(timeout: 3))
        link.tap()
        waitForStatus("link:validation-report.md", in: app)
        XCTAssertFalse(
            app.descendants(matching: .any)["garyx-task-notification-full-screen"].exists,
            "a Markdown link must not activate the card expansion"
        )

        card.coordinate(withNormalizedOffset: CGVector(dx: 0.55, dy: 0.12)).tap()
        let fullScreen = app.descendants(matching: .any)["garyx-task-notification-full-screen"]
        XCTAssertTrue(fullScreen.waitForExistence(timeout: 5))
        let endMarker = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "TASK-NOTIFICATION-E2E-END")
        ).firstMatch
        XCTAssertTrue(
            endMarker.waitForExistence(timeout: 5),
            "the stable full-screen owner must present the complete captured body"
        )
        attachScreenshot(named: "task-notification-full-screen")

        app.buttons["Done"].tap()
        XCTAssertTrue(card.waitForExistence(timeout: 3))
        XCTAssertFalse(fullScreen.exists)

        card.press(forDuration: 0.6)
        for label in ["Copy", "Select Text", "Share"] {
            XCTAssertTrue(
                app.buttons[label].waitForExistence(timeout: 3),
                "the task card must retain the shared long-press action \(label)"
            )
        }
        XCTAssertFalse(
            fullScreen.exists,
            "long press must not also activate expansion"
        )
    }

    private func waitForStatus(
        _ value: String,
        in app: XCUIApplication,
        timeout: TimeInterval = 3
    ) {
        let status = app.descendants(matching: .any)["task-notification.fixture.status"]
        XCTAssertTrue(status.waitForExistence(timeout: timeout))
        expectation(
            for: NSPredicate(format: "label == %@", value),
            evaluatedWith: status
        )
        waitForExpectations(timeout: timeout)
    }

    private func attachScreenshot(named name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
