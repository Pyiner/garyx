import XCTest

final class HomeChromeInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testFilterMenuOpensFromCircleEdgeDismissesAndSelectsChats() throws {
        let app = launchHome()
        let filter = app.buttons["Recent filter"]
        XCTAssertTrue(filter.waitForExistence(timeout: 10), "Recent filter button")
        XCTAssertEqual(filter.value as? String, "All")

        tapTrailingCircleEdge(filter)
        XCTAssertTrue(app.buttons["All"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Chats"].exists)

        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.72)).tap()
        XCTAssertFalse(app.buttons["Chats"].waitForExistence(timeout: 1))

        tapTrailingCircleEdge(filter)
        let chats = app.buttons["Chats"]
        XCTAssertTrue(chats.waitForExistence(timeout: 5))
        chats.tap()

        XCTAssertEqual(filter.value as? String, "Chats")
    }

    func testFabEdgeTapOpensNewThreadDraft() throws {
        let app = launchHome()
        let fab = app.buttons["New chat"]
        XCTAssertTrue(fab.waitForExistence(timeout: 10), "Home new-chat FAB")

        XCTAssertEqual(fab.frame.width, 56, accuracy: 1)
        XCTAssertEqual(fab.frame.height, 56, accuracy: 1)
        XCTAssertEqual(app.frame.maxX - fab.frame.maxX, 20, accuracy: 2)

        tapTrailingCircleEdge(fab)
        XCTAssertTrue(
            app.buttons["Back"].waitForExistence(timeout: 5),
            "the FAB edge must open the existing new-thread draft instead of passing through"
        )
    }

    func testFabBandClearAreaStillScrollsList() throws {
        let app = launchHome(useScrollFixture: true)
        let fab = app.buttons["New chat"]
        XCTAssertTrue(fab.waitForExistence(timeout: 10), "Home new-chat FAB")

        let leadingRow = app.staticTexts["Synthetic thread 0"].firstMatch
        XCTAssertTrue(leadingRow.waitForExistence(timeout: 10), "top synthetic row")
        let initialMinY = leadingRow.frame.minY

        let startPoint = CGPoint(x: 40, y: fab.frame.midY)
        XCTAssertFalse(fab.frame.contains(startPoint), "gesture must begin in clear chrome")
        let origin = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        origin.withOffset(CGVector(dx: startPoint.x, dy: startPoint.y)).press(
            forDuration: 0.1,
            thenDragTo: origin.withOffset(
                CGVector(dx: startPoint.x, dy: max(120, startPoint.y - 260))
            )
        )

        let deadline = Date().addingTimeInterval(5)
        while leadingRow.isHittable,
              leadingRow.frame.minY >= initialMinY - 40,
              Date() < deadline {
            Thread.sleep(forTimeInterval: 0.1)
        }
        XCTAssertTrue(
            !leadingRow.isHittable || leadingRow.frame.minY < initialMinY - 40,
            "clear space beside the FAB must leave the underlying List scroll gesture available"
        )
    }

    private func launchHome(useScrollFixture: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        if useScrollFixture {
            app.launchEnvironment["GARYX_MOBILE_HOME_SCROLL_PROBE"] = "1"
        }
        app.launch()
        XCTAssertTrue(app.staticTexts["Garyx"].waitForExistence(timeout: 15))
        return app
    }

    private func tapTrailingCircleEdge(_ element: XCUIElement) {
        element.coordinate(withNormalizedOffset: CGVector(dx: 0.88, dy: 0.5)).tap()
    }
}
