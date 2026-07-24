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

/// The one visible treatment for a conversation transcript region.
///
/// This value is derived from live inputs whenever they change. It is never
/// cached in route-opening metadata, so an asynchronous disk restore naturally
/// replaces the skeleton with content without creating a second presentation
/// authority.
public enum GaryxConversationTranscriptTreatment: Equatable, Sendable {
    case skeleton
    case content
}

/// The complete live input needed to decide the transcript treatment.
///
/// A server-owned render snapshot or already-rendered transcript pixels are
/// both renderable content. When neither exists, the initial-history oracle
/// decides between the shared skeleton and the ordinary content branch (which
/// may contain the settled empty state).
public enum GaryxConversationTranscriptTreatmentPolicy {
    public static func treatment(
        localRenderableRowCount: Int,
        hasRenderedSnapshot: Bool,
        hasTranscriptSnapshotPixels: Bool = false,
        isAwaitingInitialHistory: Bool
    ) -> GaryxConversationTranscriptTreatment {
        if localRenderableRowCount > 0
            || hasRenderedSnapshot
            || hasTranscriptSnapshotPixels
        {
            return .content
        }
        return isAwaitingInitialHistory ? .skeleton : .content
    }
}

/// Inputs shared by the Core composition policy and the route presentation
/// state. Keeping this as one value prevents the cover and live transcript
/// from deriving different treatments for the same frame.
public struct GaryxConversationTranscriptPresentationInput: Equatable, Sendable {
    public let treatment: GaryxConversationTranscriptTreatment
    public let hasTranscriptSnapshotPixels: Bool

    public init(
        treatment: GaryxConversationTranscriptTreatment,
        hasTranscriptSnapshotPixels: Bool
    ) {
        self.treatment = treatment
        self.hasTranscriptSnapshotPixels = hasTranscriptSnapshotPixels
    }
}

/// A truthful, opaque continuity surface shown while the live transcript graph
/// materializes. Snapshot pixels are content; the skeleton is the exact same
/// skeleton treatment the mounted live graph would render.
public enum GaryxConversationOpeningTranscriptCover: Equatable, Sendable {
    case skeleton
    case snapshotPixels

    public var treatment: GaryxConversationTranscriptTreatment {
        switch self {
        case .skeleton:
            .skeleton
        case .snapshotPixels:
            .content
        }
    }
}

/// Exactly one transcript surface is visible in a frame. The live graph may be
/// mounted behind an opaque opening cover for hitch preparation, but it cannot
/// become a second visible layer.
public enum GaryxConversationTranscriptPresentation: Equatable, Sendable {
    case openingCover(GaryxConversationOpeningTranscriptCover)
    case live(GaryxConversationTranscriptTreatment)

    public var treatment: GaryxConversationTranscriptTreatment {
        switch self {
        case .openingCover(let cover):
            cover.treatment
        case .live(let treatment):
            treatment
        }
    }

    public var showsOpeningCover: Bool {
        if case .openingCover = self {
            return true
        }
        return false
    }
}

/// Core-owned composition of the frame clock and the single live treatment.
///
/// A cover is legal only when it can show the same treatment as the live
/// transcript. Content without cached pixels therefore resolves directly to
/// the live surface, irrespective of the current choreography phase.
public enum GaryxConversationTranscriptPresentationPolicy {
    public static func coverIsLegal(
        for input: GaryxConversationTranscriptPresentationInput
    ) -> Bool {
        input.treatment == .skeleton || input.hasTranscriptSnapshotPixels
    }

    public static func presentation(
        renderPhase: GaryxConversationRouteRenderPhase,
        input: GaryxConversationTranscriptPresentationInput
    ) -> GaryxConversationTranscriptPresentation {
        guard renderPhase != .live else {
            return .live(input.treatment)
        }
        guard coverIsLegal(for: input) else {
            return .live(input.treatment)
        }
        switch input.treatment {
        case .skeleton:
            return .openingCover(.skeleton)
        case .content:
            return .openingCover(.snapshotPixels)
        }
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

    /// The optimistic pending-ack window bypasses only the appearance-side
    /// debounce. Once the committed frame arrives, server thinking takes
    /// ownership without unmounting an already-visible label.
    var tailThinkingPresentationMode: GaryxTailThinkingPresentationMode {
        if showsPendingAcknowledgement {
            return .immediate
        }
        if snapshot?.tailActivity == .thinking {
            return .debounced
        }
        return .hidden
    }

    var showsTailThinking: Bool {
        tailThinkingPresentationMode != .hidden
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
    /// The candidate cadence sample anchoring the current proof.
    ///
    /// This is deliberately not a historical minimum. ProMotion may deliver
    /// the lightweight opening surface at 120 Hz and the materialized
    /// transcript at a stable 60 Hz. Stability means consecutive frames share
    /// a cadence; it does not mean every later frame must remain as fast as the
    /// fastest opening frame.
    private var cadenceReferenceFrameInterval: TimeInterval?

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

    /// Reconciles the choreography clock with the live transcript treatment.
    ///
    /// The policy itself is pure and also drives SwiftUI's visible surface. If
    /// it says an opening cover would be illegal, an active route immediately
    /// promotes to `.live` instead of completing a stability proof behind
    /// stale pixels. Inactive routes defer the lifecycle mutation until
    /// activation while the pure presentation still prevents an illegal frame.
    @discardableResult
    public mutating func reconcileTranscriptPresentation(
        _ input: GaryxConversationTranscriptPresentationInput
    ) -> GaryxConversationTranscriptPresentation {
        let resolved = GaryxConversationTranscriptPresentationPolicy.presentation(
            renderPhase: renderPhase,
            input: input
        )
        guard lifecycle == .active,
              renderPhase != .live,
              case .live = resolved
        else {
            return resolved
        }

        hasBegunContentPreparation = true
        renderPhase = .live
        hasPresentedLiveTranscript = true
        deliveredFramesInPhase = 0
        cadenceReferenceFrameInterval = nil
        return resolved
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
            cadenceReferenceFrameInterval = nil
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
            cadenceReferenceFrameInterval = interval.flatMap {
                $0 > 0 ? $0 : nil
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
                cadenceReferenceFrameInterval = nil
                return renderPhase
            }
            guard let cadenceReferenceFrameInterval else {
                // The transition may reach materialization before UIKit has
                // delivered an interval sample. Establish a fresh reference
                // here instead of leaving the reveal proof unable to advance.
                self.cadenceReferenceFrameInterval = interval
                deliveredFramesInPhase = 0
                return renderPhase
            }

            // Compare the candidate cadence and the delivered sample
            // symmetrically. A discrete display-rate change starts a new proof
            // at the new cadence; compatible jitter advances that proof
            // without drifting its reference. A legitimate 120 -> 60 Hz
            // ProMotion downshift therefore converges instead of remaining
            // permanently judged against the opening cadence.
            let cadenceTolerance =
                min(cadenceReferenceFrameInterval, interval) * 0.25 + 0.0005
            let cadenceChanged =
                abs(interval - cadenceReferenceFrameInterval) > cadenceTolerance
            if cadenceChanged {
                self.cadenceReferenceFrameInterval = interval
                deliveredFramesInPhase = 1
            } else {
                deliveredFramesInPhase += 1
            }
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
