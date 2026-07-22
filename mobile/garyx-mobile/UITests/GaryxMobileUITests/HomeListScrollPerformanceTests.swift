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
        let postTerminalHitchCount: Double
        let maskedMaterializationHitchCount: Double
        let postRevealHitchCount: Double
        let perceptibleHitchCount: Double
        let openingPageChromeObserved: Double
        let conversationSurfaceObserved: Double
        let fullPagePlaceholderObserved: Double
        let messageRegionLoadingObserved: Double
        let localMessageContentObserved: Double
        let headerLoadingIndicatorObserved: Double
        let liveRevealObserved: Double
        let messagePreparationCompleted: Double
        let prewarmReadyAtPush: Double
    }

    private struct SendJitterProbeReport {
        let rawLine: String
        let frameCount: Double
        let netY: Double
        let travelY: Double
        let excessY: Double
        let upwardY: Double
        let downwardY: Double
        let directionReversals: Double
        let viewportDelta: Double
        let contentDelta: Double
        let bottomInsetDelta: Double
        let optimisticMilliseconds: Double
        let committedMilliseconds: Double
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
    /// probe. The real conversation page must be the moving destination and
    /// startup prewarming must keep both the transition and visible message
    /// preparation within budget. The in-app display-link probe is the gate;
    /// wrapping these same pushes in XCTHitchMetric perturbs the main process
    /// and can manufacture hitches in the transaction it is meant to observe.
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

            // iOS 26 can publish one SwiftUI/Liquid Glass button through both
            // its legacy Button and modern PopUpButton automation adapters.
            // They resolve to the same frame/action; selecting the first
            // adapter avoids treating that XCTest bridge duplication as two
            // product controls.
            let back = app.buttons["Back"].firstMatch
            XCTAssertTrue(back.waitForExistence(timeout: 5))
            back.tap()
            XCTAssertTrue(app.staticTexts["Thread History"].waitForExistence(timeout: 5))
        }

        openAndReturn()
        for _ in 0..<3 {
            openAndReturn()
        }

        XCTAssertEqual(reports.count, 4)
        for (index, report) in reports.enumerated() {
            let profile = index == 0 ? "cold" : "repeat_\(index)"
            print(
                "PROFILE route_push_\(profile) budget_ms=\(report.frameBudgetMilliseconds) begin_to_first_tick_ms=\(report.beginToFirstTickMilliseconds) transition_frames=\(report.transitionFrameCount) transition_hitches=\(report.transitionHitchCount) transition_max_ms=\(report.transitionMaximumIntervalMilliseconds) masked_materialization_hitches=\(report.maskedMaterializationHitchCount) post_reveal_hitches=\(report.postRevealHitchCount) perceptible_hitches=\(report.perceptibleHitchCount) local_messages=\(report.localMessageContentObserved) message_loading=\(report.messageRegionLoadingObserved) header_spinner=\(report.headerLoadingIndicatorObserved) prewarm_ready=\(report.prewarmReadyAtPush)"
            )
            assertRoutePushArchitectureGate(report, profile: profile)
        }
    }

    func testEmptyConversationPushUsesOnlyMessageRegionLoading() {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_FIXTURE"] = "empty"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_PROBE"] = "1"
        app.launch()

        XCTAssertTrue(app.staticTexts["Garyx"].waitForExistence(timeout: 10))
        let row = app.staticTexts["Thread History"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()

        guard let report = waitForRoutePushReport(transaction: 1, in: app) else {
            XCTFail("empty-cache route push probe did not finish")
            return
        }
        assertRoutePushArchitectureGate(report, profile: "cold")
        XCTAssertEqual(
            report.localMessageContentObserved,
            0,
            "an empty local transcript must not fabricate cached message content"
        )
        XCTAssertEqual(
            report.messageRegionLoadingObserved,
            1,
            "only the empty message region may show the shared loading skeleton"
        )
    }

    func testExistingThreadComposerIsLiveWhileHistoryLoads() {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] = "1"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_FIXTURE"] = "empty"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_PROBE"] = "1"
        app.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_DIAGNOSTICS"] = "1"
        app.launch()

        let row = app.staticTexts["Thread History"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()

        let composer = app.textViews["garyx-composer-uikit-input"]
        XCTAssertTrue(composer.waitForExistence(timeout: 1))
        XCTAssertEqual(composer.label, "composer-live")

        composer.tap()
        waitForKeyboard(true, in: app)
        let previousValue = composer.value as? String
        let key = app.keys.matching(
            NSPredicate(format: "label ==[c] %@", "x")
        ).firstMatch
        XCTAssertTrue(key.waitForExistence(timeout: 2))
        key.tap()
        let inputExpectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value != %@", previousValue ?? ""),
            object: composer
        )
        XCTAssertEqual(XCTWaiter.wait(for: [inputExpectation], timeout: 3), .completed)

        guard let report = waitForRoutePushReport(transaction: 1, in: app) else {
            XCTFail("route push probe did not finish the first-frame composer validation")
            return
        }
        XCTAssertEqual(
            report.messageRegionLoadingObserved,
            1,
            "the same push must exercise the zero-local-row history-loading state"
        )

        let send = app.buttons.matching(identifier: "arrow.up").firstMatch
        XCTAssertTrue(send.waitForExistence(timeout: 2))
        XCTAssertTrue(send.isEnabled)
        XCTAssertTrue(send.isHittable)
        let typedValue = composer.value as? String ?? ""
        send.tap()
        let sendExpectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value != %@", typedValue),
            object: composer
        )
        XCTAssertEqual(XCTWaiter.wait(for: [sendExpectation], timeout: 3), .completed)
    }

    func testConversationKeyboardDismissesFromBlankBackgroundAndScroll() {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "chat"
        app.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_DIAGNOSTICS"] = "1"
        app.launch()

        let composer = app.textViews["garyx-composer-uikit-input"]
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        XCTAssertEqual(composer.label, "composer-live")
        composer.tap()
        waitForKeyboard(true, in: app)

        let lastMessage = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "Wraps cleanly")
        ).firstMatch
        XCTAssertTrue(lastMessage.waitForExistence(timeout: 2))
        let blankY = (lastMessage.frame.maxY + composer.frame.minY) / 2
        app.coordinate(
            withNormalizedOffset: CGVector(
                dx: 0.5,
                dy: blankY / app.frame.height
            )
        ).tap()
        waitForKeyboard(false, in: app)

        let userMessage = app.staticTexts["Type check"]
        XCTAssertTrue(userMessage.waitForExistence(timeout: 2))
        userMessage.press(forDuration: 0.6)
        for action in ["Copy", "Select Text", "Share"] {
            XCTAssertTrue(
                app.buttons[action].waitForExistence(timeout: 2),
                "the transcript background tap owner must not steal the \(action) long-press action"
            )
        }

        app.terminate()

        let scrollingApp = XCUIApplication()
        scrollingApp.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        scrollingApp.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "chat"
        scrollingApp.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_FIXTURE"] = "send-jitter"
        scrollingApp.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_DIAGNOSTICS"] = "1"
        scrollingApp.launch()

        let scrollingComposer = scrollingApp.textViews["garyx-composer-uikit-input"]
        XCTAssertTrue(scrollingComposer.waitForExistence(timeout: 10))
        scrollingComposer.tap()
        waitForKeyboard(true, in: scrollingApp)
        let transcript = scrollingApp.scrollViews["garyx-conversation-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 2))
        XCTAssertTrue(transcript.isHittable)
        let keyboardFrame = scrollingApp.keyboards.firstMatch.frame
        let start = scrollingApp.coordinate(
            withNormalizedOffset: CGVector(
                dx: 0.5,
                dy: transcript.frame.midY / scrollingApp.frame.height
            )
        )
        let end = scrollingApp.coordinate(
            withNormalizedOffset: CGVector(
                dx: 0.5,
                dy: (keyboardFrame.minY + keyboardFrame.height * 0.6)
                    / scrollingApp.frame.height
            )
        )
        start.press(
            forDuration: 0.05,
            thenDragTo: end,
            withVelocity: .slow,
            thenHoldForDuration: 0.1
        )
        waitForKeyboard(false, in: scrollingApp)
    }

    /// Sanitized simulator replay of the reported send shape: 21 committed
    /// turns, a four-line composer, optimistic origin insertion, then the same
    /// origin materialized by the captured committed render row. The probe
    /// follows turn 21 on every display frame. A stable send can move it in
    /// one direction to make room; any excess round trip is the whole-list
    /// shake reported in TASK-2523.
    ///
    /// The round-trip assertion failed on the TASK-2523 baseline and stays as
    /// the frame-level regression gate.
    func testSendDoesNotRoundTripExistingTranscriptRows() throws {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] = "1"
        app.launchEnvironment["GARYX_MOBILE_DEBUG_PANEL"] = "chat"
        app.launchEnvironment["GARYX_MOBILE_ROUTE_PUSH_FIXTURE"] = "send-jitter"
        app.launchEnvironment["GARYX_MOBILE_SEND_JITTER_PROBE"] = "1"
        app.launchEnvironment["GARYX_MOBILE_SEND_JITTER_FIXTURE"] = "1"
        app.launchEnvironment["GARYX_MOBILE_PRODUCTION_ROUTE_DIAGNOSTICS"] = "1"
        app.launch()

        XCTAssertTrue(app.buttons["Back"].waitForExistence(timeout: 10))
        let composer = app.textViews["garyx-composer-uikit-input"]
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        let live = NSPredicate(format: "label == %@", "composer-live")
        XCTAssertEqual(
            XCTWaiter.wait(
                for: [XCTNSPredicateExpectation(predicate: live, object: composer)],
                timeout: 10
            ),
            .completed,
            "production composer never received live adapter ownership"
        )
        let send = app.buttons["Send"]
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        XCTAssertTrue(send.isEnabled)
        send.tap()

        let report = try XCTUnwrap(waitForSendJitterReport(in: app))
        print(
            "PROFILE send_jitter frame_count=\(report.frameCount) net_y=\(report.netY) travel_y=\(report.travelY) excess_y=\(report.excessY) up_y=\(report.upwardY) down_y=\(report.downwardY) reversals=\(report.directionReversals) viewport_delta=\(report.viewportDelta) content_delta=\(report.contentDelta) bottom_inset_delta=\(report.bottomInsetDelta) optimistic_ms=\(report.optimisticMilliseconds) committed_ms=\(report.committedMilliseconds)"
        )
        XCTAssertGreaterThanOrEqual(report.frameCount, 20)
        XCTAssertGreaterThan(report.contentDelta, 0)
        XCTAssertLessThanOrEqual(
            report.excessY,
            1,
            "FAILS ON BASELINE: an existing turn must not make a visible round trip while the new tail row materializes; \(report.rawLine)"
        )
        XCTAssertGreaterThan(report.upwardY, 0)
        XCTAssertEqual(report.downwardY, 0, accuracy: 0.5)
        XCTAssertEqual(report.directionReversals, 0)
    }

    private func waitForKeyboard(
        _ expected: Bool,
        in app: XCUIApplication,
        timeout: TimeInterval = 3,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == %@", NSNumber(value: expected)),
            object: app.keyboards.firstMatch
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation], timeout: timeout),
            .completed,
            "software keyboard existence did not become \(expected)",
            file: file,
            line: line
        )
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
                      let postTerminalHitches = fields["post_terminal_hitch_count"],
                      let maskedMaterializationHitches = fields["masked_materialization_hitch_count"],
                      let postRevealHitches = fields["post_reveal_hitch_count"],
                      let perceptibleHitches = fields["perceptible_hitch_count"],
                      let openingPageChromeObserved = fields["opening_page_chrome_observed"],
                      let conversationSurfaceObserved = fields["conversation_surface_observed"],
                      let fullPagePlaceholderObserved = fields["full_page_placeholder_observed"],
                      let messageRegionLoadingObserved = fields["message_region_loading_observed"],
                      let localMessageContentObserved = fields["local_message_content_observed"],
                      let headerLoadingIndicatorObserved = fields["header_loading_indicator_observed"],
                      let liveRevealObserved = fields["live_reveal_observed"],
                      let messagePreparationCompleted = fields["message_preparation_completed"],
                      let prewarmReadyAtPush = fields["prewarm_ready_at_push"] else {
                    return nil
                }
                return RoutePushProbeReport(
                    frameBudgetMilliseconds: frameBudget,
                    transitionFrameCount: transitionFrames,
                    transitionHitchCount: transitionHitches,
                    transitionMaximumIntervalMilliseconds: transitionMaximum,
                    beginToFirstTickMilliseconds: beginToFirstTick,
                    postTerminalHitchCount: postTerminalHitches,
                    maskedMaterializationHitchCount: maskedMaterializationHitches,
                    postRevealHitchCount: postRevealHitches,
                    perceptibleHitchCount: perceptibleHitches,
                    openingPageChromeObserved: openingPageChromeObserved,
                    conversationSurfaceObserved: conversationSurfaceObserved,
                    fullPagePlaceholderObserved: fullPagePlaceholderObserved,
                    messageRegionLoadingObserved: messageRegionLoadingObserved,
                    localMessageContentObserved: localMessageContentObserved,
                    headerLoadingIndicatorObserved: headerLoadingIndicatorObserved,
                    liveRevealObserved: liveRevealObserved,
                    messagePreparationCompleted: messagePreparationCompleted,
                    prewarmReadyAtPush: prewarmReadyAtPush
                )
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        return nil
    }

    private func waitForSendJitterReport(
        in app: XCUIApplication
    ) -> SendJitterProbeReport? {
        let reportElement = app.staticTexts["send-jitter-probe-report"]
        guard reportElement.waitForExistence(timeout: 5) else { return nil }
        let deadline = Date().addingTimeInterval(8)
        while Date() < deadline {
            let line = (reportElement.value as? String) ?? reportElement.label
            if line.contains("transaction=1 "), line.contains("excess_y=") {
                print("PROFILE send_jitter_raw \(line)")
                let fields = Dictionary(
                    uniqueKeysWithValues: line.split(separator: " ").compactMap {
                        field -> (String, Double)? in
                        let parts = field.split(separator: "=", maxSplits: 1)
                        guard parts.count == 2, let value = Double(parts[1]) else { return nil }
                        return (String(parts[0]), value)
                    }
                )
                guard let frameCount = fields["frame_count"],
                      let netY = fields["net_y"],
                      let travelY = fields["travel_y"],
                      let excessY = fields["excess_y"],
                      let upwardY = fields["up_y"],
                      let downwardY = fields["down_y"],
                      let directionReversals = fields["direction_reversals"],
                      let viewportDelta = fields["viewport_delta"],
                      let contentDelta = fields["content_delta"],
                      let bottomInsetDelta = fields["bottom_inset_delta"],
                      let optimisticMilliseconds = fields["optimistic_ms"],
                      let committedMilliseconds = fields["committed_ms"] else {
                    return nil
                }
                return SendJitterProbeReport(
                    rawLine: line,
                    frameCount: frameCount,
                    netY: netY,
                    travelY: travelY,
                    excessY: excessY,
                    upwardY: upwardY,
                    downwardY: downwardY,
                    directionReversals: directionReversals,
                    viewportDelta: viewportDelta,
                    contentDelta: contentDelta,
                    bottomInsetDelta: bottomInsetDelta,
                    optimisticMilliseconds: optimisticMilliseconds,
                    committedMilliseconds: committedMilliseconds
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
        // Constructing the complete first thread-page accessibility graph is
        // part of the connected simulator's first presentation. Bound that
        // cold work to four 60 Hz cadences and retained opens to three; the
        // former 173 ms stall still fails by more than 2.5x. Once the first
        // frame is delivered, every visible transition interval remains a
        // zero-hitch gate.
        let firstTickMultiplier = isCold ? 4.0 : 3.0
        let maximumIntervalMultiplier = isCold ? 2.0 : 1.5
        let allowedHitches: Double = isCold ? 1 : 0

        print(
            "PROFILE route_push_gate_\(profile) first_tick_multiplier=\(firstTickMultiplier) allowed_hitches=\(allowedHitches)"
        )

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
        // One-time AttributeGraph/Metal work may occur only while the stable
        // complete thread page is still on top. Once the live tree takes over,
        // message-region loading and content insertion are zero-hitch gates.
        XCTAssertEqual(report.postRevealHitchCount, 0, file: file, line: line)
        XCTAssertLessThanOrEqual(
            report.perceptibleHitchCount,
            allowedHitches,
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.openingPageChromeObserved,
            1,
            "\(profile) push did not present complete thread chrome from its first destination frame",
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.conversationSurfaceObserved,
            1,
            "\(profile) push never mounted the real conversation surface",
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.fullPagePlaceholderObserved,
            0,
            "\(profile) push presented a forbidden full-page placeholder",
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.localMessageContentObserved + report.messageRegionLoadingObserved,
            1,
            "\(profile) push must show cached messages, or a message-only skeleton when local data is empty",
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.headerLoadingIndicatorObserved,
            1,
            "\(profile) push did not preserve the thread-header loading spinner",
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.liveRevealObserved,
            1,
            "\(profile) push never handed off to the stable live conversation",
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.messagePreparationCompleted,
            1,
            "\(profile) push did not finish initial message preparation",
            file: file,
            line: line
        )
        XCTAssertEqual(
            report.prewarmReadyAtPush,
            1,
            "\(profile) push began before conversation rendering was prewarmed",
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
