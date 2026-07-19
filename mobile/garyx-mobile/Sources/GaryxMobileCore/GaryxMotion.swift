import Foundation

/// The single semantic motion-token catalog for Garyx mobile.
///
/// Rule: state-driven motion is critically damped (damping ratio `1`) and
/// must not overshoot. An underdamped spring is legal only for a gesture
/// release that hands real momentum into the settle trajectory. Menus,
/// toasts, presses, programmatic navigation, and snap-backs never bounce.
public enum GaryxMotion {
    public enum Token: String, CaseIterable, Sendable {
        case morphOpen
        case morphClose
        case settle
        case rowSwipe
        case snapBack
        case momentumSnapBack
        case cancelSnapBack
        case composerPayload
        case composerPanel
        case composerDrilldown
        case drilldown
        case runtimeDrilldownExit
        case runtimeDrilldownEnter
        case panelResize
        case toast
        case press
        case floatingPress
        case subtlePress
        case pressHighlight
        case messageMenu
        case threadMenuFocus
        case threadMenu
        case authenticationStep
        case disclosure
        case formDisclosure
        case imageSaveFeedback
        case imageZoom
        case imageZoomReset
        case favoriteToggle
        case avatarChange
        case avatarPreview
        case avatarLoading
        case rowRemoval
        case threadListMutation
        case scrollLatest
        case tailThinking
        case scrollToTail
        case turnDisclosure
        case turnAutoDisclosure
        case streamingResize
        case transcriptAppear
        case loadingShimmer
        case thinkingShimmer
        case inkSpinner
        case runningTyping
        case runningOrbit
    }

    public enum Kinetics: String, Sendable {
        case stateChange
        case gestureRelease
        case continuous
    }

    public enum Curve: Hashable, Sendable {
        case spring(response: TimeInterval, dampingRatio: Double)
        case easeIn(duration: TimeInterval)
        case easeOut(duration: TimeInterval)
        case easeInOut(duration: TimeInterval)
        case timingCurve(
            controlPoint1X: Double,
            controlPoint1Y: Double,
            controlPoint2X: Double,
            controlPoint2Y: Double,
            duration: TimeInterval
        )
        case snappy(duration: TimeInterval)
        case linear(duration: TimeInterval)

        public var duration: TimeInterval {
            switch self {
            case let .spring(response, _),
                 let .easeIn(response),
                 let .easeOut(response),
                 let .easeInOut(response),
                 let .snappy(response),
                 let .linear(response):
                response
            case let .timingCurve(_, _, _, _, duration):
                duration
            }
        }

        public var springCurve: GaryxMotionPhysics.SpringCurve? {
            guard case let .spring(response, dampingRatio) = self else { return nil }
            return .init(response: response, dampingRatio: dampingRatio)
        }
    }

    public struct Effect: Hashable, Sendable {
        public let scale: Double
        public let offsetX: Double
        public let offsetY: Double
        public let opacity: Double

        public init(
            scale: Double = 1,
            offsetX: Double = 0,
            offsetY: Double = 0,
            opacity: Double = 1
        ) {
            self.scale = scale
            self.offsetX = offsetX
            self.offsetY = offsetY
            self.opacity = opacity
        }

        public static let identity = Self()
    }

    public struct Specification: Hashable, Sendable {
        public let curve: Curve
        public let crossFadeCurve: Curve
        public let kinetics: Kinetics
        public let effect: Effect

        public init(
            curve: Curve,
            crossFadeCurve: Curve? = nil,
            kinetics: Kinetics = .stateChange,
            effect: Effect = .identity
        ) {
            if case let .spring(_, dampingRatio) = curve {
                precondition(
                    dampingRatio >= 1 || kinetics == .gestureRelease,
                    "Only momentum-carrying gesture releases may use an underdamped spring"
                )
            }
            self.curve = curve
            self.crossFadeCurve = crossFadeCurve ?? curve
            self.kinetics = kinetics
            self.effect = effect
        }
    }

    public struct Preferences: Equatable, Sendable {
        public let reduceMotion: Bool
        public let prefersCrossFadeTransitions: Bool

