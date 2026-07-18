import XCTest
@testable import GaryxMobileCore

final class GaryxRouteRendererStateTests: XCTestCase {
    func testVisualPolicyTruthTableAndSessionFreeze() throws {
        let cases: [(Bool, Bool, GaryxRouteVisualPolicy)] = [
            (false, false, .spatial),
            (false, true, .crossFade),
            (true, false, .immediate),
            (true, true, .crossFade),
        ]
        for (reduceMotion, crossFade, expected) in cases {
            XCTAssertEqual(
                GaryxRouteVisualPreferences(
                    reduceMotion: reduceMotion,
                    prefersCrossFadeTransitions: crossFade
                ).resolvedPolicy,
                expected
            )
        }

        let source = node("source")
        let destination = node("destination")
        let frozen = try XCTUnwrap(GaryxRouteTransitionSession(
            kind: .pop,
            source: source,
            destination: destination,
            preferences: .init(reduceMotion: false, prefersCrossFadeTransitions: false)
        ))
        XCTAssertEqual(frozen.visualPolicy, .spatial)

        let next = try XCTUnwrap(GaryxRouteTransitionSession(
            kind: .pop,
            source: source,
            destination: destination,
            preferences: .init(reduceMotion: true, prefersCrossFadeTransitions: false)
        ))
        XCTAssertEqual(next.visualPolicy, .immediate)
        XCTAssertEqual(frozen.visualPolicy, .spatial, "preference changes affect only the next transaction")
    }

    func testSpatialPopIsOneToOneWithThirtyPercentParallaxAndRTLReflection() {
        let ltr = GaryxRouteTransitionGeometry.visualState(
            kind: .pop,
            policy: .spatial,
            progress: 0.4,
            viewportWidth: 400,
            layoutDirection: .leftToRight
        )
        XCTAssertEqual(ltr.sourceTranslationX, 160, accuracy: 1e-12)
        XCTAssertEqual(ltr.destinationTranslationX, -72, accuracy: 1e-12)
        XCTAssertEqual(ltr.scrimAlpha, 0.108, accuracy: 1e-12)
        XCTAssertGreaterThan(ltr.movingShadowOpacity, 0)
        XCTAssertLessThan(ltr.movingShadowOffsetX, 0)

        let rtl = GaryxRouteTransitionGeometry.visualState(
            kind: .pop,
            policy: .spatial,
            progress: 0.4,
            viewportWidth: 400,
            layoutDirection: .rightToLeft
        )
        XCTAssertEqual(rtl.sourceTranslationX, -ltr.sourceTranslationX, accuracy: 1e-12)
        XCTAssertEqual(rtl.destinationTranslationX, -ltr.destinationTranslationX, accuracy: 1e-12)
        XCTAssertEqual(rtl.movingShadowOffsetX, -ltr.movingShadowOffsetX, accuracy: 1e-12)
    }

    func testCrossFadeAndImmediateNeverMoveParallaxOrShadow() {
        for policy in [GaryxRouteVisualPolicy.crossFade, .immediate] {
            for progress in stride(from: CGFloat(0), through: 1, by: 0.1) {
                let state = GaryxRouteTransitionGeometry.visualState(
                    kind: .pop,
                    policy: policy,
                    progress: progress,
                    viewportWidth: 393,
                    layoutDirection: .leftToRight
                )
                XCTAssertEqual(state.sourceTranslationX, 0)
                XCTAssertEqual(state.destinationTranslationX, 0)
                XCTAssertEqual(state.scrimAlpha, 0)
                XCTAssertEqual(state.movingShadowOpacity, 0)
                XCTAssertEqual(state.movingShadowOffsetX, 0)
            }
        }
    }

    func testMeasuredFastFlickCommitsAndSlowMiddleDragCancels() {
        let width: CGFloat = 393
        XCTAssertTrue(GaryxRouteEdgeGestureArbitrator.shouldCommit(
            logicalTranslation: width * 0.1824,
            logicalVelocity: 300,
            viewportWidth: width
        ))
        XCTAssertFalse(GaryxRouteEdgeGestureArbitrator.shouldCommit(
            logicalTranslation: width * 0.3947,
            logicalVelocity: 0,
            viewportWidth: width
        ))
        XCTAssertFalse(GaryxRouteEdgeGestureArbitrator.shouldCommit(
            logicalTranslation: width * 0.5,
            logicalVelocity: 0,
            viewportWidth: width
        ), "commit is strictly a projected landing past halfway")
    }

