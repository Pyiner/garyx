import CoreGraphics
import Foundation

// MARK: - Frozen transition policy

public enum GaryxRouteVisualPolicy: String, CaseIterable, Codable, Sendable {
    case spatial
    case crossFade
    case immediate
}

public struct GaryxRouteVisualPreferences: Equatable, Sendable {
    public let reduceMotion: Bool
    public let prefersCrossFadeTransitions: Bool

    public init(reduceMotion: Bool, prefersCrossFadeTransitions: Bool) {
        self.reduceMotion = reduceMotion
        self.prefersCrossFadeTransitions = prefersCrossFadeTransitions
    }

    public var resolvedPolicy: GaryxRouteVisualPolicy {
        guard GaryxAccessibilityTransitionPolicy.usesCrossFade(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        ) else {
            return .spatial
        }
        return GaryxAccessibilityTransitionPolicy.animatesTransition(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        ) ? .crossFade : .immediate
    }
}

public enum GaryxRouteTransitionKind: String, CaseIterable, Codable, Sendable {
    case push
    case pop
    case replace
}

public enum GaryxRouteLayoutDirection: String, CaseIterable, Codable, Sendable {
    case leftToRight
    case rightToLeft

    public var physicalForwardSign: CGFloat {
        self == .leftToRight ? 1 : -1
    }
}

public enum GaryxRouteTransitionCalibration {
    /// The explicit ownership zone used for arbitration. UIKit still owns its
    /// private recognition hysteresis; the measured iOS 26 threshold is kept
    /// here as an acceptance reference rather than reimplemented.
    public static let edgeZoneWidth: CGFloat = 20
    public static let measuredRecognitionThreshold: CGFloat = 12.7
    public static let incomingParallaxFraction: CGFloat = 0.30
    public static let maximumScrimAlpha: CGFloat = 0.18
    public static let commitProgress: CGFloat = 0.5

    /// 404 ms on the iOS 26 SwiftUI spring solver, inside the measured
    /// 300-440 ms system-pop settle window.
    public static var settleCurve: GaryxMotionPhysics.SpringCurve {
        .init(response: 0.22, dampingRatio: 0.88)
    }
}

public struct GaryxRouteTransitionVisualState: Equatable, Sendable {
    public let sourceTranslationX: CGFloat
    public let destinationTranslationX: CGFloat
    public let sourceAlpha: CGFloat
    public let destinationAlpha: CGFloat
    public let scrimAlpha: CGFloat
    public let movingShadowOpacity: Float
    public let movingShadowOffsetX: CGFloat

    public init(
        sourceTranslationX: CGFloat,
        destinationTranslationX: CGFloat,
        sourceAlpha: CGFloat,
        destinationAlpha: CGFloat,
        scrimAlpha: CGFloat,
        movingShadowOpacity: Float,
        movingShadowOffsetX: CGFloat
    ) {
        self.sourceTranslationX = sourceTranslationX
        self.destinationTranslationX = destinationTranslationX
        self.sourceAlpha = sourceAlpha
        self.destinationAlpha = destinationAlpha
        self.scrimAlpha = scrimAlpha
        self.movingShadowOpacity = movingShadowOpacity
        self.movingShadowOffsetX = movingShadowOffsetX
    }

    public static let identity = Self(
        sourceTranslationX: 0,
        destinationTranslationX: 0,
        sourceAlpha: 1,
        destinationAlpha: 1,
        scrimAlpha: 0,
        movingShadowOpacity: 0,
        movingShadowOffsetX: 0
    )
}

