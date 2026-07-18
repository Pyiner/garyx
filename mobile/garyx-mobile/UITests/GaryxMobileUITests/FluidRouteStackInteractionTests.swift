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
            velocity: .slow,
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

    // MARK: Helpers

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
}
