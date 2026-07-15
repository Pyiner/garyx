import XCTest

final class HomeListScrollPerformanceTests: XCTestCase {
    private struct ProbeReport {
        let hitchTimeRatio: Double
        let maxFrameInterval: Double
        let worstFrameDelta: Double
    }

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testHomeListScrollPerformanceWithVisibleRunningRows() throws {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        app.launchEnvironment["GARYX_MOBILE_HOME_SCROLL_PROBE"] = "1"
        app.launchEnvironment["GARYX_MOBILE_HOME_SCROLL_PROBE_MANUAL"] = "1"
        app.launchEnvironment["GARYX_MOBILE_PIN_REORDER_SPIKE"] = "1"
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

        let report = try recordProbeReport(app: app, scrollView: scrollView)
        print(
            "PROFILE explicit_probe hitch_time_ratio=\(report.hitchTimeRatio) max_frame_interval=\(report.maxFrameInterval) worst_frame_delta=\(report.worstFrameDelta)"
        )
        assertArchitectureGateThresholds(report)

        measure(
            metrics: scrollMetrics(for: app),
            options: measureOptions()
        ) {
            scrollView.swipeUp(velocity: .fast)
            Thread.sleep(forTimeInterval: 2.0)
            scrollView.swipeDown(velocity: .fast)
        }
    }

    private func assertArchitectureGateThresholds(
        _ report: ProbeReport,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        // Frozen before adapter wiring on the fixed iOS 26.5 / iPhone 17 Pro
        // simulator fixture (50 rows, six pinned). Relative gates allow normal
        // simulator noise while absolute gates prevent a noisy baseline from
        // masking a real frame-time regression.
        let baseline = ProbeReport(
            hitchTimeRatio: 0.04125509421993435,
            maxFrameInterval: 0.06539891667489428,
            worstFrameDelta: 0.04873224999755621
        )
        XCTAssertLessThanOrEqual(report.hitchTimeRatio, 0.075, file: file, line: line)
        XCTAssertLessThanOrEqual(
            report.hitchTimeRatio,
            baseline.hitchTimeRatio * 1.5 + 0.005,
            file: file,
            line: line
        )
        XCTAssertLessThanOrEqual(report.maxFrameInterval, 0.09, file: file, line: line)
        XCTAssertLessThanOrEqual(
            report.maxFrameInterval,
            baseline.maxFrameInterval * 1.25 + 0.008,
            file: file,
            line: line
        )
        XCTAssertLessThanOrEqual(report.worstFrameDelta, 0.075, file: file, line: line)
        XCTAssertLessThanOrEqual(
            report.worstFrameDelta,
            baseline.worstFrameDelta * 1.35 + 0.008,
            file: file,
            line: line
        )
    }

    private func recordProbeReport(
        app: XCUIApplication,
        scrollView: XCUIElement
    ) throws -> ProbeReport {
        let begin = app.buttons["home-scroll-probe-begin"]
        let end = app.buttons["home-scroll-probe-end"]
        XCTAssertTrue(begin.waitForExistence(timeout: 5))
        XCTAssertTrue(end.exists)
        begin.tap()

        scrollView.swipeUp(velocity: .fast)
        Thread.sleep(forTimeInterval: 0.4)
        scrollView.swipeUp(velocity: .fast)
        Thread.sleep(forTimeInterval: 0.4)
        scrollView.swipeDown(velocity: .fast)
        Thread.sleep(forTimeInterval: 0.4)
        scrollView.swipeDown(velocity: .fast)
        Thread.sleep(forTimeInterval: 0.8)
        end.tap()

        let reportElement = app.staticTexts["home-scroll-probe-report"]
        XCTAssertTrue(reportElement.waitForExistence(timeout: 5))
        let line = (reportElement.value as? String) ?? reportElement.label
        let fields = Dictionary(
            uniqueKeysWithValues: line.split(separator: " ").compactMap { field -> (String, Double)? in
                let parts = field.split(separator: "=", maxSplits: 1)
                guard parts.count == 2, let value = Double(parts[1]) else { return nil }
                return (String(parts[0]), value)
            }
        )
        return ProbeReport(
            hitchTimeRatio: try XCTUnwrap(fields["hitch_time_ratio"]),
            maxFrameInterval: try XCTUnwrap(fields["max_frame_interval"]),
            worstFrameDelta: try XCTUnwrap(fields["worst_frame_delta"])
        )
    }

    private func measureOptions() -> XCTMeasureOptions {
        let options = XCTMeasureOptions()
        // Four measured round trips plus the explicit four-swipe probe keep
        // this simulator gate below the UI-test runner's one-minute event-loop
        // monitoring instability while still exercising XCTHitchMetric.
        options.iterationCount = 4
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
