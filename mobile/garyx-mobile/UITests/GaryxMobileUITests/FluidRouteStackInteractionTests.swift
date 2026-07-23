import UIKit
import XCTest

final class FluidRouteStackInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testFinishCommitsInteractivePop() {
        let app = launchFakeRoutes(depth: 2)
        dragLeadingEdge(in: app, fromInset: 5, travel: app.frame.width * 0.82)

        waitForTitle("Fake route depth 1", in: app)
        waitForStatus("terminal=committed-visible", in: app)
        waitForStatus("screenChanged=1", in: app)
        waitForStatus("performance=pass", in: app)
        waitForStatus("backwards=0", in: app)
        waitForStatus("bodyDelta=0", in: app)
        attachStatus(from: app, name: "fluid-route-finish-performance")
    }

    func testSlowMiddleDragCancels() {
        let app = launchFakeRoutes(depth: 2)
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.3947,
            velocity: XCUIGestureVelocity(rawValue: 40),
            holdAtEnd: 0.35
        )

        waitForTitle("Fake route depth 2", in: app)
        waitForStatus("terminal=cancelled-visible", in: app)
        waitForStatus("screenChanged=0", in: app)
    }

    func testFastFlickCommitsAtMeasuredEighteenPointTwoFourPercent() {
        let app = launchFakeRoutes(depth: 2)
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.1824,
            velocity: .fast
        )

        waitForTitle("Fake route depth 1", in: app)
        waitForStatus("terminal=committed-visible", in: app)
    }

    func testCancelSettleCanBeRegrabbed() {
        let app = launchFakeRoutes(depth: 2)
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.30,
            // XCTest's symbolic `.slow` velocity varies enough under a full
            // suite load to cross the commit projection. Pin the same low
            // physical velocity as the deterministic cancel acceptance case.
            velocity: XCUIGestureVelocity(rawValue: 40),
            holdAtEnd: 0.12
        )
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.80,
            velocity: .fast
        )

        waitForTitle("Fake route depth 1", in: app)
        waitForStatus("regrabs=1", in: app)
        waitForStatus("terminal=committed-visible", in: app)
    }

    func testDeepStackAndFirstLayerPopToHome() {
        let deep = launchFakeRoutes(depth: 20)
        waitForStatus("mounted=2", in: deep)
        dragLeadingEdge(in: deep, fromInset: 5, travel: deep.frame.width * 0.82)
        waitForTitle("Fake route depth 19", in: deep)
        waitForStatus("peakMounted=2", in: deep)

        deep.terminate()
        let firstLayer = launchFakeRoutes(depth: 1)
        dragLeadingEdge(
            in: firstLayer,
            fromInset: 5,
            travel: firstLayer.frame.width * 0.82
        )
        waitForTitle("Fake home", in: firstLayer)
        waitForStatus("depth=0", in: firstLayer)

        dragLeadingEdge(
            in: firstLayer,
            fromInset: 5,
            travel: firstLayer.frame.width * 0.40
        )
        waitForStatus("homeEdges=1", in: firstLayer)
        waitForTitle("Fake home", in: firstLayer)
    }

    func testRTLUsesPhysicalRightEdgeAndMirroredDirection() {
        let app = launchFakeRoutes(depth: 2, rtl: true)
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.82,
            rtl: true
        )

        waitForTitle("Fake route depth 1", in: app)
        waitForStatus("direction=rightToLeft", in: app)
        waitForStatus("terminal=committed-visible", in: app)
    }

    func testTouchStartingAtFivePointsRemainsNavigationOwnedAtTwentyFive() {
        let app = launchFakeRoutes(depth: 2)
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: 20,
            velocity: .slow,
            holdAtEnd: 0.20,
            y: horizontalScrollerMidY(in: app)
        )

        waitForStatus("transactions=1", in: app)
        waitForStatus("terminal=cancelled-visible", in: app)
        waitForTitle("Fake route depth 2", in: app)
    }

    func testTouchStartingOutsideZoneThenMovingBackToEdgeStaysContentOwned() {
        let app = launchFakeRoutes(depth: 2)
        dragLeadingEdge(
            in: app,
            fromInset: 25,
            travel: -20,
            velocity: .slow,
            holdAtEnd: 0.20,
            y: horizontalScrollerMidY(in: app)
        )

        waitForStatus("transactions=0", in: app)
        waitForTitle("Fake route depth 2", in: app)
    }

    func testHomeDrawerUsesPhysicalLeadingEdgeInLTRAndRTL() {
        for rtl in [false, true] {
            let app = launchProductionHome(rtl: rtl)
            dragLeadingEdge(
                in: app,
                fromInset: 5,
                travel: app.frame.width * 0.72,
                rtl: rtl
            )

            let settings = app.buttons["Settings"]
            waitForHittable(settings, named: "drawer Settings (rtl=\(rtl))")
            dragDrawerClosed(in: app, rtl: rtl)
            waitForNotHittable(settings, named: "closed drawer Settings (rtl=\(rtl))")
            app.terminate()
        }
    }

    func testHomeDrawerCancelSettleCanBeRegrabbedAndReversed() {
        let app = launchProductionHome()
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: 80,
            velocity: XCUIGestureVelocity(rawValue: 40),
            holdAtEnd: 0.05
        )
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.72,
            velocity: .fast
        )

        waitForHittable(app.buttons["Settings"], named: "regrabbed drawer")
    }

    func testTaskTreeTrailingEdgeOwnsOpenCloseAndMakesPopIneligible() {
        let app = launchProductionConversation(taskTreeFixture: true)
        dragTrailingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.78
        )
        let taskTreeTitle = app.staticTexts["Task tree"]
        XCTAssertTrue(taskTreeTitle.waitForExistence(timeout: 5))

        // While the task surface is open, a leading-edge rightward swipe is
        // routed to task-tree close. It must never pop the conversation.
        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.78
        )
        XCTAssertTrue(app.buttons["Back"].waitForExistence(timeout: 3))
        XCTAssertFalse(taskTreeTitle.waitForExistence(timeout: 2))

        dragTrailingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.78
        )
        XCTAssertTrue(taskTreeTitle.waitForExistence(timeout: 5))
        dragTaskTreeClosed(in: app)
        XCTAssertFalse(taskTreeTitle.waitForExistence(timeout: 2))
        XCTAssertTrue(app.buttons["Back"].exists)
    }

    func testTaskTreeOpenKeepsConversationBackdropUniform() throws {
        let app = launchProductionConversation(taskTreeFixture: true)
        dragTrailingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.78
        )
        XCTAssertTrue(app.staticTexts["Task tree"].waitForExistence(timeout: 5))

        let screenshot = app.screenshot().image
        let panelWidth = min(max(app.frame.width * 0.55, 300), 420)
        let panelBoundaryX = app.frame.width - panelWidth
        let sampleY = app.frame.height * 0.5
        let farBackdrop = try averageBrightness(
            in: screenshot,
            around: CGPoint(x: 40, y: sampleY),
            appFrame: app.frame
        )
        let nearPanelBackdrop = try averageBrightness(
            in: screenshot,
            around: CGPoint(x: panelBoundaryX - 5, y: sampleY),
            appFrame: app.frame
        )

        XCTAssertEqual(
            nearPanelBackdrop,
            farBackdrop,
            accuracy: 3,
            "task-tree reveal added a localized moving shadow to the conversation backdrop"
        )
    }

    func testTaskTreeCancelSettleCanBeRegrabbed() {
        let app = launchProductionConversation(taskTreeFixture: true)
        dragTrailingEdge(
            in: app,
            fromInset: 5,
            travel: 90,
            velocity: XCUIGestureVelocity(rawValue: 40),
            holdAtEnd: 0.05
        )
        dragTrailingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.78,
            velocity: .fast
        )

        XCTAssertTrue(app.staticTexts["Task tree"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Back"].exists)
    }

    func testTaskTreeUsesPhysicalTrailingEdgeInRTL() {
        let app = launchProductionConversation(taskTreeFixture: true, rtl: true)
        dragTrailingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.78,
            rtl: true
        )

        let taskTreeTitle = app.staticTexts["Task tree"]
        XCTAssertTrue(taskTreeTitle.waitForExistence(timeout: 5))
        dragTaskTreeClosed(in: app, rtl: true)
        XCTAssertFalse(taskTreeTitle.waitForExistence(timeout: 2))
        XCTAssertTrue(app.buttons["Back"].exists)
    }

    func testSlowMotionFramesMatchFrozenSystemGeometry() {
        let app = launchFakeRoutes(depth: 2)
        let button = app.buttons["fluid.fake.slow-motion"]
        XCTAssertTrue(button.waitForExistence(timeout: 5))
        button.tap()
        Thread.sleep(forTimeInterval: 0.25)

        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "fluid-route-slow-motion-reference"
        attachment.lifetime = .keepAlways
        add(attachment)

        waitForStatus("curve=pass", in: app, timeout: 5)
        waitForTitle("Fake route depth 2", in: app)
    }

    func testFiveHundredChurnReachesStableZeroResidueState() {
        let app = launchFakeRoutes(depth: 0, churnIterations: 500)
        waitForStatus("churn=pass", in: app, timeout: 20)
        waitForStatus("churnIterations=500", in: app)
        waitForStatus("peakMounted=2", in: app)
        waitForStatus("depth=0", in: app)
        waitForTitle("Fake home", in: app)
    }

    func testProductionConversationCancelKeepsKeyboardAndMatchesSystemFrames() {
        let app = launchProductionConversation()

        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.3947,
            velocity: XCUIGestureVelocity(rawValue: 40),
            holdAtEnd: 0.35,
            y: app.frame.height * 0.30
        )

        XCTAssertTrue(app.buttons["Back"].waitForExistence(timeout: 5))
        waitForProductionStatus("terminal=cancelled-visible", in: app)
        waitForProductionStatus("curve=pass", in: app)
        waitForProductionStatus("backwards=0", in: app)
        waitForProductionStatus("focusAtStart=1", in: app)
        waitForProductionStatus("liveAdapters=1", in: app)
        waitForProductionStatus("focusedAdapters=1", in: app)
    }

    func testProductionConversationFinishDismissesKeyboardAndMatchesSystemFrames() {
        let app = launchProductionConversation()

        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.82,
            y: app.frame.height * 0.30
        )

        waitForProductionStatus("depth=0", in: app, timeout: 8)
        waitForProductionStatus("terminal=committed-visible", in: app)
        waitForProductionStatus("curve=pass", in: app)
        waitForProductionStatus("backwards=0", in: app)
        waitForProductionStatus("focusAtStart=1", in: app)
        waitForProductionStatus("liveAdapters=0", in: app)
        waitForProductionStatus("focusedAdapters=0", in: app)
        XCTAssertFalse(app.buttons["Back"].exists)
        attachProductionStatus(from: app, name: "production-route-finish-performance")
    }

    func testProductionInteractivePopHasNoDisplayHitches() {
        let app = launchProductionConversationForPopPerformance()

        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.82,
            y: app.frame.height * 0.30
        )

        let report = app.staticTexts["route-pop-probe-report"]
        XCTAssertTrue(report.waitForExistence(timeout: 8))
        let completed = NSPredicate(format: "value CONTAINS %@", "state=complete")
        let completedExpectation = XCTNSPredicateExpectation(
            predicate: completed,
            object: report
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [completedExpectation], timeout: 8),
            .completed,
            "pop probe never completed; value=\(String(describing: report.value))"
        )
        let value = String(describing: report.value)
        let attachment = XCTAttachment(string: value)
        attachment.name = "production-interactive-pop-cadence"
        attachment.lifetime = .keepAlways
        add(attachment)
        XCTAssertTrue(
            value.contains("hitches=0"),
            "interactive pop dropped a display frame: \(value)"
        )
        XCTAssertTrue(
            value.contains("settleHitches=0"),
            "interactive pop settle dropped a display frame: \(value)"
        )
        XCTAssertTrue(
            value.contains("movingModelPublications=0"),
            "model observation published while the route was moving: \(value)"
        )
        XCTAssertTrue(
            value.contains("movingStorePublications=0"),
            "route-store observation published while the route was moving: \(value)"
        )
        XCTAssertTrue(
            value.contains("movingComposerPublications=0"),
            "composer observation published while the route was moving: \(value)"
        )
        XCTAssertFalse(app.buttons["Back"].exists)
    }

    func testProductionConversationCancelSettleCanRegrabWithKeyboard() {
        let app = launchProductionConversation(automaticallyRegrabs: true)

        dragLeadingEdge(
            in: app,
            fromInset: 5,
            travel: app.frame.width * 0.30,
            velocity: XCUIGestureVelocity(rawValue: 40),
            holdAtEnd: 0.12,
            y: app.frame.height * 0.30
        )

        waitForProductionStatus("depth=0", in: app, timeout: 8)
        waitForProductionStatus("regrabs=1", in: app)
        waitForProductionStatus("terminal=committed-visible", in: app)
        waitForProductionStatus("curve=pass", in: app)
        waitForProductionStatus("backwards=0", in: app)
        waitForProductionStatus("focusAtStart=1", in: app)
        waitForProductionStatus("liveAdapters=0", in: app)
        waitForProductionStatus("focusedAdapters=0", in: app)
    }

    func testProductionComposerTapRepositionsCaretInsideEnteredText() {
        let app = launchProductionConversation()
        let composer = app.textViews["garyx-composer-uikit-input"]
        let original = "ALPHA BRAVO CHARLIE DELTA"
        UIPasteboard.general.string = original
        app.typeKey("a", modifierFlags: .command)
        app.typeKey("v", modifierFlags: .command)
        waitForValue(original, in: composer)

        let frame = composer.frame
        composer.coordinate(
            withNormalizedOffset: CGVector(
                dx: 76 / frame.width,
                dy: 25 / frame.height
            )
        ).tap()
        UIPasteboard.general.string = "X"
        app.typeKey("v", modifierFlags: .command)

        // UITextView's standard smart-insert behavior adds a separator after
        // the pasted marker; its position still proves the tap selected the
        // boundary between "ALPHA " and "BRAVO".
        let expected = "ALPHA X BRAVO CHARLIE DELTA"
        waitForValue(expected, in: composer)
        XCTAssertEqual(
            composer.value as? String,
            expected,
            "the system selection must place inserted text at the tapped coordinate; frame=\(frame)"
        )
    }

    // MARK: Helpers

    private func waitForValue(
        _ value: String,
        in composer: XCUIElement,
        timeout: TimeInterval = 5
    ) {
        let predicate = NSPredicate(format: "value == %@", value)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: composer)
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation], timeout: timeout),
            .completed,
            "composer value did not become \(value); actual=\(String(describing: composer.value))"
        )
    }

    private func launchFakeRoutes(
        depth: Int,
        rtl: Bool = false,
        churnIterations: Int = 0
    ) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_FLUID_FAKE_ROUTES"] = "1"
        app.launchEnvironment["GARYX_MOBILE_FLUID_FAKE_DEPTH"] = String(depth)
        app.launchEnvironment["GARYX_MOBILE_FLUID_FAKE_RTL"] = rtl ? "1" : "0"
        app.launchEnvironment["GARYX_MOBILE_FLUID_FAKE_VISUAL_POLICY"] = "spatial"
        app.launchEnvironment["GARYX_MOBILE_FLUID_FAKE_PAYLOAD_KB"] = "64"
        app.launchEnvironment["GARYX_MOBILE_FLUID_FAKE_CHURN"] = String(churnIterations)
        app.launch()
        waitForTitle(depth == 0 ? "Fake home" : "Fake route depth \(depth)", in: app)
        XCTAssertTrue(app.staticTexts["fluid.fake.status"].waitForExistence(timeout: 10))
        return app
    }

    private func launchProductionConversation(
        automaticallyRegrabs: Bool = false,
        taskTreeFixture: Bool = false,
        rtl: Bool = false
    ) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "chat"
        app.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_DIAGNOSTICS"] = "1"
        app.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_AUTO_FOCUS"] = "1"
        app.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_AUTO_REGRAB"] = automaticallyRegrabs
            ? "1"
            : "0"
        app.launchEnvironment["GARYX_MOBILE_A5_TASK_TREE_FIXTURE"] = taskTreeFixture ? "1" : "0"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_RTL"] = rtl ? "1" : "0"
        app.launch()
        XCTAssertTrue(app.buttons["Back"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["production.route.status"].waitForExistence(timeout: 10))
        let composer = app.textViews["garyx-composer-uikit-input"]
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        let live = NSPredicate(format: "label == %@", "composer-live")
        let liveExpectation = XCTNSPredicateExpectation(predicate: live, object: composer)
        XCTAssertEqual(
            XCTWaiter.wait(for: [liveExpectation], timeout: 10),
            .completed,
            "production composer never received live adapter ownership"
        )
        composer.tap()
        // The UI-test simulator uses a hardware keyboard, for which XCUI's
        // keyboard-focus query is not reliable. The production frame probe
        // asserts UITextView.isFirstResponder directly once the drag begins.
        Thread.sleep(forTimeInterval: 0.5)
        waitForProductionStatus("depth=1", in: app)
        return app
    }

    private func launchProductionHome(rtl: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_RTL"] = rtl ? "1" : "0"
        app.launch()
        XCTAssertTrue(app.staticTexts["Garyx"].waitForExistence(timeout: 10))
        return app
    }

    private func launchProductionConversationForPopPerformance() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "chat"
        app.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_AUTO_FOCUS"] = "1"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_POP_PROBE"] = "1"
        app.launch()
        XCTAssertTrue(app.buttons["Back"].waitForExistence(timeout: 10))
        XCTAssertTrue(
            app.staticTexts["route-pop-probe-report"].waitForExistence(timeout: 10)
        )
        let composer = app.textViews["garyx-composer-uikit-input"]
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        composer.tap()
        Thread.sleep(forTimeInterval: 0.5)
        return app
    }

    private func waitForTitle(
        _ expected: String,
        in app: XCUIApplication,
        timeout: TimeInterval = 5
    ) {
        let title = app.staticTexts["fluid.fake.route-title"]
        XCTAssertTrue(title.waitForExistence(timeout: timeout))
        let predicate = NSPredicate(format: "label == %@", expected)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: title)
        XCTAssertEqual(XCTWaiter.wait(for: [expectation], timeout: timeout), .completed)
    }

    private func waitForStatus(
        _ fragment: String,
        in app: XCUIApplication,
        timeout: TimeInterval = 3
    ) {
        let status = app.staticTexts["fluid.fake.status"]
        XCTAssertTrue(status.waitForExistence(timeout: timeout))
        let predicate = NSPredicate(format: "value CONTAINS %@", fragment)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: status)
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation], timeout: timeout),
            .completed,
            "status never contained \(fragment); value=\(String(describing: status.value))"
        )
    }

    private func waitForProductionStatus(
        _ fragment: String,
        in app: XCUIApplication,
        timeout: TimeInterval = 5
    ) {
        let status = app.staticTexts["production.route.status"]
        XCTAssertTrue(status.waitForExistence(timeout: timeout))
        let predicate = NSPredicate(format: "value CONTAINS %@", fragment)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: status)
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation], timeout: timeout),
            .completed,
            "production status never contained \(fragment); value=\(String(describing: status.value))"
        )
    }

    private func horizontalScrollerMidY(in app: XCUIApplication) -> CGFloat {
        let scroller = app.scrollViews["fluid.fake.horizontal-scroll"]
        XCTAssertTrue(scroller.waitForExistence(timeout: 5))
        return scroller.frame.midY
    }

    private func attachStatus(from app: XCUIApplication, name: String) {
        let value = String(describing: app.staticTexts["fluid.fake.status"].value)
        let attachment = XCTAttachment(string: value)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func attachProductionStatus(from app: XCUIApplication, name: String) {
        let value = String(describing: app.staticTexts["production.route.status"].value)
        let attachment = XCTAttachment(string: value)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func waitForHittable(
        _ element: XCUIElement,
        named name: String,
        timeout: TimeInterval = 5
    ) {
        XCTAssertTrue(element.waitForExistence(timeout: timeout), name)
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "hittable == true"),
            object: element
        )
        XCTAssertEqual(XCTWaiter.wait(for: [expectation], timeout: timeout), .completed, name)
    }

    private func waitForNotHittable(
        _ element: XCUIElement,
        named name: String,
        timeout: TimeInterval = 5
    ) {
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "hittable == false"),
            object: element
        )
        XCTAssertEqual(XCTWaiter.wait(for: [expectation], timeout: timeout), .completed, name)
    }

    private func dragDrawerClosed(in app: XCUIApplication, rtl: Bool) {
        let origin = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        let startX = app.frame.width * (rtl ? 0.28 : 0.72)
        let endX = rtl ? app.frame.width - 45 : 45
        origin.withOffset(CGVector(dx: startX, dy: app.frame.height * 0.56)).press(
            forDuration: 0.05,
            thenDragTo: origin.withOffset(
                CGVector(dx: endX, dy: app.frame.height * 0.56)
            ),
            withVelocity: .fast,
            thenHoldForDuration: 0
        )
    }

    private func dragTaskTreeClosed(in app: XCUIApplication, rtl: Bool = false) {
        let origin = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        let startX = app.frame.width * (rtl ? 0.72 : 0.28)
        let endX = rtl ? 45 : app.frame.width - 45
        origin.withOffset(CGVector(dx: startX, dy: app.frame.height * 0.56)).press(
            forDuration: 0.05,
            thenDragTo: origin.withOffset(
                CGVector(dx: endX, dy: app.frame.height * 0.56)
            ),
            withVelocity: .fast,
            thenHoldForDuration: 0
        )
    }

    private func dragTrailingEdge(
        in app: XCUIApplication,
        fromInset: CGFloat,
        travel: CGFloat,
        velocity: XCUIGestureVelocity = .default,
        holdAtEnd: TimeInterval = 0,
        rtl: Bool = false
    ) {
        let origin = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        let startX = rtl ? fromInset : app.frame.width - fromInset
        let physicalTravel = rtl ? travel : -travel
        let y = app.frame.height * 0.56
        origin.withOffset(CGVector(dx: startX, dy: y)).press(
            forDuration: 0.05,
            thenDragTo: origin.withOffset(
                CGVector(dx: startX + physicalTravel, dy: y)
            ),
            withVelocity: velocity,
            thenHoldForDuration: holdAtEnd
        )
    }

    private func dragLeadingEdge(
        in app: XCUIApplication,
        fromInset: CGFloat,
        travel: CGFloat,
        velocity: XCUIGestureVelocity = .default,
        holdAtEnd: TimeInterval = 0,
        rtl: Bool = false,
        y: CGFloat? = nil
    ) {
        let origin = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        let startX = rtl ? app.frame.width - fromInset : fromInset
        let physicalTravel = rtl ? -travel : travel
        let gestureY = y ?? app.frame.height * 0.56
        let start = origin.withOffset(CGVector(dx: startX, dy: gestureY))
        let end = origin.withOffset(
            CGVector(dx: startX + physicalTravel, dy: gestureY)
        )
        start.press(
            forDuration: 0.05,
            thenDragTo: end,
            withVelocity: velocity,
            thenHoldForDuration: holdAtEnd
        )
    }

    private func averageBrightness(
        in image: UIImage,
        around point: CGPoint,
        appFrame: CGRect
    ) throws -> CGFloat {
        let cgImage = try XCTUnwrap(image.cgImage)
        let data = try XCTUnwrap(cgImage.dataProvider?.data)
        let bytes = try XCTUnwrap(CFDataGetBytePtr(data))
        let bytesPerPixel = cgImage.bitsPerPixel / 8
        guard bytesPerPixel >= 3 else {
            XCTFail("screenshot pixel format has fewer than three color bytes")
            return .nan
        }

        let scaleX = CGFloat(cgImage.width) / appFrame.width
        let scaleY = CGFloat(cgImage.height) / appFrame.height
        let centerX = Int((point.x - appFrame.minX) * scaleX)
        let centerY = Int((point.y - appFrame.minY) * scaleY)
        let radius = max(1, Int(scaleX.rounded()))
        var total: CGFloat = 0
        var count = 0

        for y in max(0, centerY - radius)...min(cgImage.height - 1, centerY + radius) {
            for x in max(0, centerX - radius)...min(cgImage.width - 1, centerX + radius) {
                let index = y * cgImage.bytesPerRow + x * bytesPerPixel
                total += (
                    CGFloat(bytes[index])
                        + CGFloat(bytes[index + 1])
                        + CGFloat(bytes[index + 2])
                ) / 3
                count += 1
            }
        }
        return total / CGFloat(count)
    }
}
