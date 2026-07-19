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

    @MainActor
    private final class Harness {
        let clock = ManualTimeSource()
        let frames = ManualFrameSource()
        let store: GaryxHorizontalRevealInteractionStore

        init(projection: GaryxMotionPhysics.ProjectionPolicy) {
            store = GaryxHorizontalRevealInteractionStore(
                projection: projection,
                settleDriver: GaryxGestureSettleDriver(
                    timeSource: clock,
                    frameSource: frames
                )
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