public enum GaryxRouteTransitionGeometry {
    public static func visualState(
        kind: GaryxRouteTransitionKind,
        policy: GaryxRouteVisualPolicy,
        progress rawProgress: CGFloat,
        viewportWidth: CGFloat,
        layoutDirection: GaryxRouteLayoutDirection
    ) -> GaryxRouteTransitionVisualState {
        let progress = min(max(rawProgress, 0), 1)
        let width = max(0, viewportWidth)
        let sign = layoutDirection.physicalForwardSign

        switch policy {
        case .spatial:
            switch kind {
            case .pop:
                return GaryxRouteTransitionVisualState(
                    sourceTranslationX: sign * width * progress,
                    destinationTranslationX: -sign * width
                        * GaryxRouteTransitionCalibration.incomingParallaxFraction
                        * (1 - progress),
                    sourceAlpha: 1,
                    destinationAlpha: 1,
                    scrimAlpha: GaryxRouteTransitionCalibration.maximumScrimAlpha * (1 - progress),
                    movingShadowOpacity: progress > 0 && progress < 1 ? 0.24 : 0,
                    movingShadowOffsetX: -sign * 5
                )
            case .push:
                return GaryxRouteTransitionVisualState(
                    sourceTranslationX: -sign * width
                        * GaryxRouteTransitionCalibration.incomingParallaxFraction * progress,
                    destinationTranslationX: sign * width * (1 - progress),
                    sourceAlpha: 1,
                    destinationAlpha: 1,
                    scrimAlpha: GaryxRouteTransitionCalibration.maximumScrimAlpha * progress,
                    movingShadowOpacity: progress > 0 && progress < 1 ? 0.24 : 0,
                    movingShadowOffsetX: -sign * 5
                )
            case .replace:
                return crossFade(progress: progress)
            }
        case .crossFade:
            return crossFade(progress: progress)
        case .immediate:
            let reachedDestination = progress >= 1
            return GaryxRouteTransitionVisualState(
                sourceTranslationX: 0,
                destinationTranslationX: 0,
                sourceAlpha: reachedDestination ? 0 : 1,
                destinationAlpha: reachedDestination ? 1 : 0,
                scrimAlpha: 0,
                movingShadowOpacity: 0,
                movingShadowOffsetX: 0
            )
        }
    }

    private static func crossFade(progress: CGFloat) -> GaryxRouteTransitionVisualState {
        GaryxRouteTransitionVisualState(
            sourceTranslationX: 0,
            destinationTranslationX: 0,
            sourceAlpha: 1 - progress,
            destinationAlpha: progress,
            scrimAlpha: 0,
            movingShadowOpacity: 0,
            movingShadowOffsetX: 0
        )
    }
}

// MARK: - Gesture arbitration and projection

public enum GaryxRouteLogicalEdge: String, CaseIterable, Codable, Sendable {
    case leading
    case trailing
}

public struct GaryxRouteEdgeTouchSnapshot: Equatable, Sendable {
    public let physicalX: CGFloat
    public let viewportWidth: CGFloat
    public let logicalEdge: GaryxRouteLogicalEdge
    public let layoutDirection: GaryxRouteLayoutDirection

    public init(
        physicalX: CGFloat,
        viewportWidth: CGFloat,
        logicalEdge: GaryxRouteLogicalEdge,
        layoutDirection: GaryxRouteLayoutDirection
    ) {
        self.physicalX = physicalX
        self.viewportWidth = viewportWidth
        self.logicalEdge = logicalEdge
        self.layoutDirection = layoutDirection
    }

    public func isInsideEdgeZone(
        width zoneWidth: CGFloat = GaryxRouteTransitionCalibration.edgeZoneWidth
    ) -> Bool {
        guard viewportWidth > 0, physicalX >= 0, physicalX <= viewportWidth else { return false }
        let physicalLeadingIsLeft = layoutDirection == .leftToRight
        let targetIsLeft = logicalEdge == .leading
            ? physicalLeadingIsLeft
            : !physicalLeadingIsLeft
        return targetIsLeft
            ? physicalX <= zoneWidth
            : physicalX >= viewportWidth - zoneWidth
    }
}

public enum GaryxRouteGestureAxis: Equatable, Sendable {
    case horizontal
    case vertical
    case undecided
}

public enum GaryxRouteGestureCompetitionSurface: String, CaseIterable, Codable, Sendable {
    case horizontalScroll
    case composerKeyboardDismiss
    case rowSwipe
    case verticalScroll
    case modalPresentation
    case taskTree
}

public enum GaryxRouteGestureWinner: Equatable, Sendable {
    case navigation
    case descendant
    case modal
    case taskTree
    case undecided
}

public enum GaryxRouteGestureDirection: Equatable, Sendable {
    case positive
    case negative
    case either

    public func accepts(_ logicalValue: CGFloat) -> Bool {
        switch self {
        case .positive:
            logicalValue > 0
        case .negative:
            logicalValue < 0
        case .either:
            logicalValue != 0
        }
    }
}

public enum GaryxRouteEdgeGestureArbitrator {
    public static func axis(
        translation: CGSize,
        velocity: CGSize,
        dominanceRatio: CGFloat = 1.05
    ) -> GaryxRouteGestureAxis {
        let vector = velocity == .zero ? translation : velocity
        let horizontal = abs(vector.width)
        let vertical = abs(vector.height)
        guard max(horizontal, vertical) > 0 else { return .undecided }
        if horizontal >= vertical * dominanceRatio { return .horizontal }
        if vertical >= horizontal * dominanceRatio { return .vertical }
        return .undecided
    }

