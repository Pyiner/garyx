import Combine
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxHorizontalRevealInteractionTests: XCTestCase {
    func testReleaseVelocityIsInjectedIntoAnalyticSettle() throws {
        let harness = Harness(projection: .fullScreenNavigation)
        harness.store.configure(extent: 330, restingPosition: .closed)
        harness.store.beginGesture()
        harness.store.updateGesture(logicalTranslation: 60)

        XCTAssertEqual(harness.store.endGesture(logicalVelocity: 300), .open)
        XCTAssertEqual(harness.store.presentation.phase, .settling(.open))
        XCTAssertEqual(harness.store.reveal, 60, accuracy: 1e-9)

        harness.advance(by: 0.06)
        let expected = GaryxMotionPhysics.SettleTrajectory(
            initialValue: 60,
            targetValue: 330,
            initialVelocity: 300,
            curve: GaryxRouteTransitionCalibration.settleCurve
        ).sample(elapsedTime: 0.06)
        XCTAssertEqual(
            harness.store.reveal,
            GaryxHorizontalRevealState.rubberBandedReveal(expected.value, extent: 330),
            accuracy: 1e-8
        )
    }

    func testFullAndShortTravelSettlesMatchEvery120HzAnalyticFrame() throws {
        let scenarios: [(
            projection: GaryxMotionPhysics.ProjectionPolicy,
            extent: CGFloat,
            reveal: CGFloat,
            velocity: CGFloat
        )] = [
            (.fullScreenNavigation, 330, 60, 300),
            (.shortTravelDismiss, 106, 18, 220),
        ]
        let curve = GaryxRouteTransitionCalibration.settleCurve
        let frameInterval = 1.0 / 120.0

        for scenario in scenarios {
            let harness = Harness(projection: scenario.projection)
            harness.store.configure(extent: scenario.extent, restingPosition: .closed)
            harness.store.beginGesture()
            harness.store.updateGesture(logicalTranslation: scenario.reveal)
            XCTAssertEqual(harness.store.endGesture(logicalVelocity: scenario.velocity), .open)

            let trajectory = GaryxMotionPhysics.SettleTrajectory(
                initialValue: scenario.reveal,
                targetValue: scenario.extent,
                initialVelocity: scenario.velocity,
                curve: curve
            )
            let frameCount = Int(ceil(curve.settlingDuration / frameInterval))
            for frame in 1...frameCount {
                harness.advance(by: frameInterval)
                let elapsed = Double(frame) * frameInterval
                if elapsed >= curve.settlingDuration {
                    XCTAssertEqual(harness.store.reveal, scenario.extent, accuracy: 1e-8)
                    XCTAssertEqual(harness.store.presentation.phase, .idle)
                } else {
                    let expected = trajectory.sample(elapsedTime: elapsed)
                    XCTAssertEqual(
                        harness.store.reveal,
                        GaryxHorizontalRevealState.rubberBandedReveal(
                            expected.value,
                            extent: scenario.extent
                        ),
                        accuracy: 1e-8,
                        "frame \(frame), projection \(scenario.projection)"
                    )
                    XCTAssertEqual(harness.store.presentation.phase, .settling(.open))
                }
            }
            XCTAssertFalse(harness.frames.isRunning)
        }
    }

    func testProgrammaticSettleUsesCriticallyDampedToken() {
        let harness = Harness(projection: .fullScreenNavigation)
        harness.store.configure(extent: 330, restingPosition: .closed)
        harness.store.setTarget(.open, animated: true)

        harness.advance(by: 0.06)
        let curve = GaryxRouteTransitionCalibration.programmaticSettleCurve
        let expected = GaryxMotionPhysics.SettleTrajectory(
            initialValue: 0,
            targetValue: 330,
            initialVelocity: 0,
            curve: curve
        ).sample(elapsedTime: 0.06)

        XCTAssertEqual(curve.dampingRatio, 1)
        XCTAssertEqual(
            harness.store.reveal,
            GaryxHorizontalRevealState.rubberBandedReveal(expected.value, extent: 330),
            accuracy: 1e-8
        )
    }

    func testSettleRegrabAdoptsDrawnValueAndCanReverseTarget() throws {
        let harness = Harness(projection: .fullScreenNavigation)
        harness.store.configure(extent: 320, restingPosition: .closed)
        harness.store.beginGesture()
        harness.store.updateGesture(logicalTranslation: 80)
        XCTAssertEqual(harness.store.endGesture(logicalVelocity: 420), .open)

        harness.advance(by: 0.08)
        let valueAtInterrupt = harness.store.reveal
        harness.store.beginGesture()

        XCTAssertEqual(harness.store.presentation.phase, .dragging)
        XCTAssertEqual(harness.store.reveal, valueAtInterrupt, accuracy: 1e-8)
        XCTAssertFalse(harness.frames.isRunning)

        harness.store.updateGesture(logicalTranslation: -valueAtInterrupt * 0.8)
        XCTAssertEqual(harness.store.endGesture(logicalVelocity: -300), .closed)
        XCTAssertEqual(harness.store.presentation.phase, .settling(.closed))
    }

    func testCancellationAfterRegrabResumesInterruptedEndpoint() {
        let harness = Harness(projection: .fullScreenNavigation)
        harness.store.configure(extent: 300, restingPosition: .closed)
        harness.store.beginGesture()
        harness.store.updateGesture(logicalTranslation: 210)
        XCTAssertEqual(harness.store.endGesture(logicalVelocity: 0), .open)
        harness.advance(by: 0.05)

        harness.store.beginGesture()
        harness.store.updateGesture(logicalTranslation: -50)
        XCTAssertEqual(harness.store.cancelGesture(), .open)
        XCTAssertEqual(harness.store.presentation.phase, .settling(.open))
    }

    func testImmediateAccessibilityTargetInterruptsSettle() {
        let harness = Harness(projection: .shortTravelDismiss)
        harness.store.configure(extent: 106, restingPosition: .closed)
        harness.store.setTarget(.open, animated: true, initialVelocity: 240)
        XCTAssertTrue(harness.frames.isRunning)

        harness.store.setTarget(.closed, animated: false)

        XCTAssertFalse(harness.frames.isRunning)
        XCTAssertEqual(harness.store.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .closed
        ))
    }

    func testZeroExtentCannotRetainASettleWithoutAFrameDriver() {
        let harness = Harness(projection: .fullScreenNavigation)
        harness.store.configure(extent: 330, restingPosition: .closed)
        harness.store.setTarget(.open, animated: true)
        XCTAssertEqual(harness.store.presentation.phase, .settling(.open))
        XCTAssertTrue(harness.frames.isRunning)

        harness.store.configure(extent: 0, restingPosition: .open)

        XCTAssertEqual(harness.store.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .open
        ))
        XCTAssertFalse(harness.frames.isRunning)
    }

    func testGeometryChangeTerminatesDragAtCanonicalEndpoint() {
        let harness = Harness(projection: .fullScreenNavigation)
        harness.store.configure(extent: 330, restingPosition: .open)
        harness.store.beginGesture()
        harness.store.updateGesture(logicalTranslation: -170)
        XCTAssertEqual(harness.store.presentation.phase, .dragging)

        harness.store.configure(extent: 440, restingPosition: .open)

        XCTAssertEqual(harness.store.presentation, .init(
            reveal: 440,
            phase: .idle,
            target: .open
        ))
        XCTAssertFalse(harness.frames.isRunning)
        XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)
    }

    func testEveryExternalInvalidationStopsDriverAndReleasesPhase() {
        for invalidation in GaryxHorizontalRevealInvalidation.allCases {
            let harness = Harness(projection: .fullScreenNavigation)
            harness.store.configure(extent: 330, restingPosition: .closed)
            harness.store.beginGesture()
            harness.store.updateGesture(logicalTranslation: 140)

            harness.store.forceTerminal(invalidation, position: .closed)

            XCTAssertEqual(harness.store.presentation, .init(
                reveal: 0,
                phase: .idle,
                target: .closed
            ), "dragging / \(invalidation)")
            XCTAssertFalse(harness.frames.isRunning, "dragging / \(invalidation)")
            XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)

            harness.store.setTarget(.open, animated: true)
            XCTAssertTrue(harness.frames.isRunning)
            harness.store.forceTerminal(invalidation, position: .open)

            XCTAssertEqual(harness.store.presentation, .init(
                reveal: 330,
                phase: .idle,
                target: .open
            ), "settling / \(invalidation)")
            XCTAssertFalse(harness.frames.isRunning, "settling / \(invalidation)")
            XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)
        }
    }

    func testRevealInvalidationStressHasZeroSteadyStateResidue() {
        let harness = Harness(projection: .fullScreenNavigation)
        harness.store.configure(extent: 330, restingPosition: .closed)
        let invalidations = GaryxHorizontalRevealInvalidation.allCases
        var position = GaryxHorizontalRevealPosition.closed

        for iteration in 0..<1_000 {
            let nextPosition: GaryxHorizontalRevealPosition = position == .closed
                ? .open
                : .closed
            harness.store.setTarget(nextPosition, animated: true)
            XCTAssertTrue(harness.frames.isRunning, "settle iteration \(iteration)")
            if iteration.isMultiple(of: 3) {
                harness.advance(by: 1.0 / 120.0)
            }
            harness.store.forceTerminal(
                invalidations[iteration % invalidations.count],
                position: nextPosition
            )
            XCTAssertFalse(harness.frames.isRunning, "settle iteration \(iteration)")
            XCTAssertFalse(
                harness.store.diagnostics.hasTerminalResidue,
                "settle iteration \(iteration)"
            )

            position = nextPosition
            harness.store.beginGesture()
            harness.store.updateGesture(
                logicalTranslation: position == .closed ? 50 : -50
            )
            harness.store.forceTerminal(
                invalidations[(iteration + 1) % invalidations.count],
                position: position
            )
            XCTAssertFalse(harness.frames.isRunning, "drag iteration \(iteration)")
            XCTAssertFalse(
                harness.store.diagnostics.hasTerminalResidue,
                "drag iteration \(iteration)"
            )
        }
    }

    func testRootSurfaceAndUIKitHostEndsSynchronouslyReleaseTheDriver() {
        let harness = Harness(
            projection: .fullScreenNavigation,
            bindsToRootSurfaceHost: true
        )
        let firstRoot = GaryxRootSurfaceOccurrenceID(rawValue: 1)
        let firstHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: firstRoot,
            rawValue: "first-host"
        )
        harness.store.applyRootSurfaceOccurrenceTransition(
            .navigationShellBegan(firstRoot),
            position: .closed
        )
        harness.store.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: firstRoot
        )
        harness.store.attachHostOccurrence(firstHost, position: .closed)
        harness.store.setTarget(.open, animated: true)
        XCTAssertTrue(harness.frames.isRunning)

        harness.store.applyRootSurfaceOccurrenceTransition(
            .navigationShellEnded(firstRoot),
            position: .closed
        )

        XCTAssertFalse(harness.frames.isRunning)
        XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)
        XCTAssertTrue(harness.store.presentation.phase.allowsSurfaceHitTesting)

        // Model commands may update the canonical endpoint while no host is
        // mounted, but cannot create an ownerless animated settle.
        harness.store.setTarget(.open, animated: true)
        XCTAssertEqual(harness.store.presentation.phase, .idle)
        XCTAssertEqual(harness.store.presentation.target, .open)
        XCTAssertFalse(harness.frames.isRunning)

        let secondRoot = GaryxRootSurfaceOccurrenceID(rawValue: 2)
        let secondHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: secondRoot,
            rawValue: "second-host"
        )
        harness.store.applyRootSurfaceOccurrenceTransition(
            .navigationShellBegan(secondRoot),
            position: .open
        )
        harness.store.attachHostOccurrence(secondHost, position: .open)

        harness.store.beginGesture(in: firstHost)
        XCTAssertEqual(harness.store.presentation.phase, .idle)
        harness.store.setTarget(.closed, animated: true)
        XCTAssertTrue(harness.frames.isRunning)

        let replacementHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: secondRoot,
            rawValue: "replacement-host"
        )
        harness.store.attachHostOccurrence(replacementHost, position: .open)
        XCTAssertFalse(harness.frames.isRunning)
        XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)

        harness.store.detachHostOccurrence(secondHost, position: .open)
        harness.store.beginGesture(in: secondHost)
        XCTAssertEqual(harness.store.presentation.phase, .idle)
        harness.store.beginGesture(in: replacementHost)
        XCTAssertEqual(harness.store.presentation.phase, .dragging)

        harness.store.detachHostOccurrence(replacementHost, position: .open)
        XCTAssertFalse(harness.frames.isRunning)
        XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)
        XCTAssertTrue(harness.store.presentation.phase.allowsSurfaceHitTesting)
    }

    func testDeferredHostDetachStopsDriverNowAndPublishesTerminalProjectionLater() {
        let scheduler = GaryxManualObservableSettlementScheduler()
        let harness = Harness(
            projection: .fullScreenNavigation,
            bindsToRootSurfaceHost: true,
            observableSettlementScheduler: scheduler
        )
        let root = GaryxRootSurfaceOccurrenceID(rawValue: 10)
        let host = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: root,
            rawValue: "deferred-host"
        )
        harness.store.applyRootSurfaceOccurrenceTransition(
            .navigationShellBegan(root),
            position: .closed
        )
        harness.store.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: root
        )
        harness.store.attachHostOccurrence(host, position: .closed)
        harness.store.setTarget(.open, animated: true)
        XCTAssertTrue(harness.frames.isRunning)

        var publicationCount = 0
        let publication = harness.store.objectWillChange.sink {
            publicationCount += 1
        }
        harness.store.detachHostOccurrence(
            host,
            position: .closed,
            observableSettlement: .afterViewGraphUpdate
        )

        XCTAssertFalse(harness.frames.isRunning, "driver ownership settles synchronously")
        XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)
        XCTAssertEqual(harness.store.diagnostics.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .closed
        ))
        XCTAssertEqual(
            harness.store.presentation.phase,
            .settling(.open),
            "the observable projection is deliberately unchanged in the lifecycle callback"
        )
        XCTAssertEqual(publicationCount, 0)
        XCTAssertEqual(scheduler.pendingCount, 1)

        scheduler.runNext()

        XCTAssertEqual(harness.store.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .closed
        ))
        XCTAssertEqual(publicationCount, 1)
        XCTAssertEqual(scheduler.pendingCount, 0)
        withExtendedLifetime(publication) {}
    }

    func testDeferredHostDetachCannotOverwriteNewerRemountProjection() {
        let scheduler = GaryxManualObservableSettlementScheduler()
        let harness = Harness(
            projection: .fullScreenNavigation,
            bindsToRootSurfaceHost: true,
            observableSettlementScheduler: scheduler
        )
        let root = GaryxRootSurfaceOccurrenceID(rawValue: 11)
        let firstHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: root,
            rawValue: "first-host"
        )
        let replacementHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: root,
            rawValue: "replacement-host"
        )
        harness.store.applyRootSurfaceOccurrenceTransition(
            .navigationShellBegan(root),
            position: .closed
        )
        harness.store.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: root
        )
        harness.store.attachHostOccurrence(firstHost, position: .closed)
        harness.store.setTarget(.open, animated: true)
        harness.store.detachHostOccurrence(
            firstHost,
            position: .closed,
            observableSettlement: .afterViewGraphUpdate
        )
        XCTAssertEqual(scheduler.pendingCount, 1)

        harness.store.attachHostOccurrence(replacementHost, position: .closed)
        harness.store.setTarget(.open, animated: false)
        XCTAssertEqual(harness.store.presentation, .init(
            reveal: 330,
            phase: .idle,
            target: .open
        ))

        scheduler.runNext()

        XCTAssertEqual(harness.store.presentation, .init(
            reveal: 330,
            phase: .idle,
            target: .open
        ), "an older deferred terminal projection cannot win after remount")
        XCTAssertFalse(harness.store.diagnostics.hasTerminalResidue)
    }

    func testModelConnectionReplacementTerminatesBeforePublishingTheNewRoot() throws {
        let suiteName = "GaryxRootInteractionOwnershipTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let model = GaryxMobileModel(defaults: defaults)
        model.gatewayURL = "http://127.0.0.1:4000"
        model.connectionState = .ready(version: "before-reconnect")
        let firstRoot = try rootOccurrenceID(of: model.homeObservationStore)
        let firstHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: firstRoot,
            rawValue: "first-host"
        )
        model.drawerRevealInteraction.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: firstRoot
        )
        model.attachGlobalRevealHostOccurrence(firstHost)
        model.drawerRevealInteraction.beginGesture(in: firstHost)
        model.drawerRevealInteraction.updateGesture(
            logicalTranslation: 120,
            in: firstHost
        )
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)

        model.connectionState = .checking

        XCTAssertEqual(model.homeObservationStore.rootSurface, .gatewaySetup)
        XCTAssertFalse(model.drawerRevealInteraction.diagnostics.hasTerminalResidue)
        XCTAssertTrue(
            model.drawerRevealInteraction.presentation.phase.allowsSurfaceHitTesting
        )

        model.connectionState = .ready(version: "after-reconnect")
        let secondRoot = try rootOccurrenceID(of: model.homeObservationStore)
        let secondHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: secondRoot,
            rawValue: "second-host"
        )
        model.attachGlobalRevealHostOccurrence(secondHost)

        model.drawerRevealInteraction.beginGesture(in: firstHost)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .idle)
        model.drawerRevealInteraction.beginGesture(in: secondHost)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)
        model.detachGlobalRevealHostOccurrence(secondHost)
        XCTAssertFalse(model.drawerRevealInteraction.diagnostics.hasTerminalResidue)
    }

    private func rootOccurrenceID(
        of store: GaryxHomeObservationStore
    ) throws -> GaryxRootSurfaceOccurrenceID {
        guard case .navigationShell(let occurrenceID) = store.rootSurface else {
            throw RootOccurrenceError.navigationShellNotVisible
        }
        return occurrenceID
    }

    private enum RootOccurrenceError: Error {
        case navigationShellNotVisible
    }

    @MainActor
    private final class Harness {
        let clock = ManualTimeSource()
        let frames = ManualFrameSource()
        let store: GaryxHorizontalRevealInteractionStore

        init(
            projection: GaryxMotionPhysics.ProjectionPolicy,
            bindsToRootSurfaceHost: Bool = false,
            observableSettlementScheduler: (
                any GaryxObservableSettlementScheduling
            )? = nil
        ) {
            store = GaryxHorizontalRevealInteractionStore(
                projection: projection,
                bindsToRootSurfaceHost: bindsToRootSurfaceHost,
                settleDriver: GaryxGestureSettleDriver(
                    timeSource: clock,
                    frameSource: frames
                ),
                observableSettlementScheduler: observableSettlementScheduler
            )
        }

        func advance(by interval: TimeInterval) {
            clock.now += interval
            frames.fire()
        }
    }

    private final class ManualTimeSource: GaryxGestureSettleTimeSource {
        var now: TimeInterval = 10
    }

    private final class ManualFrameSource: GaryxGestureSettleFrameSource {
        var onFrame: (() -> Void)?
        private(set) var isRunning = false

        func start() { isRunning = true }
        func invalidate() { isRunning = false }
        func fire() {
            guard isRunning else { return }
            onFrame?()
        }
    }
}

@MainActor
final class GaryxManualObservableSettlementScheduler:
    GaryxObservableSettlementScheduling
{
    private var actions: [@MainActor () -> Void] = []

    var pendingCount: Int { actions.count }

    func schedule(_ action: @escaping @MainActor () -> Void) {
        actions.append(action)
    }

    func runNext() {
        precondition(!actions.isEmpty, "no deferred observable settlement is pending")
        actions.removeFirst()()
    }
}
