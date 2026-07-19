import Foundation

/// The render phase for a newly mounted conversation route.
///
/// A route's UIKit host must exist before a push can start, but constructing
/// the complete SwiftUI transcript, header materials, and composer in that
/// same frame can make Core Animation compile their RenderBox/Metal surfaces
/// inside the transition window. The route therefore presents a cheap,
/// immutable placeholder first and prepares live content only after terminal.
public enum GaryxConversationRouteRenderPhase: String, Equatable, Sendable {
    /// The only surface mounted while the route transition is moving.
    case transitionPlaceholder
    /// Live content is mounted behind the opaque placeholder to finish its
    /// first display-list commit without exposing a partially rendered page.
    case preparingLiveContent
    /// Live content has presented stable frames and may be revealed.
    case live
}

/// Pure frame/lifecycle policy for the staged conversation route surface.
///
/// This state machine deliberately counts delivered frames instead of using a
/// fixed delay. Two terminal placeholder frames separate the navigation
/// completion from live-content mounting. Once the initial renderable snapshot
/// is ready, consecutive on-budget frames prove the prepared surface has
/// survived its first display-list commits before the placeholder is revealed.
/// Once live, retained predecessor hosts stay live while inactive so back
/// navigation never tears down and rebuilds an already prepared surface.
public struct GaryxConversationRoutePresentationState: Equatable, Sendable {
    public static let defaultTerminalPlaceholderFrameCount = 2
    public static let defaultLivePreparationFrameCount = 3

    public private(set) var lifecycle: GaryxRouteHostLifecyclePhase
    public private(set) var renderPhase: GaryxConversationRouteRenderPhase
    public private(set) var hasPresentedLiveContent: Bool

    private let terminalPlaceholderFrameCount: Int
    private let livePreparationFrameCount: Int
    private var deliveredFramesInPhase = 0
    private var referenceFrameInterval: TimeInterval?
    private var liveContentIsReady = false

    public init(
        lifecycle: GaryxRouteHostLifecyclePhase = .mounted,
        terminalPlaceholderFrameCount: Int = Self.defaultTerminalPlaceholderFrameCount,
        livePreparationFrameCount: Int = Self.defaultLivePreparationFrameCount
    ) {
        precondition(terminalPlaceholderFrameCount > 0)
        precondition(livePreparationFrameCount > 0)
        self.lifecycle = lifecycle
        self.terminalPlaceholderFrameCount = terminalPlaceholderFrameCount
        self.livePreparationFrameCount = livePreparationFrameCount
        renderPhase = .transitionPlaceholder
        hasPresentedLiveContent = false
    }

    public var mountsLiveContent: Bool {
        renderPhase != .transitionPlaceholder
    }

    public var showsPlaceholder: Bool {
        renderPhase != .live
    }

    public var needsPresentedFrameClock: Bool {
        guard lifecycle == .active, !hasPresentedLiveContent else { return false }
        return renderPhase == .transitionPlaceholder || liveContentIsReady
    }

    /// Applies the container-owned route lifecycle projection.
    ///
    /// Non-terminal destinations remain placeholder-only. If a transaction is
    /// cancelled or superseded before live preparation finishes, all partial
    /// frame progress is discarded. A host that has already reached `live`
    /// retains that surface through inactive predecessor residency.
    @discardableResult
    public mutating func apply(
        lifecycle nextLifecycle: GaryxRouteHostLifecyclePhase
    ) -> GaryxConversationRouteRenderPhase {
        guard lifecycle != nextLifecycle else { return renderPhase }
        lifecycle = nextLifecycle

        if hasPresentedLiveContent {
            renderPhase = .live
            deliveredFramesInPhase = 0
        } else if nextLifecycle != .active {
            renderPhase = .transitionPlaceholder
            deliveredFramesInPhase = 0
            referenceFrameInterval = nil
            liveContentIsReady = false
        }
        return renderPhase
    }

    /// Marks the initial renderable snapshot ready for presentation. The
    /// prepared surface remains hidden until it subsequently delivers a run of
    /// on-budget frames.
    public mutating func contentDidBecomeReady() {
        guard lifecycle == .active,
              renderPhase == .preparingLiveContent,
              !hasPresentedLiveContent
        else { return }
        liveContentIsReady = true
        deliveredFramesInPhase = 0
    }

    /// Records one frame delivered after the route became active. The interval
    /// is measured between actual display-link deliveries; `nil` begins a new
    /// measurement series and never counts as proof of stability.
    @discardableResult
    public mutating func presentedFrame(
        interval: TimeInterval?
    ) -> GaryxConversationRouteRenderPhase {
        guard lifecycle == .active, !hasPresentedLiveContent else { return renderPhase }

        switch renderPhase {
        case .transitionPlaceholder:
            if let interval, interval > 0 {
                referenceFrameInterval = min(referenceFrameInterval ?? interval, interval)
            }
            deliveredFramesInPhase += 1
            guard deliveredFramesInPhase >= terminalPlaceholderFrameCount else {
                return renderPhase
            }
            renderPhase = .preparingLiveContent
            deliveredFramesInPhase = 0
        case .preparingLiveContent:
            guard liveContentIsReady,
                  let interval,
                  interval > 0,
                  let referenceFrameInterval
            else {
                deliveredFramesInPhase = 0
                return renderPhase
            }
            // Allow ordinary callback jitter without treating a missed frame
            // as stable. The reference comes from the cheap terminal surface,
            // so this adapts to both 60 Hz simulator and 120 Hz device cadence.
            let stabilityCeiling = referenceFrameInterval * 1.25 + 0.0005
            guard interval <= stabilityCeiling else {
                deliveredFramesInPhase = 0
                return renderPhase
            }
            deliveredFramesInPhase += 1
            guard deliveredFramesInPhase >= livePreparationFrameCount else {
                return renderPhase
            }
            renderPhase = .live
            hasPresentedLiveContent = true
            deliveredFramesInPhase = 0
        case .live:
            break
        }
        return renderPhase
    }
}
