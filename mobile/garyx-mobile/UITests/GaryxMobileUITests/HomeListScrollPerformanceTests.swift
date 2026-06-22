import XCTest

final class HomeListScrollPerformanceTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testHomeListScrollPerformanceWithVisibleRunningRows() throws {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        app.launchEnvironment["GARYX_MOBILE_HOME_SCROLL_PROBE"] = "1"
        app.launch()

        XCTAssertTrue(app.staticTexts["Garyx"].waitForExistence(timeout: 10))

        // The native List surfaces as a collectionView, so poll for the home
        // list container rather than `app.scrollViews` (which matched the old
        // ScrollView+LazyVStack but not a List).
        let deadline = Date().addingTimeInterval(10)
        var homeList = visibleHomeScrollView(in: app)
        while homeList == nil, Date() < deadline {
            Thread.sleep(forTimeInterval: 0.25)
            homeList = visibleHomeScrollView(in: app)
        }
        let scrollView = try XCTUnwrap(homeList)

        let runningRows = app.descendants(matching: .any).matching(identifier: "Running")
        print("PROFILE visible_running_accessibility_nodes=\(runningRows.count)")
        if runningRows.count == 0 {
            throw XCTSkip("Home list must show at least one active run row before this performance profile.")
        }
        print("PROFILE home_scroll_frame=\(scrollView.frame)")

        measure(
            metrics: scrollMetrics(for: app),
            options: measureOptions()
        ) {
            scrollView.swipeUp(velocity: .fast)
            Thread.sleep(forTimeInterval: 2.0)
            scrollView.swipeDown(velocity: .fast)
        }
    }

    private func measureOptions() -> XCTMeasureOptions {
        let options = XCTMeasureOptions()
        options.iterationCount = 8
        return options
    }

    private func scrollMetrics(for app: XCUIApplication) -> [any XCTMetric] {
        var metrics: [any XCTMetric] = [
            XCTOSSignpostMetric.scrollingAndDecelerationMetric,
            XCTClockMetric(),
            XCTCPUMetric(application: app)
        ]
        if #available(iOS 26.0, *) {
            metrics.insert(XCTHitchMetric(application: app), at: 0)
        }
        return metrics
    }

    private func visibleHomeScrollView(in app: XCUIApplication) -> XCUIElement? {
        // A native List surfaces as a collectionView (or table); the old
        // ScrollView+LazyVStack surfaced as a scrollView. Accept any so this
        // profile keeps working across the M6 container swap.
        let candidates = app.collectionViews.allElementsBoundByIndex
            + app.tables.allElementsBoundByIndex
            + app.scrollViews.allElementsBoundByIndex
        return candidates
            .filter { element in
                element.exists
                    && element.isHittable
                    && !element.frame.isEmpty
                    && app.frame.intersects(element.frame)
                    && element.frame.width > 200
                    && element.frame.height > 300
            }
            .max { lhs, rhs in
                lhs.frame.width * lhs.frame.height < rhs.frame.width * rhs.frame.height
            }
    }
}