    public static func logicalTranslation(
        physicalTranslationX: CGFloat,
        edge: GaryxRouteLogicalEdge,
        layoutDirection: GaryxRouteLayoutDirection
    ) -> CGFloat {
        let leadingSign = layoutDirection.physicalForwardSign
        let sign = edge == .leading ? leadingSign : -leadingSign
        return physicalTranslationX * sign
    }

    public static func shouldBegin(
        touch: GaryxRouteEdgeTouchSnapshot,
        translation: CGSize,
        velocity: CGSize,
        modalBarrierActive: Bool,
        actionEligible: Bool,
        requiresEdgeZone: Bool = true,
        direction: GaryxRouteGestureDirection = .positive
    ) -> Bool {
        guard actionEligible, !modalBarrierActive else { return false }
        if requiresEdgeZone, !touch.isInsideEdgeZone() { return false }
        guard axis(translation: translation, velocity: velocity) == .horizontal else { return false }
        let intent = velocity == .zero ? translation.width : velocity.width
        let logicalIntent = logicalTranslation(
            physicalTranslationX: intent,
            edge: touch.logicalEdge,
            layoutDirection: touch.layoutDirection
        )
        return direction.accepts(logicalIntent)
    }

    public static func winner(
        surface: GaryxRouteGestureCompetitionSurface,
        touchStartedInEdgeZone: Bool,
        actionEligible: Bool,
        requiresEdgeZone: Bool = true
    ) -> GaryxRouteGestureWinner {
        switch surface {
        case .modalPresentation:
            return .modal
        case .taskTree:
            return actionEligible && (!requiresEdgeZone || touchStartedInEdgeZone)
                ? .taskTree
                : .descendant
        case .verticalScroll:
            return .descendant
        case .horizontalScroll, .composerKeyboardDismiss, .rowSwipe:
            return touchStartedInEdgeZone && actionEligible ? .navigation : .descendant
        }
    }

    public static func progress(logicalTranslation: CGFloat, viewportWidth: CGFloat) -> CGFloat {
        guard viewportWidth > 0 else { return 0 }
        if logicalTranslation < 0 {
            return GaryxMotionPhysics.rubberband(
                overshoot: logicalTranslation,
                dimension: viewportWidth
            ) / viewportWidth
        }
        if logicalTranslation > viewportWidth {
            return 1 + GaryxMotionPhysics.rubberband(
                overshoot: logicalTranslation - viewportWidth,
                dimension: viewportWidth
            ) / viewportWidth
        }
        return logicalTranslation / viewportWidth
    }

    public static func shouldCommit(
        logicalTranslation: CGFloat,
        logicalVelocity: CGFloat,
        viewportWidth: CGFloat
    ) -> Bool {
        guard viewportWidth > 0 else { return false }
        let projectedLanding = GaryxMotionPhysics.ProjectionPolicy.fullScreenNavigation
            .projectedValue(
                valuePoints: logicalTranslation,
                velocityPointsPerSecond: logicalVelocity
            )
        return projectedLanding > viewportWidth * GaryxRouteTransitionCalibration.commitProgress
    }
}

// MARK: - One frozen four-phase transition

public struct GaryxRouteTransitionSession: Equatable, Sendable {
    public let kind: GaryxRouteTransitionKind
    public let source: GaryxRoutePresentationNode
    public let destination: GaryxRoutePresentationNode
    public let visualPolicy: GaryxRouteVisualPolicy
    public private(set) var coordinator: GaryxPresentationTransactionCoordinator
    public private(set) var progress: CGFloat
    public private(set) var settleTarget: CGFloat?

    public init?(
        kind: GaryxRouteTransitionKind,
        source: GaryxRoutePresentationNode,
        destination: GaryxRoutePresentationNode,
        preferences: GaryxRouteVisualPreferences,
        initialProgress: CGFloat = 0
    ) {
        var coordinator = GaryxPresentationTransactionCoordinator()
        guard coordinator.begin() else { return nil }
        self.kind = kind
        self.source = source
        self.destination = destination
        visualPolicy = preferences.resolvedPolicy
        self.coordinator = coordinator
        progress = initialProgress
        settleTarget = nil
    }

    public mutating func update(progress: CGFloat) -> Bool {
        guard coordinator.phase == .preCommit else { return false }
        self.progress = progress
        return true
    }