    func testTouchDownSnapshotOwnsEdgeZoneInLTRAndRTL() {
        let ltrLeading = GaryxRouteEdgeTouchSnapshot(
            physicalX: 5,
            viewportWidth: 393,
            logicalEdge: .leading,
            layoutDirection: .leftToRight
        )
        XCTAssertTrue(ltrLeading.isInsideEdgeZone())
        XCTAssertTrue(GaryxRouteEdgeGestureArbitrator.shouldBegin(
            touch: ltrLeading,
            translation: CGSize(width: 25, height: 0),
            velocity: CGSize(width: 300, height: 0),
            modalBarrierActive: false,
            actionEligible: true
        ), "a touch that begins at 5 pt stays navigation-owned after moving to 25 pt")

        let movedIntoEdge = GaryxRouteEdgeTouchSnapshot(
            physicalX: 25,
            viewportWidth: 393,
            logicalEdge: .leading,
            layoutDirection: .leftToRight
        )
        XCTAssertFalse(movedIntoEdge.isInsideEdgeZone())
        XCTAssertFalse(GaryxRouteEdgeGestureArbitrator.shouldBegin(
            touch: movedIntoEdge,
            translation: CGSize(width: -20, height: 0),
            velocity: CGSize(width: 300, height: 0),
            modalBarrierActive: false,
            actionEligible: true
        ), "moving into the edge cannot rewrite the touch-down snapshot")

        let rtlLeading = GaryxRouteEdgeTouchSnapshot(
            physicalX: 390,
            viewportWidth: 393,
            logicalEdge: .leading,
            layoutDirection: .rightToLeft
        )
        XCTAssertTrue(GaryxRouteEdgeGestureArbitrator.shouldBegin(
            touch: rtlLeading,
            translation: CGSize(width: -25, height: 0),
            velocity: CGSize(width: -300, height: 0),
            modalBarrierActive: false,
            actionEligible: true
        ))
    }

    func testGestureCompetitionAndAxisTable() {
        for surface in [
            GaryxRouteGestureCompetitionSurface.horizontalScroll,
            .composerKeyboardDismiss,
            .rowSwipe,
        ] {
            XCTAssertEqual(
                GaryxRouteEdgeGestureArbitrator.winner(
                    surface: surface,
                    touchStartedInEdgeZone: true,
                    actionEligible: true
                ),
                .navigation
            )
            XCTAssertEqual(
                GaryxRouteEdgeGestureArbitrator.winner(
                    surface: surface,
                    touchStartedInEdgeZone: false,
                    actionEligible: true
                ),
                .descendant
            )
        }
        XCTAssertEqual(
            GaryxRouteEdgeGestureArbitrator.winner(
                surface: .modalPresentation,
                touchStartedInEdgeZone: true,
                actionEligible: true
            ),
            .modal
        )
        XCTAssertEqual(
            GaryxRouteEdgeGestureArbitrator.winner(
                surface: .taskTree,
                touchStartedInEdgeZone: true,
                actionEligible: true
            ),
            .taskTree
        )
        XCTAssertEqual(
            GaryxRouteEdgeGestureArbitrator.axis(
                translation: CGSize(width: 20, height: 100),
                velocity: .zero
            ),
            .vertical
        )
    }

    func testFourPhaseSessionAllowsOnlyCancelSettleRegrab() throws {
        let source = node("source")
        let destination = node("destination")
        var session = try XCTUnwrap(GaryxRouteTransitionSession(
            kind: .pop,
            source: source,
            destination: destination,
            preferences: .init(reduceMotion: false, prefersCrossFadeTransitions: false)
        ))
        XCTAssertEqual(session.coordinator.phase, .preCommit)
        XCTAssertTrue(session.update(progress: 0.3947))
        XCTAssertEqual(
            session.release(logicalTranslation: 394.7, logicalVelocity: 0, viewportWidth: 1_000),
            .cancelled
        )
        XCTAssertEqual(session.coordinator.phase, .cancelSettle)
        XCTAssertTrue(session.regrabCancelSettle(progress: 0.31))
        XCTAssertEqual(session.coordinator.phase, .preCommit)
        XCTAssertNil(session.settleTarget)

        XCTAssertEqual(
            session.release(logicalTranslation: 650, logicalVelocity: 0, viewportWidth: 1_000),
            .committed
        )
        XCTAssertEqual(session.coordinator.phase, .commitSettle)
        XCTAssertFalse(session.regrabCancelSettle(progress: 0.5))
        XCTAssertNotNil(session.finish(visibility: .visible))
        XCTAssertEqual(session.coordinator.terminalState?.outcome, .committed)
    }

