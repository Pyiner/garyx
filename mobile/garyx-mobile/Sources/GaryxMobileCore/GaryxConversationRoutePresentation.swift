import CoreGraphics
import Foundation

/// The transcript implementation presented for one conversation occurrence.
///
/// Production header and composer chrome are live from the first destination
/// frame. Only the heavier transcript graph participates in the staged
/// opening/materialization handoff: cached transcript pixels or the shared
/// message-local loading treatment cover that region until delivered frames
/// prove the live transcript stable.
public enum GaryxConversationRouteRenderPhase: String, Equatable, Sendable {
    case openingPage
    case materializingConversation
    case live
}

/// Chooses the presentation pipeline once for a conversation route occurrence.
///
/// A draft is a complete local surface, so it mounts the final transcript
/// immediately and never creates the gateway-thread opening state machine.
/// Existing threads retain their staged transcript cover, prewarm handoff, and
/// delivered-frame stability gates. The app keeps this plan stable when a
/// draft is promoted in place so promotion cannot introduce a loading cover.
public enum GaryxConversationRoutePresentationPlan: Equatable, Sendable {
    case directLocal
    case stagedGatewayThread

    public var mountsFinalChromeOnFirstFrame: Bool {
        self == .directLocal
    }

    public var usesOpeningMaterializationStateMachine: Bool {
        self == .stagedGatewayThread
    }
}

public enum GaryxConversationRoutePresentationPolicy {
    public static func plan(
        for destination: GaryxRouteDestination
    ) -> GaryxConversationRoutePresentationPlan? {
        switch destination {
        case .conversation:
            .stagedGatewayThread
        case .conversationDraft:
            .directLocal
        case .panel, .settingsDetail, .workspaceDrilldown:
            nil
        }
    }
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

/// Scroll geometry retained with one compositor snapshot. The viewport frame
/// is expressed in the owning route page's coordinate space so outer route
/// transition transforms do not become part of transcript placement.
public struct GaryxConversationTranscriptSnapshotCaptureGeometry: Equatable, Sendable {
    public struct Insets: Equatable, Sendable {
        public let top: CGFloat
        public let left: CGFloat
        public let bottom: CGFloat
        public let right: CGFloat

        public init(top: CGFloat, left: CGFloat, bottom: CGFloat, right: CGFloat) {
            self.top = top
            self.left = left
            self.bottom = bottom
            self.right = right
        }
    }

    public let viewportFrameInPage: CGRect
    public let adjustedContentInsets: Insets
    public let contentOffset: CGPoint

    public init(
        viewportFrameInPage: CGRect,
        adjustedContentInsets: Insets,
        contentOffset: CGPoint
    ) {
        self.viewportFrameInPage = viewportFrameInPage
        self.adjustedContentInsets = adjustedContentInsets
        self.contentOffset = contentOffset
    }

    public func snapshotPoint(forContentPoint point: CGPoint) -> CGPoint {
        CGPoint(
            x: point.x - contentOffset.x,
            y: point.y - contentOffset.y
        )
    }
}

public enum GaryxConversationTranscriptSnapshotGeometry {
    public static func installationFrame(
        capture: GaryxConversationTranscriptSnapshotCaptureGeometry,
        containerFrameInPage: CGRect
    ) -> CGRect {
        CGRect(
            x: capture.viewportFrameInPage.minX - containerFrameInPage.minX,
            y: capture.viewportFrameInPage.minY - containerFrameInPage.minY,
            width: capture.viewportFrameInPage.width,
            height: capture.viewportFrameInPage.height
        )
    }
}

/// Render inputs owned by one mounted conversation route occurrence.
///
/// The route chooses the local body pool; `GaryxMobileRenderStateMapper`
/// remains the dumb adapter from the server snapshot plus that pool into
/// mobile rows. Keeping this selection in Core makes draft/thread promotion
/// testable without mounting SwiftUI.
struct GaryxConversationRouteRenderInput: Equatable {
    let messages: [GaryxMobileMessage]
    let snapshot: GaryxRenderSnapshot?
    let transcriptMessages: [GaryxTranscriptMessage]

    /// Client-owned pending-ack chrome for a user row that has not appeared
    /// in committed history yet. This is deliberately separate from the
    /// server-owned `snapshot.tailActivity` value.
    var showsPendingAcknowledgement: Bool {
        messages.contains { message in
            message.role == .user
                && message.localState == .optimistic
                && message.statusText == nil
        }
    }