    @discardableResult
    public mutating func release(
        logicalTranslation: CGFloat,
        logicalVelocity: CGFloat,
        viewportWidth: CGFloat
    ) -> GaryxPresentationTerminalOutcome? {
        guard coordinator.phase == .preCommit else { return nil }
        let commits = GaryxRouteEdgeGestureArbitrator.shouldCommit(
            logicalTranslation: logicalTranslation,
            logicalVelocity: logicalVelocity,
            viewportWidth: viewportWidth
        )
        guard coordinator.release(commit: commits) else { return nil }
        settleTarget = commits ? 1 : 0
        return commits ? .committed : .cancelled
    }

    public mutating func regrabCancelSettle(progress: CGFloat) -> Bool {
        guard coordinator.regrabCancelSettle() else { return false }
        self.progress = progress
        settleTarget = nil
        return true
    }

    public mutating func updateSettle(progress: CGFloat) -> Bool {
        guard coordinator.phase == .cancelSettle || coordinator.phase == .commitSettle else {
            return false
        }
        self.progress = progress
        return true
    }

    @discardableResult
    public mutating func handle(
        _ event: GaryxPresentationCoordinatorEvent
    ) -> GaryxPresentationEventEffect {
        let effect = coordinator.handle(event)
        switch effect {
        case .transitioned(.cancelSettle):
            settleTarget = 0
        case .reachedTerminal(let terminal):
            settleTarget = terminal.outcome == .committed ? 1 : 0
        case .transitioned, .rederiveGeometry, .ignored:
            break
        }
        return effect
    }

    public mutating func finish(
        visibility: GaryxPresentationVisibility
    ) -> GaryxPresentationTerminalState? {
        guard coordinator.finish(visibility: visibility) else { return nil }
        progress = settleTarget ?? progress
        return coordinator.terminalState
    }

    public func visualState(
        viewportWidth: CGFloat,
        layoutDirection: GaryxRouteLayoutDirection
    ) -> GaryxRouteTransitionVisualState {
        GaryxRouteTransitionGeometry.visualState(
            kind: kind,
            policy: visualPolicy,
            progress: progress,
            viewportWidth: viewportWidth,
            layoutDirection: layoutDirection
        )
    }
}

// MARK: - Bounded route-state store

public enum GaryxRoutePresentationIdentity: Hashable, Codable, Sendable {
    case home
    case entry(GaryxRouteInstanceID)

    public init(_ node: GaryxRoutePresentationNode) {
        switch node {
        case .home:
            self = .home
        case .entry(let entry):
            self = .entry(entry.id)
        }
    }
}

/// The only fields the renderer may preserve after evicting a route host.
public enum GaryxRouteStateField: String, CaseIterable, Codable, Sendable {
    case scrollAnchor
    case selectedSegment
    case expansionState
    case draftSnapshot
    case retiredSessionTombstone
}

public enum GaryxRouteStateFieldValue: Equatable, Codable, Sendable {
    case string(String)
    case integer(Int64)
    case flag(Bool)
    case strings([String])
    case bytes(Data)

    public var estimatedCostBytes: Int {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return (try? encoder.encode(self).count) ?? 0
    }
}

public struct GaryxRouteStateStoreMetrics: Equatable, Sendable {
    public let totalEntryCount: Int
    public let totalCostBytes: Int
    public let pinnedEntryCount: Int
    public let pinnedCostBytes: Int
    public let evictableEntryCount: Int
    public let evictableCostBytes: Int
    public let pinnedBudgetFaultCount: Int

    public init(
        totalEntryCount: Int,
        totalCostBytes: Int,
        pinnedEntryCount: Int,
        pinnedCostBytes: Int,
        evictableEntryCount: Int,
        evictableCostBytes: Int,
        pinnedBudgetFaultCount: Int
    ) {
        self.totalEntryCount = totalEntryCount
        self.totalCostBytes = totalCostBytes
        self.pinnedEntryCount = pinnedEntryCount
        self.pinnedCostBytes = pinnedCostBytes
        self.evictableEntryCount = evictableEntryCount
        self.evictableCostBytes = evictableCostBytes
        self.pinnedBudgetFaultCount = pinnedBudgetFaultCount
    }
}

public struct GaryxRouteStateStore: Sendable {
    public static let defaultMaximumEvictableEntries = 32
    public static let defaultMaximumEvictableCostBytes = 2 * 1_024 * 1_024

