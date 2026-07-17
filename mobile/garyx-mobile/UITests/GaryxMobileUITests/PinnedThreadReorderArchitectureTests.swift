import XCTest

final class PinnedThreadReorderArchitectureTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testAcceptedDropCommitsOnceFreezesSnapshotAndPreservesDelegates() throws {
        let app = launchHome(injectMidLiftSnapshot: true)
        let source = pinnedRow(0, in: app)
        let destination = pinnedRow(3, in: app)
        XCTAssertTrue(source.waitForExistence(timeout: 10))
        XCTAssertTrue(destination.exists)

        drag(source, to: destination)

        let lifecycle = try waitForValue(
            of: app.staticTexts["pinned-reorder-lifecycle"],
            containing: "accepted=1"
        )
        let result = try waitForValue(
            of: app.staticTexts["pinned-reorder-result"],
            containing: "commits=1"
        )
        XCTAssertTrue(lifecycle.contains("began=1"), lifecycle)
        XCTAssertTrue(lifecycle.contains("cancelled=0"), lifecycle)
        XCTAssertTrue(lifecycle.contains("delegates_unchanged=1"), lifecycle)
        XCTAssertTrue(result.contains("remote_mutations=0"), result)
        XCTAssertTrue(result.contains("midlift_frozen=1"), result)

