import Foundation

/// The full-screen conversation surface presented for one route occurrence.
///
/// The opening surface is already a complete thread page: navigation chrome,
/// title, actions, composer, and a transcript-local loading treatment. It is
/// deliberately not a plain background or a whole-page skeleton. The heavier
/// live graph materializes only after the moving push reaches terminal, then
/// replaces the pixel-equivalent opening page after delivered frames are
/// stable.
public enum GaryxConversationRouteRenderPhase: String, Equatable, Sendable {
    case openingPage
    case materializingConversation
    case live
}

/// Message availability is independent of full-page presentation. A thread
/// page may already be live while its transcript is still loading.
public enum GaryxConversationRouteMessagePhase: String, Equatable, Sendable {
    case waitingForActivation
    case loading
    case ready
}

public enum GaryxConversationRoutePresentationAction: Equatable, Sendable {
    case none
    case beginMessagePreparation
}

/// Selects only the transcript treatment for the first destination frame.
/// Existing local messages are content, even while the gateway refresh is in
/// flight; a message skeleton is valid only when there is nothing local to
/// render. Page chrome is never part of this decision.
public enum GaryxConversationOpeningTranscriptPresentation: Equatable, Sendable {
    case localMessages
    case loading
}

public enum GaryxConversationOpeningTranscriptPolicy {
    public static func presentation(
        localRenderableRowCount: Int,
        hasRenderedSnapshot: Bool = false
    ) -> GaryxConversationOpeningTranscriptPresentation {
        localRenderableRowCount > 0 || hasRenderedSnapshot ? .localMessages : .loading
    }
}

/// Core-owned lifecycle and delivered-frame policy for a conversation route.
///
/// The route page is visible from mount. Once terminal, the first delivered
/// opening-page frame closes the moving transition before message preparation
/// begins; two delivered opening-page frames separate navigation settle from
/// the expensive live SwiftUI mount. A short run of consecutive on-budget
/// frames then proves that mount is composited before the opening page is
/// removed. Already live predecessor hosts remain live while inactive so back
/// navigation never reconstructs them.
public struct GaryxConversationRoutePresentationState: Equatable, Sendable {
    public static let defaultTerminalOpeningFrameCount = 2
    public static let defaultMaterializationFrameCount = 12

    public private(set) var lifecycle: GaryxRouteHostLifecyclePhase
    public private(set) var renderPhase: GaryxConversationRouteRenderPhase
    public private(set) var messagePhase: GaryxConversationRouteMessagePhase
    public private(set) var hasPresentedLiveConversation: Bool

    private let terminalOpeningFrameCount: Int
    private let materializationFrameCount: Int
    private var deliveredFramesInPhase = 0
    private var referenceFrameInterval: TimeInterval?

    public init(
        lifecycle: GaryxRouteHostLifecyclePhase = .mounted,
        terminalOpeningFrameCount: Int = Self.defaultTerminalOpeningFrameCount,
        materializationFrameCount: Int = Self.defaultMaterializationFrameCount
    ) {
        precondition(terminalOpeningFrameCount > 0)
        precondition(materializationFrameCount > 0)
        self.lifecycle = lifecycle
        self.terminalOpeningFrameCount = terminalOpeningFrameCount
        self.materializationFrameCount = materializationFrameCount
        renderPhase = .openingPage
        messagePhase = .waitingForActivation
        hasPresentedLiveConversation = false
    }

    /// The thread page is always the full-screen route surface. Only the
    /// implementation behind that page changes after terminal.
    public var presentsConversationPage: Bool { true }

    /// A route-level skeleton or plain cover is forbidden by policy.
    public var showsFullScreenPlaceholder: Bool { false }

    public var mountsLiveConversation: Bool {
        renderPhase != .openingPage
    }

    public var showsOpeningPage: Bool {
        renderPhase != .live
    }

    public var showsMessageLoading: Bool {
        messagePhase != .ready
    }

    public var needsPresentedFrameClock: Bool {
        guard lifecycle == .active, !hasPresentedLiveConversation else { return false }
        return renderPhase == .openingPage || messagePhase == .ready
    }

