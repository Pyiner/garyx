import CoreGraphics
import Foundation
import SwiftUI

/// Shared analytic motion primitives for interactive Garyx surfaces.
///
/// Values and distances are expressed in points, velocities in points per
/// second, and all time values in seconds. Springs are sampled from absolute
/// elapsed time so their result never depends on the display refresh cadence.
public enum GaryxMotionPhysics {
    public struct MotionSample: Equatable, Sendable {
        public let value: CGFloat
        public let velocity: CGFloat

        public init(value: CGFloat, velocity: CGFloat) {
            self.value = value
            self.velocity = velocity
        }
    }

    /// Analytic wrapper around SwiftUI's system spring solver.
    public struct SpringCurve: Hashable, Sendable {
        public let response: TimeInterval
        public let dampingRatio: Double

        private let spring: Spring

        public init(response: TimeInterval, dampingRatio: Double) {
            precondition(response > 0, "Spring response must be positive")
            precondition(dampingRatio >= 0, "Spring damping ratio cannot be negative")
            self.response = response
            self.dampingRatio = dampingRatio
            spring = Spring(response: response, dampingRatio: dampingRatio)
        }

        public var settlingDuration: TimeInterval {
            spring.settlingDuration
        }

        public func value(
            target: CGFloat,
            initialVelocity: CGFloat = 0,
            time: TimeInterval
        ) -> CGFloat {
            spring.value(
                target: target,
                initialVelocity: initialVelocity,
                time: max(0, time)
            )
        }

        public func velocity(
            target: CGFloat,
            initialVelocity: CGFloat = 0,
            time: TimeInterval
        ) -> CGFloat {
            spring.velocity(
                target: target,
                initialVelocity: initialVelocity,
                time: max(0, time)
            )
        }
    }

    /// Momentum projection policies with their time units encoded in the case.
    public enum ProjectionPolicy: Equatable, Sendable {
        /// UIScrollView-style velocity decay per millisecond. Valid rates are
        /// greater than zero and less than one.
        case decelerationRate(Double)
        /// A fixed projection horizon measured in seconds.
        case horizon(TimeInterval)

        /// Long-travel, full-screen navigation projection.
        public static let fullScreenNavigation = Self.decelerationRate(0.998)
        /// Short-travel dismiss projection, calibrated against capsule drag.
        public static let shortTravelDismiss = Self.horizon(0.20)

        public func projectedDisplacement(
            velocityPointsPerSecond velocity: CGFloat
        ) -> CGFloat {
            switch self {
            case let .decelerationRate(ratePerMillisecond):
                guard ratePerMillisecond > 0, ratePerMillisecond < 1 else { return 0 }
                let millisecondsPerSecond = 1_000.0
                let durationMilliseconds = ratePerMillisecond / (1 - ratePerMillisecond)
                return velocity / millisecondsPerSecond * durationMilliseconds
            case let .horizon(seconds):
                return velocity * max(0, seconds)
            }
        }

        public func projectedValue(
            valuePoints value: CGFloat,
            velocityPointsPerSecond velocity: CGFloat
        ) -> CGFloat {
            value + projectedDisplacement(velocityPointsPerSecond: velocity)
        }

        public func projectedTranslation(
            _ translationPoints: CGSize,
            velocityPointsPerSecond velocity: CGSize
        ) -> CGSize {
            CGSize(
                width: projectedValue(
                    valuePoints: translationPoints.width,
                    velocityPointsPerSecond: velocity.width
                ),
                height: projectedValue(
                    valuePoints: translationPoints.height,
                    velocityPointsPerSecond: velocity.height
                )
            )
        }
    }

    /// One scalar settle evaluated by the system spring's analytic solution.
    public struct SettleTrajectory: Hashable, Sendable {
        public let initialValue: CGFloat
        public let targetValue: CGFloat
        public let initialVelocity: CGFloat
        public let curve: SpringCurve

        public init(
            initialValue: CGFloat,
            targetValue: CGFloat,
            initialVelocity: CGFloat,
            curve: SpringCurve
        ) {
            self.initialValue = initialValue
            self.targetValue = targetValue
            self.initialVelocity = initialVelocity
            self.curve = curve
        }

        public var settlingDuration: TimeInterval {
            curve.settlingDuration
        }

        public func sample(elapsedTime: TimeInterval) -> MotionSample {
            let elapsedTime = max(0, elapsedTime)
            guard elapsedTime < settlingDuration else {
                return MotionSample(value: targetValue, velocity: 0)
            }
            let relativeTarget = targetValue - initialValue
            return MotionSample(
                value: initialValue + curve.value(
                    target: relativeTarget,
                    initialVelocity: initialVelocity,
                    time: elapsedTime
                ),
                velocity: curve.velocity(
                    target: relativeTarget,
                    initialVelocity: initialVelocity,
                    time: elapsedTime
                )
            )
        }
    }

    /// Applies Apple's rubber-band curve to signed overshoot in points.
    /// Finite input stays strictly inside `dimension` and asymptotically
    /// approaches that bound. The default constant matches UIScrollView feel.
    public static func rubberband(
        overshoot: CGFloat,
        dimension: CGFloat,
        constant: CGFloat = 0.55
    ) -> CGFloat {
        guard overshoot != 0, dimension > 0, constant > 0 else { return 0 }
        let magnitude = abs(overshoot)
        let damped = dimension * constant * magnitude / (dimension + constant * magnitude)
        return overshoot < 0 ? -damped : damped
    }
}
