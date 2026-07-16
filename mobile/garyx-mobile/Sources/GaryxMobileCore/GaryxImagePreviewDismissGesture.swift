import CoreGraphics

public enum GaryxImagePreviewDragPhase: Equatable, Sendable {
    case pending
    case downwardDismiss
    case rejected
}

public struct GaryxImagePreviewDismissMetrics: Equatable, Sendable {
    public let decisionDistance: CGFloat
    public let beginDominanceRatio: CGFloat
    public let dismissalDistance: CGFloat
    public let dismissalDominanceRatio: CGFloat

    public init(
        decisionDistance: CGFloat = 10,
        beginDominanceRatio: CGFloat = 1,
        dismissalDistance: CGFloat = 88,
        dismissalDominanceRatio: CGFloat = 1.25
    ) {
        self.decisionDistance = max(0, decisionDistance)
        self.beginDominanceRatio = max(1, beginDominanceRatio)
        self.dismissalDistance = max(0, dismissalDistance)
        self.dismissalDominanceRatio = max(1, dismissalDominanceRatio)
    }

    public static let `default` = GaryxImagePreviewDismissMetrics()
}

public enum GaryxImagePreviewDismissGesture {
    public static let metrics = GaryxImagePreviewDismissMetrics.default

    public static func classify(
        currentPhase: GaryxImagePreviewDragPhase = .pending,
        translation: CGSize,
        metrics: GaryxImagePreviewDismissMetrics = metrics
    ) -> GaryxImagePreviewDragPhase {
        guard currentPhase == .pending else { return currentPhase }
        let horizontal = abs(translation.width)
        let vertical = translation.height
        guard max(horizontal, abs(vertical)) >= metrics.decisionDistance else {
            return .pending
        }
        return isDownwardIntent(translation, metrics: metrics) ? .downwardDismiss : .rejected
    }

    public static func isDownwardIntent(
        _ translation: CGSize,
        metrics: GaryxImagePreviewDismissMetrics = metrics
    ) -> Bool {
        translation.height > 0
            && translation.height > abs(translation.width) * metrics.beginDominanceRatio
    }

    public static func visibleOffset(
        phase: GaryxImagePreviewDragPhase,
        translation: CGSize
    ) -> CGFloat {
        phase == .downwardDismiss ? max(0, translation.height) : 0
    }

    public static func shouldDismiss(
        phase: GaryxImagePreviewDragPhase,
        translation: CGSize,
        metrics: GaryxImagePreviewDismissMetrics = metrics
    ) -> Bool {
        guard phase == .downwardDismiss else { return false }
        return translation.height > metrics.dismissalDistance
            && translation.height > abs(translation.width) * metrics.dismissalDominanceRatio
    }
}