    @discardableResult
    public mutating func apply(
        lifecycle nextLifecycle: GaryxRouteHostLifecyclePhase
    ) -> GaryxConversationRoutePresentationAction {
        guard lifecycle != nextLifecycle else { return .none }
        lifecycle = nextLifecycle

        if hasPresentedLiveConversation {
            renderPhase = .live
            deliveredFramesInPhase = 0
            return .none
        }

        guard nextLifecycle == .active else {
            renderPhase = .openingPage
            messagePhase = .waitingForActivation
            deliveredFramesInPhase = 0
            referenceFrameInterval = nil
            return .none
        }

        return .none
    }

    public mutating func messageContentDidBecomeReady() {
        guard lifecycle == .active, messagePhase == .loading else { return }
        messagePhase = .ready
    }

    /// Records one frame delivered after terminal activation. `nil` begins a
    /// fresh measurement series and never proves materialization stability.
    @discardableResult
    public mutating func presentedFrame(
        interval: TimeInterval?
    ) -> GaryxConversationRouteRenderPhase {
        guard lifecycle == .active, !hasPresentedLiveConversation else {
            return renderPhase
        }

        switch renderPhase {
        case .openingPage:
            if messagePhase == .waitingForActivation {
                // Keep terminal activation itself free of transcript work.
                // The exact thread page is already visible, including cached
                // rows or its message-local skeleton and header spinner.
                messagePhase = .loading
            }
            if let interval, interval > 0 {
                referenceFrameInterval = min(referenceFrameInterval ?? interval, interval)
            }
            deliveredFramesInPhase += 1
            guard deliveredFramesInPhase >= terminalOpeningFrameCount else {
                return renderPhase
            }
            renderPhase = .materializingConversation
            deliveredFramesInPhase = 0

        case .materializingConversation:
            guard messagePhase == .ready,
                  let interval,
                  interval > 0
            else {
                deliveredFramesInPhase = 0
                return renderPhase
            }
            guard let referenceFrameInterval else {
                // Content readiness deliberately resets the delivered-frame
                // clock. If that reset coincides with the second opening
                // frame, materialization begins without an interval sample;
                // establish a fresh reference here instead of leaving the
                // reveal proof permanently unable to advance.
                self.referenceFrameInterval = interval
                deliveredFramesInPhase = 0
                return renderPhase
            }
            let stabilityCeiling = referenceFrameInterval * 1.25 + 0.0005
            guard interval <= stabilityCeiling else {
                deliveredFramesInPhase = 0
                return renderPhase
            }
            deliveredFramesInPhase += 1
            guard deliveredFramesInPhase >= materializationFrameCount else {
                return renderPhase
            }
            renderPhase = .live
            hasPresentedLiveConversation = true
            deliveredFramesInPhase = 0

        case .live:
            break
        }
        return renderPhase
    }
}

public enum GaryxConversationRenderPrewarmPhase: String, Equatable, Sendable {
    case pending
    case materializing
    case ready
}

/// Delivered-frame proof that startup warm-up has completed its first
/// RenderBox/Metal materialization before a user can start a route push.
public struct GaryxConversationRenderPrewarmState: Equatable, Sendable {
    public static let defaultStableFrameCount = 12

    public private(set) var phase: GaryxConversationRenderPrewarmPhase = .pending

    private let requiredStableFrameCount: Int
    private var stableFrameCount = 0

    public init(requiredStableFrameCount: Int = Self.defaultStableFrameCount) {
        precondition(requiredStableFrameCount > 0)
        self.requiredStableFrameCount = requiredStableFrameCount
    }

    public var rendersWarmupSurface: Bool { phase != .ready }

    public mutating func begin() {
        guard phase == .pending else { return }
        phase = .materializing
        stableFrameCount = 0
    }

    @discardableResult
    public mutating func presentedFrame(
        interval: TimeInterval?,
        frameBudget: TimeInterval
    ) -> GaryxConversationRenderPrewarmPhase {
        guard phase == .materializing,
              let interval,
              interval > 0,
              frameBudget > 0
        else { return phase }

        let stabilityCeiling = frameBudget * 1.5 + 0.0005
        if interval <= stabilityCeiling {
            stableFrameCount += 1
        } else {
            stableFrameCount = 0
        }
        if stableFrameCount >= requiredStableFrameCount {
            phase = .ready
            stableFrameCount = 0
        }
        return phase
    }
}
