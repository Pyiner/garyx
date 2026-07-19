import XCTest

final class HomeListScrollPerformanceTests: XCTestCase {
    private struct ProbeReport {
        let hitchTimeRatio: Double
        let maxFrameInterval: Double
        let worstFrameDelta: Double
    }

    private struct RoutePushProbeReport {
        let frameBudgetMilliseconds: Double
        let transitionFrameCount: Double
        let transitionHitchCount: Double
        let transitionMaximumIntervalMilliseconds: Double
        let beginToFirstTickMilliseconds: Double
        let maskedHitchCount: Double
        let postRevealHitchCount: Double
        let perceptibleHitchCount: Double
        let revealObserved: Double
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

    /// A4a route-entry gate: the existing scroll profile covers steady-state
    /// List drag, while this profile brackets the first and repeated
    /// list-to-conversation pushes with the production Release-capable frame
    /// probe. Expensive live transcript preparation may be reported as masked
    /// work behind the static staged surface. Retained moving transitions and
    /// every post-reveal window remain zero-budget; the cold open has an
    /// explicit two-cadence simulator ceiling for the XCTest/AX transaction.
    func testListToLongConversationPushStaysWithinFrameBudget() {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_FIXTURE"] = "long"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_PROBE"] = "1"
        app.launch()

