import XCTest

final class DurableDeliveryInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testUnknownSendRestoresWithoutASecondDecisionCard() {
        let app = launchFixture()
        let restore = app.buttons["Restore uncertain send as draft"]
        XCTAssertTrue(restore.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(restore.frame.height, 44)

        restore.tap()

        waitForStatus("restore:automatic", in: app)
        XCTAssertFalse(app.staticTexts["Send status unknown"].exists)
        XCTAssertFalse(app.staticTexts["Recovered message is ready"].exists)
        XCTAssertFalse(app.buttons["Use recovered message draft"].exists)
        XCTAssertFalse(app.buttons["Keep current message draft"].exists)
    }

    func testDuplicateRiskExitRequiresExplicitWarningAndFreshIntentAction() {
        let app = launchFixture()
        let resend = app.buttons["Resend a duplicate-risk copy"]
        XCTAssertTrue(resend.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(resend.frame.height, 44)
        resend.tap()

        let alert = app.alerts["This may create a duplicate"]
        XCTAssertTrue(alert.waitForExistence(timeout: 3))
        let warning = alert.staticTexts.matching(
            NSPredicate(
                format: "label == %@",
                "The original message or conversation may already exist. The copy uses a new intent ID, but the gateway cannot prevent a duplicate yet."
            )
        ).firstMatch
        XCTAssertTrue(warning.exists)
        XCTAssertTrue(alert.buttons["Cancel"].exists)
        let confirm = alert.buttons["Send duplicate-risk copy"]
        XCTAssertTrue(confirm.exists)
        confirm.tap()

        waitForStatus("resend:new-client-intent", in: app)
        XCTAssertFalse(app.staticTexts["Send status unknown"].exists)
    }

    func testDurableFeedbackChipsOwnAcknowledgementRetryAndRemovalActions() {
        var app = launchFixture()
        let acknowledge = app.buttons["Dismiss this durable notice"]
        XCTAssertTrue(acknowledge.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(acknowledge.frame.height, 44)
        acknowledge.tap()
        waitForStatus("feedback:acknowledged", in: app)
        XCTAssertFalse(app.staticTexts["Too many sends awaiting confirmation"].exists)

        let retry = app.buttons["Retry the failed attachment upload"]
        XCTAssertTrue(retry.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(retry.frame.height, 44)
        retry.tap()
        waitForStatus("upload:retried", in: app)
        XCTAssertFalse(app.staticTexts["Upload did not finish"].exists)

        app.terminate()
        app = launchFixture()
        let remove = app.buttons["Remove the failed attachment"]
        XCTAssertTrue(remove.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(remove.frame.height, 44)
        remove.tap()
        waitForStatus("upload:removed", in: app)
        XCTAssertFalse(app.staticTexts["Upload did not finish"].exists)
    }

    func testAmbiguousCreateRestoresWithoutASecondDecisionCard() {
        var app = launchFixture(
            scenario: "create",
            expectedNoticeTitle: "Conversation creation status unknown"
        )
        let restore = app.buttons["Restore uncertain send as draft"]
        let rebuild = app.buttons["Rebuild a duplicate-risk conversation copy"]
        XCTAssertTrue(restore.waitForExistence(timeout: 3))
        XCTAssertTrue(rebuild.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(restore.frame.height, 44)
        XCTAssertGreaterThanOrEqual(rebuild.frame.height, 44)
        XCTAssertTrue(restore.isHittable)
        XCTAssertTrue(rebuild.isHittable)

        restore.tap()
        waitForStatus("create:restore:automatic", in: app)
        XCTAssertFalse(app.staticTexts["Recovered message is ready"].exists)

        app.terminate()
        app = launchFixture(
            scenario: "create",
            expectedNoticeTitle: "Conversation creation status unknown"
        )
        let relaunchedRebuild = app.buttons["Rebuild a duplicate-risk conversation copy"]
        XCTAssertTrue(relaunchedRebuild.waitForExistence(timeout: 3))
        relaunchedRebuild.tap()
        let alert = app.alerts["This may create a duplicate"]
        XCTAssertTrue(alert.waitForExistence(timeout: 3))
        let confirm = alert.buttons["Send duplicate-risk copy"]
        XCTAssertTrue(confirm.exists)
        confirm.tap()
        waitForStatus("create:rebuild:new-client-intent", in: app)
        XCTAssertFalse(app.staticTexts["Conversation creation status unknown"].exists)
    }

    func testSendExitAndChipSurfacesPassVoiceOverDescriptionAndHitRegionAudit() throws {
        let app = launchFixture()
        let send = app.buttons["Send fixture message"]
        XCTAssertTrue(send.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(send.frame.height, 44)
        send.tap()
        waitForStatus("send:ambiguous", in: app)

        let voiceOverLabels = [
            "Restore uncertain send as draft",
            "Resend a duplicate-risk copy",
            "Dismiss this durable notice",
            "Retry the failed attachment upload",
            "Remove the failed attachment",
        ]
        for label in voiceOverLabels {
            let element = app.buttons[label]
            XCTAssertTrue(element.waitForExistence(timeout: 3), label)
            XCTAssertTrue(element.isHittable, label)
            XCTAssertGreaterThanOrEqual(element.frame.height, 44, label)
        }

        try app.performAccessibilityAudit(for: [.hitRegion, .sufficientElementDescription])
    }

    private func launchFixture(
        scenario: String? = nil,
        expectedNoticeTitle: String = "Send status unknown"
    ) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DURABLE_DELIVERY_FIXTURE"] = "1"
        if let scenario {
            app.launchEnvironment["GARYX_MOBILE_DURABLE_DELIVERY_SCENARIO"] = scenario
        }
        app.launch()
        XCTAssertTrue(app.staticTexts[expectedNoticeTitle].waitForExistence(timeout: 10))
        return app
    }

    private func waitForStatus(
        _ value: String,
        in app: XCUIApplication,
        timeout: TimeInterval = 3
    ) {
        let status = app.staticTexts["durable.fixture.status"]
        XCTAssertTrue(status.waitForExistence(timeout: timeout))
        let predicate = NSPredicate(format: "label == %@", value)
        expectation(for: predicate, evaluatedWith: status)
        waitForExpectations(timeout: timeout)
    }
}
