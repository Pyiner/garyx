import XCTest
@testable import GaryxMobileCore

final class GaryxMotionPhysicsTests: XCTestCase {
    func testSpringCurveIsIndependentOfFrameCadenceAtEqualElapsedTime() {
        let trajectory = GaryxMotionPhysics.SettleTrajectory(
            initialValue: 120,
            targetValue: 0,
            initialVelocity: 540,
            curve: GaryxMotionPhysics.SpringCurve(response: 0.34, dampingRatio: 0.82)
        )
        let checkpoints = [0.1, 0.2, 0.3]
        let sixtyHertz = stride(from: 0.0, through: 0.3, by: 1.0 / 60.0).map { $0 }
        let oneTwentyHertz = stride(from: 0.0, through: 0.3, by: 1.0 / 120.0).map { $0 }
        let irregular = [0.0, 0.011, 0.037, 0.061, 0.1, 0.143, 0.2, 0.238, 0.3]

        let reference = samples(at: checkpoints, afterFrames: sixtyHertz, trajectory: trajectory)
        XCTAssertEqual(
            samples(at: checkpoints, afterFrames: oneTwentyHertz, trajectory: trajectory),
            reference
        )
        XCTAssertEqual(
            samples(at: checkpoints, afterFrames: irregular, trajectory: trajectory),
            reference
        )
    }

    func testProjectionPolicyReferenceValuesAndUnits() {
        XCTAssertEqual(
            GaryxMotionPhysics.ProjectionPolicy.fullScreenNavigation
                .projectedDisplacement(velocityPointsPerSecond: 1_000),
            499,
            accuracy: 1e-9
        )
        XCTAssertEqual(
            GaryxMotionPhysics.ProjectionPolicy.shortTravelDismiss
                .projectedDisplacement(velocityPointsPerSecond: 1_000),
            200,
            accuracy: 1e-12
        )
    }

    func testRubberbandIsContinuousAtOriginAndSigned() {
        XCTAssertEqual(GaryxMotionPhysics.rubberband(overshoot: 0, dimension: 400), 0)
        let nearOrigin = GaryxMotionPhysics.rubberband(overshoot: 0.000_001, dimension: 400)
        XCTAssertEqual(nearOrigin / 0.000_001, 0.55, accuracy: 1e-8)
        XCTAssertEqual(
            GaryxMotionPhysics.rubberband(overshoot: -80, dimension: 400),
            -GaryxMotionPhysics.rubberband(overshoot: 80, dimension: 400),
            accuracy: 1e-12
        )
    }

    func testRubberbandIsMonotonicAndApproachesDimensionFromBelow() {
        let inputs: [CGFloat] = [0, 10, 50, 100, 500, 5_000, 5_000_000]
        let outputs = inputs.map {
            GaryxMotionPhysics.rubberband(overshoot: $0, dimension: 400)
        }
        for pair in zip(outputs, outputs.dropFirst()) {
            XCTAssertLessThan(pair.0, pair.1)
        }
        XCTAssertTrue(outputs.allSatisfy { $0 >= 0 && $0 < 400 })
        XCTAssertEqual(outputs.last!, 400, accuracy: 0.06)
    }

    private func samples(
        at checkpoints: [TimeInterval],
        afterFrames frames: [TimeInterval],
        trajectory: GaryxMotionPhysics.SettleTrajectory
    ) -> [GaryxMotionPhysics.MotionSample] {
        var samplesByCheckpoint: [Int: GaryxMotionPhysics.MotionSample] = [:]
        for frame in frames {
            for (index, checkpoint) in checkpoints.enumerated()
            where abs(frame - checkpoint) < 1e-9 {
                samplesByCheckpoint[index] = trajectory.sample(elapsedTime: frame)
            }
        }
        return checkpoints.indices.compactMap { samplesByCheckpoint[$0] }
    }
}