        public init(reduceMotion: Bool, prefersCrossFadeTransitions: Bool) {
            self.reduceMotion = reduceMotion
            self.prefersCrossFadeTransitions = prefersCrossFadeTransitions
        }

        public static let standard = Self(
            reduceMotion: false,
            prefersCrossFadeTransitions: false
        )
    }

    public struct Resolution: Hashable, Sendable {
        public let token: Token
        public let mode: GaryxAccessibilityTransitionMode
        public let curve: Curve?
        public let effect: Effect

        public var animates: Bool { curve != nil }
        public var allowsSpatialMotion: Bool { mode == .spatial }
    }

    /// Shared render cadence for manually phased TimelineView animations.
    public static let timelineMinimumInterval: TimeInterval = 1.0 / 30.0
    /// Runtime drilldown waits for its 100 ms exit plus a 50 ms visual rest.
    public static let runtimeDrilldownSwapDelay: TimeInterval = 0.15
    /// Defers the first refresh until the drawer's visible settle is complete.
    public static let drawerRefreshDeferral: TimeInterval = 0.30

    public static func resolve(
        _ token: Token,
        preferences: Preferences
    ) -> Resolution {
        let specification = specification(for: token)
        let mode = GaryxAccessibilityTransitionPolicy.mode(
            reduceMotion: preferences.reduceMotion,
            prefersCrossFadeTransitions: preferences.prefersCrossFadeTransitions
        )
        switch mode {
        case .spatial:
            return Resolution(
                token: token,
                mode: mode,
                curve: specification.curve,
                effect: specification.effect
            )
        case .crossFade:
            return Resolution(
                token: token,
                mode: mode,
                curve: specification.crossFadeCurve,
                effect: Effect(
                    opacity: specification.effect.opacity
                )
            )
        case .immediate:
            return Resolution(
                token: token,
                mode: mode,
                curve: nil,
                effect: Effect(
                    opacity: specification.effect.opacity
                )
            )
        }
    }

    public static func springCurve(for token: Token) -> GaryxMotionPhysics.SpringCurve {
        guard let curve = specification(for: token).curve.springCurve else {
            preconditionFailure("Motion token \(token.rawValue) is not a spring")
        }
        return curve
    }

