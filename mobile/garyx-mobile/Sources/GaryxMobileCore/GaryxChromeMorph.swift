import CoreGraphics
import Foundation

public struct GaryxChromeMorphSurfaceMetrics: Equatable, Sendable {
    public let horizontalMargin: CGFloat
    public let maximumExpandedWidth: CGFloat
    public let collapsedCornerRadius: CGFloat
    public let expandedCornerRadius: CGFloat

    public init(
        horizontalMargin: CGFloat,
        maximumExpandedWidth: CGFloat,
        collapsedCornerRadius: CGFloat,
        expandedCornerRadius: CGFloat
    ) {
        self.horizontalMargin = max(0, horizontalMargin)
        self.maximumExpandedWidth = max(0, maximumExpandedWidth)
        self.collapsedCornerRadius = max(0, collapsedCornerRadius)
        self.expandedCornerRadius = max(0, expandedCornerRadius)
    }
}

public struct GaryxChromeMorphSurfaceLayout: Equatable, Sendable {
    public let expandedWidth: CGFloat
    public let expandedX: CGFloat
    public let outerWidth: CGFloat
    public let outerHeight: CGFloat?
    public let outerX: CGFloat
    public let outerY: CGFloat
    public let cornerRadius: CGFloat
}

/// Shared, headless geometry for in-place chrome morphs. The content is always
/// laid out at `expandedWidth`; `outerWidth/outerHeight` describe the clipping
/// window that changes between the compact anchor and expanded surface.
public enum GaryxChromeMorphSurfaceGeometry {
    public static func expandedWidth(
        containerWidth: CGFloat,
        metrics: GaryxChromeMorphSurfaceMetrics
    ) -> CGFloat {
        max(0, min(
            containerWidth - metrics.horizontalMargin * 2,
            metrics.maximumExpandedWidth
        ))
    }

    public static func expandedX(containerWidth: CGFloat, expandedWidth: CGFloat) -> CGFloat {
        (containerWidth - expandedWidth) / 2
    }

    public static func layout(
        isExpanded: Bool,
        anchorRect: CGRect,
        containerSize: CGSize,
        metrics: GaryxChromeMorphSurfaceMetrics
    ) -> GaryxChromeMorphSurfaceLayout {
        let width = expandedWidth(containerWidth: containerSize.width, metrics: metrics)
        let x = expandedX(containerWidth: containerSize.width, expandedWidth: width)
        return GaryxChromeMorphSurfaceLayout(
            expandedWidth: width,
            expandedX: x,
            outerWidth: isExpanded ? width : anchorRect.width,
            outerHeight: isExpanded ? nil : anchorRect.height,
            outerX: isExpanded ? x : anchorRect.minX,
            outerY: anchorRect.minY,
            cornerRadius: isExpanded
                ? metrics.expandedCornerRadius
                : metrics.collapsedCornerRadius
        )
    }
}

public enum GaryxChromeMorphPresentationState: Equatable, Sendable {
    case hidden
    case presentedCollapsed
    case expanded
    case collapsing

    public var isPresented: Bool { self != .hidden }
    public var isExpanded: Bool { self == .expanded }
}

public enum GaryxChromeMorphPresentationEvent: Equatable, Sendable {
    case requestPresent
    case expandTick
    case requestDismiss
    case dismissAnimationCompleted
}

public enum GaryxChromeMorphPresentationAnimation: Equatable, Sendable {
    case none
    case open
    case close
}

public enum GaryxChromeMorphPresentationSchedule: Equatable, Sendable {
    case none
    case expandOnNextTick
    case completeDismissAfterAnimation
}

public struct GaryxChromeMorphPresentationTransition: Equatable, Sendable {
    public let state: GaryxChromeMorphPresentationState
    public let animation: GaryxChromeMorphPresentationAnimation
    public let schedule: GaryxChromeMorphPresentationSchedule
}

/// Production state machine consumed directly by the Capsule morph wiring.
/// Thread runtime settings retain their existing booleans/timing unchanged.
public enum GaryxChromeMorphPresentationReducer {
    public static func reduce(
        state: GaryxChromeMorphPresentationState,
        event: GaryxChromeMorphPresentationEvent,
        transitionMode: GaryxAccessibilityTransitionMode
    ) -> GaryxChromeMorphPresentationTransition {
        switch (state, event) {
        case (.hidden, .requestPresent), (.collapsing, .requestPresent):
            if transitionMode == .immediate {
                return .init(state: .expanded, animation: .none, schedule: .none)
            }
            return .init(
                state: .presentedCollapsed,
                animation: .none,
                schedule: .expandOnNextTick
            )
        case (.presentedCollapsed, .expandTick):
            return .init(state: .expanded, animation: .open, schedule: .none)
        case (.expanded, .requestDismiss):
            if transitionMode == .immediate {
                return .init(state: .hidden, animation: .none, schedule: .none)
            }
            return .init(
                state: .collapsing,
                animation: .close,
                schedule: .completeDismissAfterAnimation
            )
        case (.presentedCollapsed, .requestDismiss):
            return .init(state: .hidden, animation: .none, schedule: .none)
        case (.collapsing, .dismissAnimationCompleted):
            return .init(state: .hidden, animation: .none, schedule: .none)
        default:
            return .init(state: state, animation: .none, schedule: .none)
        }
    }
}