        let recognizers = value(of: app.staticTexts["pinned-reorder-recognizers"])
        print("SPIKE recognizers=\(recognizers)")
        print("SPIKE accepted lifecycle=\(lifecycle) result=\(result)")
    }

    func testCancelledDragRestoresBaselineWithZeroCommit() throws {
        let app = launchHome()
        let source = pinnedRow(0, in: app)
        XCTAssertTrue(source.waitForExistence(timeout: 10))
        let baseline = pinnedOrder(in: app)
        let start = rowInteractionCoordinate(source)
        let outsideList = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02))

        drag(start, to: outsideList)

        let lifecycle = try waitForValue(
            of: app.staticTexts["pinned-reorder-lifecycle"],
            containing: "cancelled=1"
        )
        let result = value(of: app.staticTexts["pinned-reorder-result"])
        XCTAssertEqual(pinnedOrder(in: app), baseline)
        XCTAssertTrue(lifecycle.contains("accepted=0"), lifecycle)
        XCTAssertTrue(lifecycle.contains("delegates_unchanged=1"), lifecycle)
        XCTAssertTrue(result.contains("commits=0"), result)
        XCTAssertTrue(result.contains("remote_mutations=0"), result)
    }

    func testDestinationPastPinnedSegmentClampsToPinnedTail() throws {
        let app = launchHome()
        let source = pinnedRow(0, in: app)
        let pinnedTail = pinnedRow(5, in: app)
        XCTAssertTrue(source.waitForExistence(timeout: 10))
        XCTAssertTrue(pinnedTail.waitForExistence(timeout: 10))
        drag(
            rowInteractionCoordinate(source),
            to: rowInteractionCoordinate(pinnedTail, y: 0.95)
        )

        _ = try waitForValue(
            of: app.staticTexts["pinned-reorder-lifecycle"],
            containing: "accepted=1"
        )
        XCTAssertEqual(pinnedOrder(in: app), [1, 2, 3, 4, 5, 0])
        XCTAssertTrue(
            value(of: app.staticTexts["pinned-reorder-result"]).contains("commits=1")
        )
    }

    func testStationaryLongPressShowsMenuWhileMovementHandsOffToReorder() throws {
        var app = launchHome()
        var source = pinnedRow(0, in: app)
        XCTAssertTrue(source.waitForExistence(timeout: 10))

        rowInteractionCoordinate(source).press(forDuration: 0.55)
        let stationaryLifecycle = value(of: app.staticTexts["pinned-reorder-lifecycle"])
        print("SPIKE stationary lifecycle=\(stationaryLifecycle)")
        XCTAssertTrue(
            app.buttons["Favorite thread"].waitForExistence(timeout: 3),
            "a stationary hold must keep the existing action menu"
        )

        app.terminate()
        app = launchHome()
        source = pinnedRow(0, in: app)
        let destination = pinnedRow(3, in: app)
        XCTAssertTrue(source.waitForExistence(timeout: 10))
        drag(source, to: destination, holdDuration: 0.55)

        _ = try waitForValue(
            of: app.staticTexts["pinned-reorder-lifecycle"],
            containing: "accepted=1"
        )
        XCTAssertFalse(
            app.buttons["Favorite thread"].exists,
            "detected movement must dismiss the row action menu and hand off to reorder"
        )
    }

    func testPinTransitionMovesOneStableRowIdentity() throws {
        let app = launchHome()
        let row = pinnedRow(0, in: app)
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        let initialY = row.frame.midY
        let trigger = app.buttons["pinned-reorder-debug-pin-move"]
        XCTAssertTrue(trigger.waitForExistence(timeout: 5))

        trigger.tap()

        let deadline = Date().addingTimeInterval(3)
        while abs(row.frame.midY - initialY) < 40, Date() < deadline {
            Thread.sleep(forTimeInterval: 0.05)
        }
        XCTAssertEqual(
            app.staticTexts.matching(identifier: "Synthetic thread 0").count,
            1,
            "pin/unpin must animate one stable thread identity, not delete and reinsert twins"
        )
        XCTAssertGreaterThan(abs(row.frame.midY - initialY), 40)
        XCTAssertTrue(
            value(of: app.staticTexts["pinned-reorder-result"]).contains("pin_moves=1")
        )
    }

    private func launchHome(injectMidLiftSnapshot: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        app.launchEnvironment["GARYX_MOBILE_HOME_SCROLL_PROBE"] = "1"
        app.launchEnvironment["GARYX_MOBILE_HOME_SCROLL_PROBE_MANUAL"] = "1"
        app.launchEnvironment["GARYX_MOBILE_PIN_REORDER_SPIKE"] = "1"
        if injectMidLiftSnapshot {
            app.launchEnvironment["GARYX_MOBILE_PIN_REORDER_INJECT_MIDLIFT"] = "1"
        }
        app.launch()
        XCTAssertTrue(app.staticTexts["Garyx"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["pinned-reorder-lifecycle"].waitForExistence(timeout: 5))
        return app
    }

    private func pinnedRow(_ index: Int, in app: XCUIApplication) -> XCUIElement {
        // The unified row exposes an outer accessibility Button around the
        // title, pin control, and timestamp. Drive that stable row surface;
        // title StaticText coordinates can land beside the nested Unpin button
        // and intermittently fail to enter the native List drag recognizer.
        app.buttons.matching(
            NSPredicate(format: "label BEGINSWITH %@", "Synthetic thread \(index),")
        ).firstMatch
    }

    private func rowInteractionCoordinate(
        _ row: XCUIElement,
        y: CGFloat = 0.5
    ) -> XCUICoordinate {
        // 82% is the row's trailing empty hit region: outside the title and
        // nested pin control, but still inside the native List cell.
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.82, dy: y))
    }

    private func drag(
        _ source: XCUIElement,
        to destination: XCUIElement,
        holdDuration: TimeInterval = 0.65
    ) {
        drag(
            rowInteractionCoordinate(source),
            to: rowInteractionCoordinate(destination),
            holdDuration: holdDuration
        )
    }

    private func drag(
        _ source: XCUICoordinate,
        to destination: XCUICoordinate,
        holdDuration: TimeInterval = 0.65
    ) {
        // Native List reordering needs movement samples after the lift and a
        // settled destination before release. The convenience overload uses
        // the default 500 pt/s path with no destination hold; on iOS 26.5 it
        // intermittently collapses to a stationary long press and emits zero
        // drag callbacks. A slow interpolated path plus a short terminal hold
        // keeps this an end-to-end native gesture while making its event
        // sequence deterministic.
        source.press(
            forDuration: holdDuration,
            thenDragTo: destination,
            withVelocity: .slow,
            thenHoldForDuration: 0.35
        )
    }

    private func pinnedOrder(in app: XCUIApplication) -> [Int] {
        (0..<6)
            .compactMap { index -> (Int, CGFloat)? in
                let row = pinnedRow(index, in: app)
                guard row.exists else { return nil }
                return (index, row.frame.midY)
            }
            .sorted { $0.1 < $1.1 }
            .map(\.0)
    }

    private func value(of element: XCUIElement) -> String {
        (element.value as? String) ?? element.label
    }

    private func waitForValue(
        of element: XCUIElement,
        containing expected: String,
        timeout: TimeInterval = 5
    ) throws -> String {
        XCTAssertTrue(element.waitForExistence(timeout: timeout))
        let predicate = NSPredicate(format: "value CONTAINS %@", expected)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: element)
        let result = XCTWaiter.wait(for: [expectation], timeout: timeout)
        XCTAssertEqual(result, .completed, "Expected \(expected) in \(value(of: element))")
        return value(of: element)
    }
}