    /// The existing transcript bubble is shared by server thinking and the
    /// explicitly permitted optimistic pending-ack window. No transport or
    /// run-projection state participates in this decision.
    var showsTailThinking: Bool {
        snapshot?.tailActivity == .thinking || showsPendingAcknowledgement
    }
}

enum GaryxConversationRouteRenderInputResolver {
    static func resolve(
        destination: GaryxRouteDestination,
        draftMessages: [GaryxMobileMessage],
        threadMessages: [GaryxMobileMessage],
        threadSnapshot: GaryxRenderSnapshot?,
        threadTranscriptMessages: [GaryxTranscriptMessage]
    ) -> GaryxConversationRouteRenderInput {
        switch destination {
        case .conversation:
            return GaryxConversationRouteRenderInput(
                messages: threadMessages,
                snapshot: threadSnapshot,
                transcriptMessages: threadTranscriptMessages
            )
        case .conversationDraft:
            return GaryxConversationRouteRenderInput(
                messages: draftMessages,
                snapshot: nil,
                transcriptMessages: []
            )
        case .panel, .settingsDetail, .workspaceDrilldown:
            preconditionFailure("conversation render input requires a conversation destination")
        }
    }
}

/// Core-owned lifecycle and delivered-frame policy for a staged gateway thread.
///
/// The route page, including its production composer, is visible from mount.
/// Once terminal, the first delivered opening frame closes the moving
/// transition before transcript runtime work begins; two delivered opening
/// frames separate navigation settle from the expensive live transcript mount.
/// A short run of consecutive on-budget frames then proves that mount is
/// composited before the transcript cover is removed. This materialization
/// clock deliberately has no network-readiness input: history refresh may
/// control message-local loading and header chrome, but can never retain the
/// transcript cover or lock the composer. Already live predecessor hosts remain
/// live while inactive so back navigation never reconstructs them. Local drafts
/// are excluded by
/// `GaryxConversationRoutePresentationPolicy` and never instantiate this state.
public struct GaryxConversationRoutePresentationState: Equatable, Sendable {
    public static let defaultTerminalOpeningFrameCount = 2
    public static let defaultMaterializationFrameCount = 12

    public private(set) var lifecycle: GaryxRouteHostLifecyclePhase
    public private(set) var renderPhase: GaryxConversationRouteRenderPhase
    public private(set) var hasBegunContentPreparation: Bool
    public private(set) var hasPresentedLiveTranscript: Bool

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
        hasBegunContentPreparation = false
        hasPresentedLiveTranscript = false
    }

    /// The thread page is always the full-screen route surface. Only the
    /// implementation behind that page changes after terminal.
    public var presentsConversationPage: Bool { true }

    /// A route-level skeleton or plain cover is forbidden by policy.
    public var showsFullScreenPlaceholder: Bool { false }

    public var mountsLiveTranscript: Bool {
        renderPhase != .openingPage
    }

    public var showsOpeningTranscriptCover: Bool {
        renderPhase != .live
    }

    /// Interaction belongs to the real transcript only after the compositor
    /// handoff. Before then, its cover is a brief transition-continuity layer
    /// and must not outlive the materialization stability proof.
    public var allowsTranscriptInteraction: Bool {
        renderPhase == .live
    }

    /// Transcript staging is never a composer lock. Canonical-route ownership
    /// and durable payload readiness are enforced independently by the route
    /// and composer coordinators.
    public var allowsComposerInteraction: Bool { true }

    public var needsPresentedFrameClock: Bool {
        lifecycle == .active && !hasPresentedLiveTranscript
    }

    public mutating func apply(
        lifecycle nextLifecycle: GaryxRouteHostLifecyclePhase
    ) {
        guard lifecycle != nextLifecycle else { return }
        lifecycle = nextLifecycle

        if hasPresentedLiveTranscript {
            renderPhase = .live
            deliveredFramesInPhase = 0
            return
        }

        guard nextLifecycle == .active else {
            renderPhase = .openingPage
            hasBegunContentPreparation = false
            deliveredFramesInPhase = 0
            referenceFrameInterval = nil
            return
        }
    }

    /// Records one frame delivered after terminal activation. `nil` begins a
    /// fresh measurement series and never proves materialization stability.
    @discardableResult
    public mutating func presentedFrame(
        interval: TimeInterval?
    ) -> GaryxConversationRouteRenderPhase {
        guard lifecycle == .active, !hasPresentedLiveTranscript else {
            return renderPhase
        }

        switch renderPhase {
        case .openingPage:
            if !hasBegunContentPreparation {
                // Keep terminal activation itself free of transcript work.
                // The exact thread page is already visible, including cached
                // rows or its message-local skeleton and header spinner.
                hasBegunContentPreparation = true
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
            guard let interval,
                  interval > 0
            else {
                deliveredFramesInPhase = 0
                return renderPhase
            }
            guard let referenceFrameInterval else {
                // The transition may reach materialization before UIKit has
                // delivered an interval sample. Establish a fresh reference
                // here instead of leaving the reveal proof unable to advance.
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
            hasPresentedLiveTranscript = true
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