    public static func specification(for token: Token) -> Specification {
        switch token {
        case .morphOpen:
            return Specification(
                curve: .spring(response: 0.42, dampingRatio: 1),
                crossFadeCurve: .easeOut(duration: 0.18)
            )
        case .morphClose:
            return Specification(
                curve: .spring(response: 0.32, dampingRatio: 1),
                crossFadeCurve: .easeOut(duration: 0.18)
            )
        case .settle:
            return Specification(
                curve: .spring(response: 0.22, dampingRatio: 0.88),
                kinetics: .gestureRelease
            )
        case .rowSwipe:
            return Specification(
                curve: .spring(response: 0.22, dampingRatio: 0.88),
                kinetics: .gestureRelease
            )
        case .snapBack:
            return Specification(curve: .spring(response: 0.22, dampingRatio: 1))
        case .momentumSnapBack:
            return Specification(
                curve: .spring(response: 0.34, dampingRatio: 0.82),
                kinetics: .gestureRelease
            )
        case .cancelSnapBack:
            return Specification(curve: .spring(response: 0.28, dampingRatio: 1))
        case .composerPayload:
            return Specification(curve: .spring(response: 0.24, dampingRatio: 1))
        case .composerPanel:
            return Specification(curve: .spring(response: 0.22, dampingRatio: 1))
        case .composerDrilldown:
            return Specification(curve: .snappy(duration: 0.22))
        case .drilldown:
            return Specification(curve: .easeOut(duration: 0.16))
        case .runtimeDrilldownExit:
            return Specification(
                curve: .easeIn(duration: 0.10),
                effect: Effect(offsetX: 12)
            )
        case .runtimeDrilldownEnter:
            return Specification(
                curve: .easeOut(duration: 0.18),
                crossFadeCurve: .easeOut(duration: 0.12),
                effect: Effect(offsetX: 12)
            )
        case .panelResize:
            return Specification(curve: .easeOut(duration: 0.18))
        case .toast:
            return Specification(curve: .easeOut(duration: 0.18))
        case .press:
            return Specification(
                curve: .easeOut(duration: 0.12),
                effect: Effect(scale: 0.96, opacity: 0.78)
            )
        case .floatingPress:
            return Specification(
                curve: .easeOut(duration: 0.12),
                effect: Effect(scale: 0.96, opacity: 0.85)
            )
        case .subtlePress:
            return Specification(
                curve: .easeOut(duration: 0.12),
                effect: Effect(scale: 0.985)
            )
        case .pressHighlight:
            return Specification(curve: .easeOut(duration: 0.10))
        case .messageMenu:
            return Specification(
                curve: .easeOut(duration: 0.14),
                effect: Effect(scale: 0.97)
            )
        case .threadMenuFocus:
            return Specification(
                curve: standardTimingCurve(duration: 0.16),
                effect: Effect(scale: 0.985)
            )
        case .threadMenu:
            return Specification(
                curve: standardTimingCurve(duration: 0.17),
                effect: Effect(scale: 0.965)
            )
        case .authenticationStep:
            return Specification(curve: .easeInOut(duration: 0.24))
        case .disclosure:
            return Specification(curve: .easeInOut(duration: 0.20))
        case .formDisclosure:
            return Specification(curve: .easeInOut(duration: 0.18))
        case .imageSaveFeedback:
            return Specification(curve: .easeOut(duration: 0.18))
        case .imageZoom:
            return Specification(curve: .spring(response: 0.28, dampingRatio: 1))
        case .imageZoomReset:
            return Specification(curve: .easeOut(duration: 0.18))
        case .favoriteToggle:
            return Specification(curve: .easeOut(duration: 0.18))
        case .avatarChange:
            return Specification(curve: .easeInOut(duration: 0.20))
        case .avatarPreview:
            return Specification(curve: .easeInOut(duration: 0.18))
        case .avatarLoading:
            return Specification(curve: .easeInOut(duration: 0.16))
        case .rowRemoval:
            return Specification(
                curve: standardTimingCurve(duration: 0.20),
                effect: Effect(scale: 0.98, offsetX: 18, opacity: 0)
            )
        case .threadListMutation:
            return Specification(curve: standardTimingCurve(duration: 0.28))
        case .scrollLatest:
            return Specification(
                curve: .easeOut(duration: 0.18),
                effect: Effect(scale: 0.88)
            )
        case .tailThinking:
            return Specification(curve: .easeOut(duration: 0.15))
        case .scrollToTail:
            return Specification(curve: .easeOut(duration: 0.20))
        case .turnDisclosure:
            return Specification(curve: .easeOut(duration: 0.18))
        case .turnAutoDisclosure:
            return Specification(curve: .easeOut(duration: 0.20))
        case .streamingResize:
            return Specification(curve: .easeOut(duration: 0.16))
        case .transcriptAppear:
            return Specification(
                curve: .easeOut(duration: 0.18),
                effect: Effect(offsetY: 10)
            )
        case .loadingShimmer:
            return Specification(
                curve: .linear(duration: 2.4),
                kinetics: .continuous
            )
        case .thinkingShimmer:
            return Specification(
                curve: .linear(duration: 2.6),
                kinetics: .continuous
            )
        case .inkSpinner:
            return Specification(
                curve: .linear(duration: 1.05),
                kinetics: .continuous
            )
        case .runningTyping:
            return Specification(
                curve: .easeInOut(duration: 0.34),
                kinetics: .continuous
            )
        case .runningOrbit:
            return Specification(
                curve: .linear(duration: 1.55),
                kinetics: .continuous
            )
        }
    }

    private static func standardTimingCurve(duration: TimeInterval) -> Curve {
        .timingCurve(
            controlPoint1X: 0.22,
            controlPoint1Y: 1,
            controlPoint2X: 0.36,
            controlPoint2Y: 1,
            duration: duration
        )
    }
}
