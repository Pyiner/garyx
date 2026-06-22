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

        XCTAssertTrue(app.scrollViews.firstMatch.waitForExistence(timeout: 10))
        let scrollView = try XCTUnwrap(visibleHomeScrollView(in: app))

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
        app.scrollViews.allElementsBoundByIndex
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