    func testRecognizerCancellationAndVisibilityEventsUseA3DecisionTable() throws {
        var cancelled = try XCTUnwrap(GaryxRouteTransitionSession(
            kind: .pop,
            source: node("source"),
            destination: node("destination"),
            preferences: .init(reduceMotion: false, prefersCrossFadeTransitions: false)
        ))
        XCTAssertEqual(cancelled.handle(.recognizerCancelled), .transitioned(.cancelSettle))
        XCTAssertEqual(cancelled.settleTarget, 0)

        var inactive = try XCTUnwrap(GaryxRouteTransitionSession(
            kind: .pop,
            source: node("source"),
            destination: node("destination"),
            preferences: .init(reduceMotion: false, prefersCrossFadeTransitions: false)
        ))
        XCTAssertEqual(
            inactive.handle(.sceneInactive),
            .reachedTerminal(.init(outcome: .cancelled, visibility: .inactive))
        )
        XCTAssertEqual(inactive.coordinator.phase, .terminal)
    }

    func testSettleCalibrationStaysInsideMeasuredSystemWindow() {
        XCTAssertEqual(GaryxRouteTransitionCalibration.measuredRecognitionThreshold, 12.7)
        XCTAssertGreaterThanOrEqual(GaryxRouteTransitionCalibration.settleCurve.settlingDuration, 0.300)
        XCTAssertLessThanOrEqual(GaryxRouteTransitionCalibration.settleCurve.settlingDuration, 0.440)
    }

    func testStateStoreEvictsLRUButNeverPinnedEntries() {
        var store = GaryxRouteStateStore(
            maximumEvictableEntries: 2,
            maximumEvictableCostBytes: 10_000
        )
        let pinned = identity("pinned")
        store.setPinned(true, identity: pinned)
        store.set(.string("keep"), field: .draftSnapshot, identity: pinned)
        store.set(.string("one"), field: .draftSnapshot, identity: identity("one"))
        store.set(.string("two"), field: .draftSnapshot, identity: identity("two"))
        _ = store.value(field: .draftSnapshot, identity: identity("one"))
        store.set(.string("three"), field: .draftSnapshot, identity: identity("three"))

        XCTAssertEqual(store.metrics.pinnedEntryCount, 1)
        XCTAssertEqual(store.metrics.evictableEntryCount, 2)
        XCTAssertEqual(store.value(field: .draftSnapshot, identity: pinned), .string("keep"))
        XCTAssertEqual(store.value(field: .draftSnapshot, identity: identity("one")), .string("one"))
        XCTAssertNil(store.value(field: .draftSnapshot, identity: identity("two")))
        XCTAssertEqual(store.value(field: .draftSnapshot, identity: identity("three")), .string("three"))
    }

    func testPinnedOverflowReportsFaultWithoutEviction() {
        var store = GaryxRouteStateStore(
            maximumEvictableEntries: 1,
            maximumEvictableCostBytes: 64
        )
        for index in 0..<3 {
            let identity = identity("pinned-\(index)")
            store.setPinned(true, identity: identity)
            store.set(.string(String(repeating: "x", count: 100)), field: .draftSnapshot, identity: identity)
        }
        XCTAssertEqual(store.metrics.pinnedEntryCount, 3)
        XCTAssertGreaterThan(store.metrics.pinnedCostBytes, 64)
        XCTAssertGreaterThan(store.metrics.pinnedBudgetFaultCount, 0)
    }

    func testFiveHundredEntryChurnReachesBoundedSteadyState() {
        var store = GaryxRouteStateStore()
        for index in 0..<500 {
            store.set(
                .bytes(Data(repeating: UInt8(index % 255), count: 80_000)),
                field: .retiredSessionTombstone,
                identity: identity("route-\(index)")
            )
        }
        XCTAssertLessThanOrEqual(store.metrics.evictableEntryCount, 32)
        XCTAssertLessThanOrEqual(store.metrics.evictableCostBytes, 2 * 1_024 * 1_024)
        XCTAssertEqual(store.metrics.pinnedEntryCount, 0)
    }

    private func identity(_ value: String) -> GaryxRoutePresentationIdentity {
        .entry(.init(rawValue: value))
    }

    private func node(_ value: String) -> GaryxRoutePresentationNode {
        .entry(GaryxRouteEntry(
            id: .init(rawValue: value),
            destination: .panel(value)
        ))
    }
}
