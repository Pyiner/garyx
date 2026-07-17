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

    func testThreadActionsUseLongPressMenuInsteadOfSwipeActions() throws {
        let app = launchHome(useScrollFixture: true)
        let row = app.staticTexts["Synthetic thread 7"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "archiveable unpinned thread row")

        row.press(forDuration: 0.8)

        let pinAction = app.buttons["Pin thread"]
        XCTAssertTrue(
            pinAction.waitForExistence(timeout: 5),
            "long-pressing a thread must present the pin action"
        )
        XCTAssertTrue(
            app.buttons["Archive thread"].waitForExistence(timeout: 5),
            "long-pressing a thread must present the destructive archive action"
        )
        XCTAssertEqual(
            pinAction.frame.width / app.frame.width,
            0.565,
            accuracy: 0.025,
            "the compact menu must preserve the reference image's screen-width proportion"
        )
        XCTAssertEqual(pinAction.frame.height, 44, accuracy: 2)
    }

    func testThreadSwipeRevealsCapabilityActionsWithoutOpeningThread() throws {
        let app = launchHome(useScrollFixture: true)
        let row = app.staticTexts["Synthetic thread 7"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "swipeable unpinned thread row")

        row.swipeLeft()

        let pinAction = app.buttons["Pin thread"]
        XCTAssertTrue(pinAction.waitForExistence(timeout: 5))
        XCTAssertTrue(pinAction.isHittable)
        XCTAssertTrue(app.buttons["Favorite thread"].isHittable)
        XCTAssertTrue(app.buttons["Archive thread"].isHittable)
        XCTAssertFalse(app.buttons["Back"].exists)
    }

    func testThresholdLongPressPresentsMenuWithoutOpeningThread() throws {
        let app = launchHome(useScrollFixture: true)
        let row = app.staticTexts["Synthetic thread 7"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "archiveable unpinned thread row")

        // Releasing just after the 0.36s recognition threshold is the human
        // path that can satisfy both the row tap and the simultaneous long
        // press. A deliberately long 0.8s XCTest press masks that race.
        row.press(forDuration: 0.42)

        XCTAssertTrue(
            app.buttons["Pin thread"].waitForExistence(timeout: 3),
            "a threshold long press must keep the action menu visible"
        )
        XCTAssertFalse(
            app.buttons["Back"].exists,
            "releasing a recognized long press must not also open the thread"
        )
        XCTAssertTrue(row.exists, "the pressed Home row must stay on the Home surface")
    }

    func testThreadRowShortTapStillOpensThread() throws {
        let app = launchHome(useScrollFixture: true)
        let row = app.staticTexts["Synthetic thread 7"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "tappable synthetic thread row")

        row.tap()

        XCTAssertTrue(
            app.buttons["Back"].waitForExistence(timeout: 5),
            "an ordinary short tap must still open the selected thread"
        )
        XCTAssertFalse(app.buttons["Pin thread"].exists)
    }

    func testPinnedThreadPinButtonRemainsDirectActionWithoutOpeningThread() throws {
        let app = launchHome(useScrollFixture: true)
        let row = app.staticTexts["Synthetic thread 0"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "pinned synthetic thread row")

        let unpinButtons = app.buttons.matching(identifier: "Unpin thread")
        XCTAssertGreaterThan(unpinButtons.count, 0, "Home keeps the direct Unpin affordance")

        unpinButtons.firstMatch.tap()

        XCTAssertFalse(
            app.buttons["Back"].waitForExistence(timeout: 1),
            "the direct Unpin action must not also open the thread"
        )
        XCTAssertTrue(row.exists, "the direct Unpin action must keep the row on Home")
    }

    func testThreadRowDragStillScrollsWithoutOpeningOrPresentingMenu() throws {
        let app = launchHome(useScrollFixture: true)
        let row = app.staticTexts["Synthetic thread 0"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "top synthetic thread row")
        let initialMinY = row.frame.minY
        let start = row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let origin = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        start.press(
            forDuration: 0.1,
            thenDragTo: origin.withOffset(
                CGVector(dx: row.frame.midX, dy: max(120, row.frame.midY - 280))
            )
        )

        let deadline = Date().addingTimeInterval(5)
        while row.isHittable,
              row.frame.minY >= initialMinY - 40,
              Date() < deadline {
            Thread.sleep(forTimeInterval: 0.1)
        }
        XCTAssertTrue(
            !row.isHittable || row.frame.minY < initialMinY - 40,
            "dragging from a thread row must preserve the List scroll gesture"
        )
        XCTAssertFalse(app.buttons["Back"].exists)
        XCTAssertFalse(app.buttons["Pin thread"].exists)
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
