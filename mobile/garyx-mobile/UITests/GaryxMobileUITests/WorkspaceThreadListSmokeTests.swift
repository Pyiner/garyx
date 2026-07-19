import XCTest

final class WorkspaceThreadListSmokeTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testWorkspaceDrilldownShowsScopedThreadRows() {
        let app = launchWorkspaceDrilldown()

        XCTAssertTrue(
            app.staticTexts["Thread History"].waitForExistence(timeout: 10),
            "the native workspace List must resolve scoped membership through summaryById"
        )
        XCTAssertTrue(app.staticTexts["Tasks"].exists)
        XCTAssertTrue(app.buttons["Workspaces"].exists)
    }

    func testWorkspaceDrilldownUsesUnifiedSwipeActions() {
        let app = launchWorkspaceDrilldown()
        let row = app.staticTexts["Thread History"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))

        row.swipeLeft()
        let swipePin = app.buttons["Pin thread"]
        XCTAssertTrue(swipePin.waitForExistence(timeout: 5))
        XCTAssertTrue(swipePin.isHittable)
        XCTAssertTrue(app.buttons["Favorite thread"].isHittable)
        XCTAssertTrue(app.buttons["Archive thread"].isHittable)
    }

    func testWorkspaceDrilldownUsesUnifiedLongPressActions() {
        let app = launchWorkspaceDrilldown()
        let row = app.staticTexts["Thread History"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))

        row.press(forDuration: 0.8)

        let menuPin = app.buttons["Pin thread"]
        XCTAssertTrue(menuPin.waitForExistence(timeout: 5))
        XCTAssertEqual(menuPin.frame.height, 44, accuracy: 2)
        XCTAssertTrue(app.buttons["Favorite thread"].exists)
        XCTAssertTrue(app.buttons["Archive thread"].exists)
    }

    private func launchWorkspaceDrilldown() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "workspaceBots"
        app.launch()

        let workspace = app.buttons["workspace-row-/workspace/garyx"]
        XCTAssertTrue(workspace.waitForExistence(timeout: 15), "workspace row")
        workspace.tap()
        return app
    }
}
