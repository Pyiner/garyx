import CoreGraphics

/// One axis-locked state machine shared by SwiftUI presentation and the UIKit
/// ownership adapter. `pending` is held until the 14pt decision boundary;
/// afterward the drag is permanently vertical, horizontal, or ignored.
public enum GaryxCapsuleDragPhase: Equatable, Sendable {
    case pending
    case verticalDismiss
    case horizontalDismiss
    case ignored

    public var ownsGesture: Bool {
        self == .verticalDismiss || self == .horizontalDismiss
    }
}

public struct GaryxCapsuleDragDismissMetrics: Equatable, Sendable {
    public let leadingEdgeWidth: CGFloat
    public let decisionDistance: CGFloat
    public let horizontalDominanceRatio: CGFloat
    public let horizontalThresholdRatio: CGFloat
    public let horizontalThresholdMaximum: CGFloat
    public let verticalThreshold: CGFloat
    public let velocityProjectionSeconds: CGFloat
    public let fullPullDistance: CGFloat

    public init(
        leadingEdgeWidth: CGFloat = 24,
        decisionDistance: CGFloat = 14,
        horizontalDominanceRatio: CGFloat = 1.5,
        horizontalThresholdRatio: CGFloat = 1.0 / 3.0,
        horizontalThresholdMaximum: CGFloat = 260,
        verticalThreshold: CGFloat = 120,
        velocityProjectionSeconds: CGFloat = 0.20,
        fullPullDistance: CGFloat = 240
    ) {
        self.leadingEdgeWidth = max(0, leadingEdgeWidth)
        self.decisionDistance = max(0, decisionDistance)
        self.horizontalDominanceRatio = max(1, horizontalDominanceRatio)
        self.horizontalThresholdRatio = max(0, horizontalThresholdRatio)
        self.horizontalThresholdMaximum = max(0, horizontalThresholdMaximum)
        self.verticalThreshold = max(0, verticalThreshold)
        self.velocityProjectionSeconds = max(0, velocityProjectionSeconds)
        self.fullPullDistance = max(1, fullPullDistance)
    }

    public static let `default` = GaryxCapsuleDragDismissMetrics()
}

public struct GaryxCapsuleDragDismissState: Equatable, Sendable {
    public var phase: GaryxCapsuleDragPhase
    public var translation: CGSize

    public init(phase: GaryxCapsuleDragPhase = .pending, translation: CGSize = .zero) {
        self.phase = phase
        self.translation = translation
    }
}

public enum GaryxCapsuleDragDismissEvent: Equatable, Sendable {
    case changed(
        startX: CGFloat,
        translation: CGSize,
        webAtTop: Bool,
        panelPresented: Bool
    )
    case released(velocity: CGSize, containerWidth: CGFloat)
    case cancelled
}

public enum GaryxCapsuleDragDismissEffect: Equatable, Sendable {
    case none
    case dismiss
    case snapBack
}

public enum GaryxCapsuleDragDismiss {
    public static let metrics = GaryxCapsuleDragDismissMetrics.default

    public static func classify(
        currentPhase: GaryxCapsuleDragPhase = .pending,
        startX: CGFloat,
        translation: CGSize,
        webAtTop: Bool,
        panelPresented: Bool,
        metrics: GaryxCapsuleDragDismissMetrics = metrics
    ) -> GaryxCapsuleDragPhase {
        guard currentPhase == .pending else { return currentPhase }
        guard !panelPresented else { return .ignored }

        let dx = translation.width
        let dy = translation.height
        let horizontalMagnitude = abs(dx)
        let verticalMagnitude = abs(dy)
        guard max(horizontalMagnitude, verticalMagnitude) >= metrics.decisionDistance else {
            return .pending
        }

        let rightwardHorizontal = dx > 0
            && horizontalMagnitude >= metrics.horizontalDominanceRatio * verticalMagnitude
        if rightwardHorizontal {
            return startX <= metrics.leadingEdgeWidth ? .horizontalDismiss : .ignored
        }
        if webAtTop, dy > 0 {
            return .verticalDismiss
        }
        return .ignored
    }

    public static func resolvedTranslation(
        phase: GaryxCapsuleDragPhase,
        translation: CGSize
    ) -> CGSize {
        switch phase {
        case .horizontalDismiss:
            return CGSize(width: max(0, translation.width), height: 0)
        case .verticalDismiss:
            return CGSize(width: 0, height: max(0, translation.height))
        case .pending, .ignored:
            return .zero
        }
    }

    public static func horizontalDismissThreshold(
        containerWidth: CGFloat,
        metrics: GaryxCapsuleDragDismissMetrics = metrics
    ) -> CGFloat {
        min(max(0, containerWidth) * metrics.horizontalThresholdRatio, metrics.horizontalThresholdMaximum)
    }

    public static func projectedEndTranslation(
        translation: CGSize,
        velocity: CGSize,
        metrics: GaryxCapsuleDragDismissMetrics = metrics
    ) -> CGSize {
        CGSize(
            width: translation.width + velocity.width * metrics.velocityProjectionSeconds,
            height: translation.height + velocity.height * metrics.velocityProjectionSeconds
        )
    }

    public static func shouldDismiss(
        phase: GaryxCapsuleDragPhase,
        translation: CGSize,
        velocity: CGSize,
        containerWidth: CGFloat,
        metrics: GaryxCapsuleDragDismissMetrics = metrics
    ) -> Bool {
        let resolved = resolvedTranslation(phase: phase, translation: translation)
        let projected = resolvedTranslation(
            phase: phase,
            translation: projectedEndTranslation(
                translation: translation,
                velocity: velocity,
                metrics: metrics
            )
        )
        switch phase {
        case .horizontalDismiss:
            let threshold = horizontalDismissThreshold(containerWidth: containerWidth, metrics: metrics)
            guard threshold > 0 else { return false }
            return resolved.width >= threshold || projected.width >= threshold
        case .verticalDismiss:
            return resolved.height >= metrics.verticalThreshold
                || projected.height >= metrics.verticalThreshold
        case .pending, .ignored:
            return false
        }
    }

    public static func dragProgress(
        phase: GaryxCapsuleDragPhase,
        translation: CGSize,
        metrics: GaryxCapsuleDragDismissMetrics = metrics
    ) -> Double {
        let resolved = resolvedTranslation(phase: phase, translation: translation)
        let distance = phase == .horizontalDismiss ? resolved.width : resolved.height
        return min(1, max(0, Double(distance / metrics.fullPullDistance)))
    }

    @discardableResult
    public static func reduce(
        state: inout GaryxCapsuleDragDismissState,
        event: GaryxCapsuleDragDismissEvent,
        metrics: GaryxCapsuleDragDismissMetrics = metrics
    ) -> GaryxCapsuleDragDismissEffect {
        switch event {
        case let .changed(startX, translation, webAtTop, panelPresented):
            state.phase = classify(
                currentPhase: state.phase,
                startX: startX,
                translation: translation,
                webAtTop: webAtTop,
                panelPresented: panelPresented,
                metrics: metrics
            )
            state.translation = resolvedTranslation(phase: state.phase, translation: translation)
            return .none
        case let .released(velocity, containerWidth):
            let owned = state.phase.ownsGesture
            let dismiss = shouldDismiss(
                phase: state.phase,
                translation: state.translation,
                velocity: velocity,
                containerWidth: containerWidth,
                metrics: metrics
            )
            state = GaryxCapsuleDragDismissState()
            if dismiss { return .dismiss }
            return owned ? .snapBack : .none
        case .cancelled:
            state = GaryxCapsuleDragDismissState()
            return .none
        }
    }
}