    private struct Entry: Sendable {
        var fields: [GaryxRouteStateField: GaryxRouteStateFieldValue]
        var lastAccess: UInt64
        var estimatedCostBytes: Int
    }

    public let maximumEvictableEntries: Int
    public let maximumEvictableCostBytes: Int
    private var entries: [GaryxRoutePresentationIdentity: Entry]
    private var pinned: Set<GaryxRoutePresentationIdentity>
    private var accessClock: UInt64
    private var pinnedBudgetFaultCount: Int

    public init(
        maximumEvictableEntries: Int = Self.defaultMaximumEvictableEntries,
        maximumEvictableCostBytes: Int = Self.defaultMaximumEvictableCostBytes
    ) {
        precondition(maximumEvictableEntries >= 0)
        precondition(maximumEvictableCostBytes >= 0)
        self.maximumEvictableEntries = maximumEvictableEntries
        self.maximumEvictableCostBytes = maximumEvictableCostBytes
        entries = [:]
        pinned = []
        accessClock = 0
        pinnedBudgetFaultCount = 0
    }

    public var metrics: GaryxRouteStateStoreMetrics {
        let pinnedEntries = entries.filter { pinned.contains($0.key) }
        let evictableEntries = entries.filter { !pinned.contains($0.key) }
        let pinnedCost = pinnedEntries.values.reduce(0) { $0 + $1.estimatedCostBytes }
        let evictableCost = evictableEntries.values.reduce(0) { $0 + $1.estimatedCostBytes }
        return GaryxRouteStateStoreMetrics(
            totalEntryCount: entries.count,
            totalCostBytes: pinnedCost + evictableCost,
            pinnedEntryCount: pinnedEntries.count,
            pinnedCostBytes: pinnedCost,
            evictableEntryCount: evictableEntries.count,
            evictableCostBytes: evictableCost,
            pinnedBudgetFaultCount: pinnedBudgetFaultCount
        )
    }

    public mutating func setPinned(
        _ isPinned: Bool,
        identity: GaryxRoutePresentationIdentity
    ) {
        if isPinned {
            pinned.insert(identity)
        } else {
            pinned.remove(identity)
        }
        enforceBudget()
    }

    public mutating func set(
        _ value: GaryxRouteStateFieldValue?,
        field: GaryxRouteStateField,
        identity: GaryxRoutePresentationIdentity
    ) {
        accessClock &+= 1
        var entry = entries[identity] ?? Entry(
            fields: [:],
            lastAccess: accessClock,
            estimatedCostBytes: 0
        )
        entry.lastAccess = accessClock
        if let oldValue = entry.fields[field] {
            entry.estimatedCostBytes -= field.rawValue.utf8.count + oldValue.estimatedCostBytes
        }
        entry.fields[field] = value
        if let value {
            entry.estimatedCostBytes += field.rawValue.utf8.count + value.estimatedCostBytes
        }
        if entry.fields.isEmpty {
            entries.removeValue(forKey: identity)
            pinned.remove(identity)
        } else {
            entries[identity] = entry
        }
        enforceBudget()
    }

    public mutating func value(
        field: GaryxRouteStateField,
        identity: GaryxRoutePresentationIdentity
    ) -> GaryxRouteStateFieldValue? {
        guard var entry = entries[identity] else { return nil }
        accessClock &+= 1
        entry.lastAccess = accessClock
        entries[identity] = entry
        return entry.fields[field]
    }

    public mutating func removePermanently(
        identity: GaryxRoutePresentationIdentity
    ) {
        entries.removeValue(forKey: identity)
        pinned.remove(identity)
    }

    public mutating func removeAll() {
        entries.removeAll(keepingCapacity: false)
        pinned.removeAll(keepingCapacity: false)
    }

    private mutating func enforceBudget() {
        let pinnedMetrics = metrics
        if pinnedMetrics.pinnedEntryCount > maximumEvictableEntries
            || pinnedMetrics.pinnedCostBytes > maximumEvictableCostBytes {
            pinnedBudgetFaultCount &+= 1
        }

        while true {
            let current = metrics
            guard current.evictableEntryCount > maximumEvictableEntries
                    || current.evictableCostBytes > maximumEvictableCostBytes
            else { break }
            guard let victim = entries
                .filter({ !pinned.contains($0.key) })
                .min(by: { $0.value.lastAccess < $1.value.lastAccess })?.key
            else { break }
            entries.removeValue(forKey: victim)
        }
    }
}