        XCTAssertTrue(app.staticTexts["Garyx"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Thread History"].waitForExistence(timeout: 10))

        var reports: [RoutePushProbeReport] = []
        var expectedTransaction = 0
        let options = XCTMeasureOptions()
        // The cold push has its own in-app CADisplayLink budget below. Start
        // XCTHitchMetric only after it so metric-collector bootstrap cannot be
        // mistaken for app work in the first route transaction.
        options.iterationCount = 2

        let openAndReturn: () -> Void = {
            expectedTransaction += 1
            let row = app.staticTexts["Thread History"]
            XCTAssertTrue(row.waitForExistence(timeout: 5))
            row.tap()

            guard let report = self.waitForRoutePushReport(
                transaction: expectedTransaction,
                in: app
            ) else {
                XCTFail("route push probe did not finish transaction \(expectedTransaction)")
                return
            }
            reports.append(report)

            let back = app.buttons["Back"]
            XCTAssertTrue(back.waitForExistence(timeout: 5))
            back.tap()
            XCTAssertTrue(app.staticTexts["Thread History"].waitForExistence(timeout: 5))
        }

        openAndReturn()
        measure(
            metrics: [
                XCTHitchMetric(application: app),
                XCTClockMetric(),
                XCTCPUMetric(application: app),
            ],
            options: options
        ) {
            openAndReturn()
        }

        // XCTest adds one discarded warm-up invocation to the two measured
        // repeats, in addition to the explicit cold push above.
        XCTAssertEqual(reports.count, options.iterationCount + 2)
        for (index, report) in reports.enumerated() {
            let profile = index == 0 ? "cold" : "repeat_\(index)"
            print(
                "PROFILE route_push_\(profile) budget_ms=\(report.frameBudgetMilliseconds) begin_to_first_tick_ms=\(report.beginToFirstTickMilliseconds) transition_frames=\(report.transitionFrameCount) transition_hitches=\(report.transitionHitchCount) transition_max_ms=\(report.transitionMaximumIntervalMilliseconds) masked_hitches=\(report.maskedHitchCount) post_reveal_hitches=\(report.postRevealHitchCount) perceptible_hitches=\(report.perceptibleHitchCount)"
            )
            assertRoutePushArchitectureGate(report, profile: profile)
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

    private func waitForRoutePushReport(
        transaction: Int,
        in app: XCUIApplication
    ) -> RoutePushProbeReport? {
        let reportElement = app.staticTexts["route-push-probe-report"]
        guard reportElement.waitForExistence(timeout: 5) else { return nil }
        let transactionMarker = "transaction=\(transaction) "
        let deadline = Date().addingTimeInterval(12)
        while Date() < deadline {
            let line = (reportElement.value as? String) ?? reportElement.label
            if line.contains(transactionMarker), line.contains("perceptible_hitch_count=") {
                print("PROFILE route_push_raw \(line)")
                let fields = Dictionary(
                    uniqueKeysWithValues: line.split(separator: " ").compactMap {
                        field -> (String, Double)? in
                        let parts = field.split(separator: "=", maxSplits: 1)
                        guard parts.count == 2, let value = Double(parts[1]) else { return nil }
                        return (String(parts[0]), value)
                    }
                )
                guard let frameBudget = fields["frame_budget_ms"],
                      let transitionFrames = fields["transition_frame_count"],
                      let transitionHitches = fields["transition_hitch_count"],
                      let transitionMaximum = fields["transition_max_interval_ms"],
                      let beginToFirstTick = fields["begin_to_first_tick_ms"],
                      let maskedHitches = fields["masked_hitch_count"],
                      let postRevealHitches = fields["post_reveal_hitch_count"],
                      let perceptibleHitches = fields["perceptible_hitch_count"],
                      let revealObserved = fields["reveal_observed"] else {
                    return nil
                }
                return RoutePushProbeReport(
                    frameBudgetMilliseconds: frameBudget,
                    transitionFrameCount: transitionFrames,
                    transitionHitchCount: transitionHitches,
                    transitionMaximumIntervalMilliseconds: transitionMaximum,
                    beginToFirstTickMilliseconds: beginToFirstTick,
                    maskedHitchCount: maskedHitches,
                    postRevealHitchCount: postRevealHitches,
                    perceptibleHitchCount: perceptibleHitches,
                    revealObserved: revealObserved
                )
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        return nil
    }

    private func assertRoutePushArchitectureGate(
        _ report: RoutePushProbeReport,
        profile: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let isCold = profile == "cold"
        // The first simulator push is also the first CoreAnimation transaction
        // observed by the connected XCUI/AX process. Give that one sample a
        // hard two-cadence ceiling; retained pushes remain zero-hitch gates.
        // A regression to the former 173 ms stall still exceeds this budget by
        // more than 5x. Release real-gateway captures enforce the zero-hitch
        // product target independently of this instrumented simulator bound.
        let firstTickMultiplier = isCold ? 2.25 : 1.5
        let maximumIntervalMultiplier = isCold ? 2.0 : 1.5
        let allowedHitches: Double = isCold ? 1 : 0

        XCTAssertGreaterThanOrEqual(
            report.transitionFrameCount,
            12,
            "\(profile) push did not sample the complete transition",
            file: file,
            line: line
        )
        XCTAssertLessThanOrEqual(
            report.beginToFirstTickMilliseconds,
            report.frameBudgetMilliseconds * firstTickMultiplier + 0.25,
            "\(profile) push missed its first presentation budget",
            file: file,
            line: line
        )
        XCTAssertLessThanOrEqual(
            report.transitionHitchCount,
            allowedHitches,
            file: file,
            line: line
        )
        XCTAssertLessThanOrEqual(
            report.transitionMaximumIntervalMilliseconds,
            report.frameBudgetMilliseconds * maximumIntervalMultiplier + 0.25,
            "\(profile) push exceeded the delivered-frame hitch threshold",
            file: file,
            line: line
        )
        XCTAssertEqual(report.postRevealHitchCount, 0, file: file, line: line)
        XCTAssertLessThanOrEqual(
            report.perceptibleHitchCount,
            allowedHitches,
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.revealObserved,
            1,
            "\(profile) push never presented prepared content",
            file: file,
            line: line
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
        [
            XCTHitchMetric(application: app),
            XCTOSSignpostMetric.scrollingAndDecelerationMetric,
            XCTClockMetric(),
            XCTCPUMetric(application: app),
        ]
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
