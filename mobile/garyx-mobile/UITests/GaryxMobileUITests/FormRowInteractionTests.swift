import XCTest

/// Simulator verification for the P2 form-row hardening (design decision D10).
///
/// These drive the create-automation form, which is reachable from the debug
/// snapshot without a live gateway and whose schedule editor uses the migrated
/// primitives with static (non-seeded) data:
/// - "Repeat" is a `GaryxFormMenuRow` (whole row is the menu label).
/// - "Automation name" is a focus-on-tap `GaryxFormTextFieldRow`.
///
/// The dead-click bug was that only the trailing control was hittable; the fix
/// makes the entire row the tap target. Reverting the migration turns these red.
final class FormRowInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    private func launchCreateAutomationForm(
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "automations"
        app.launch()

        let addButton = app.buttons["New Automation"]
        XCTAssertTrue(
            addButton.waitForExistence(timeout: 25),
            "Automations panel should expose the New Automation button",
            file: file,
            line: line
        )
        addButton.tap()

        XCTAssertTrue(
            app.staticTexts["New Automation"].waitForExistence(timeout: 10),
            "Create automation form should present",
            file: file,
            line: line
        )
        return app
    }

    /// Rock-solid invariant: the migrated `GaryxFormMenuRow` control now spans
    /// most of the row width (the previously-dead title + spacer is part of the
    /// tap target). Before the fix the `Menu` wrapped only the trailing value.
    func testMenuRowControlSpansFullRowWidth() throws {
        let app = launchCreateAutomationForm()

        let repeatControl = repeatMenuControl(in: app)
        XCTAssertTrue(repeatControl.waitForExistence(timeout: 10), "Repeat menu control")

        let rowWidth = repeatControl.frame.width
        let screenWidth = app.windows.firstMatch.frame.width
        XCTAssertGreaterThan(
            rowWidth,
            screenWidth * 0.6,
            "The menu control should span the whole row (dead zone absorbed); got \(rowWidth) of \(screenWidth)"
        )
    }

    /// Functional proof: tapping the far-left of the row (the old dead zone)
    /// opens the menu and a selection updates the row.
    func testMenuRowOpensFromLeadingDeadZone() throws {
        let app = launchCreateAutomationForm()

        let repeatControl = repeatMenuControl(in: app)
        XCTAssertTrue(repeatControl.waitForExistence(timeout: 10), "Repeat menu control")

        repeatControl.coordinate(withNormalizedOffset: CGVector(dx: 0.04, dy: 0.5)).tap()

        let weeklyOption = app.buttons["Every Week"]
        XCTAssertTrue(
            weeklyOption.waitForExistence(timeout: 6),
            "Tapping the leading dead zone should open the repeat menu"
        )
        weeklyOption.tap()

        // Selecting from the menu updates the row's value (menu wiring intact).
        let updatedValue = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS[c] %@", "Every Week")).firstMatch
        XCTAssertTrue(
            updatedValue.waitForExistence(timeout: 6),
            "The Repeat row should reflect the chosen option"
        )
    }

    /// Focus-on-tap: tapping the leading label (not the field) focuses the text
    /// field so typing lands in it — the field keeps its own tap handling, so it
    /// is never wrapped in a `Button`.
    func testTextFieldRowFocusesFromLabelTap() throws {
        let app = launchCreateAutomationForm()

        let nameTitle = app.staticTexts["Name"]
        XCTAssertTrue(nameTitle.waitForExistence(timeout: 10), "Automation name row title")

        nameTitle.tap()
        // Settle for the field to take focus; the soft keyboard may be hidden
        // when a hardware keyboard is attached, so do not assert on it.
        _ = app.keyboards.element.waitForExistence(timeout: 4)
        app.typeText("QA")

        let typedField = app.textFields
            .matching(NSPredicate(format: "value CONTAINS %@", "QA")).firstMatch
        XCTAssertTrue(
            typedField.waitForExistence(timeout: 5),
            "Tapping the label should focus the field so typed text lands in it"
        )
    }

    // MARK: - Helpers

    /// The `GaryxFormMenuRow` surfaces as a button whose combined label carries
    /// the "Repeat" title and its current "Every Day" value.
    private func repeatMenuControl(in app: XCUIApplication) -> XCUIElement {
        let byTitle = app.buttons
            .matching(NSPredicate(format: "label CONTAINS[c] %@", "Repeat")).firstMatch
        if byTitle.exists { return byTitle }
        return app.buttons
            .matching(NSPredicate(format: "label CONTAINS[c] %@", "Every Day")).firstMatch
    }
}
