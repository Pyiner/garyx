import Foundation

// MARK: - Position calculation

/// Pure position calculation for the conversation transcript.
///
/// Holds the live measurements reported by the scroll view (in the viewport
/// coordinate space) and derives every positional fact the scroll state
/// machine needs. No UI state lives here; this is the single place where
/// transcript geometry math and thresholds are defined.
public struct GaryxConversationLayoutMetrics: Equatable {
    /// Content top edge offset. `nil` until the first layout pass reports it.
    public var contentTopOffset: CGFloat?
    /// Content bottom edge offset.
    public var contentBottomOffset: CGFloat
    /// Intrinsic transcript tail before send-anchor filler and the existing
    /// bottom chrome clearance. This lets the button policy distinguish real
    /// reply content below the viewport from blank run space.
    public var contentTailOffset: CGFloat?
    public var viewportHeight: CGFloat

    public init(
        contentTopOffset: CGFloat? = nil,
        contentBottomOffset: CGFloat = 0,
        contentTailOffset: CGFloat? = nil,
        viewportHeight: CGFloat = 0
    ) {
        self.contentTopOffset = contentTopOffset
        self.contentBottomOffset = contentBottomOffset
        self.contentTailOffset = contentTailOffset
        self.viewportHeight = viewportHeight
    }

    // MARK: Thresholds

    /// Distance from the bottom within which the reader still counts as
    /// anchored to the tail.
    public static let nearBottomThreshold: CGFloat = 96
    public static let historyPrefetchMinDistance: CGFloat = 640
    public static let historyPrefetchViewportMultiplier: CGFloat = 1.5
    /// How far the content top must be pulled below the viewport top (top
    /// rubber band) before the pull counts as a deliberate "show me older
    /// history" gesture instead of scroll jitter.
    public static let topPullIntentThreshold: CGFloat = 24

    // MARK: Derived position facts

    /// How far the content bottom sits below the viewport bottom.
    public var distanceFromBottom: CGFloat {
        contentBottomOffset - viewportHeight
    }

    public var contentHeight: CGFloat? {
        guard let contentTopOffset else { return nil }
        return contentBottomOffset - contentTopOffset
    }

    /// Whether the transcript tail is visible (or the content is shorter
    /// than the viewport, where the tail is always visible).
    public var isNearBottom: Bool {
        guard viewportHeight > 0 else { return true }
        return distanceFromBottom <= Self.nearBottomThreshold
    }

    /// Whether real transcript content, excluding send-anchor filler, extends
    /// below the visible viewport.
    public var isContentTailBelowViewport: Bool {
        guard viewportHeight > 0 else { return false }
        guard let contentTailOffset else { return !isNearBottom }
        return contentTailOffset > viewportHeight
    }

    /// True when scrollable content has been pulled past the bottom, leaving
    /// a gap between the content bottom and the viewport bottom (for example
    /// after a tool-call turn collapses or the keyboard shrinks the
    /// viewport).
    public var hasVisibleTailGap: Bool {
        guard viewportHeight > 0, let contentTopOffset else { return false }
        let bottomAboveViewportBottom =
            contentBottomOffset < viewportHeight - Self.nearBottomThreshold
        let topScrolledAboveViewport = contentTopOffset < -Self.nearBottomThreshold
        return bottomAboveViewportBottom && topScrolledAboveViewport
    }

    /// Whether the loaded history start is close enough to prefetch the next
    /// older page.
    public var isNearLoadedHistoryStart: Bool {
        guard let contentTopOffset, viewportHeight > 0 else { return false }
        let prefetchDistance = max(
            Self.historyPrefetchMinDistance,
            viewportHeight * Self.historyPrefetchViewportMultiplier
        )
        return contentTopOffset >= -prefetchDistance
    }

    /// The content top was pulled below the viewport top (top rubber band).
    /// This is the only geometric "show me older history" signal a short
    /// transcript can produce — content that does not overflow the viewport
    /// can never move the anchoring to `.browsingHistory`.
    public var isPulledPastTop: Bool {
        guard let contentTopOffset, viewportHeight > 0 else { return false }
        return contentTopOffset >= Self.topPullIntentThreshold
    }
}

// MARK: - Atomic content-edge measurement

/// One atomic transcript measurement carrying all relevant content edges.
///
/// The top sentinel, intrinsic tail, and bottom anchor each contribute one
/// part; the view layer reduces them into a single value per layout pass
/// (one SwiftUI preference key), so the scroll state machine only ever
/// observes atomic frames. Feeding the edges through two separate callbacks
/// made every real scroll step look like a content-height change (top moved,
/// bottom not yet), which permanently reset the upward-travel accumulator
/// and broke the pre-iOS 18 reader-intent path (#TASK-2073 P2).
public struct GaryxConversationContentEdges: Equatable {
    public var top: CGFloat?
    public var bottom: CGFloat?
    public var tail: CGFloat?

    public init(
        top: CGFloat? = nil,
        bottom: CGFloat? = nil,
        tail: CGFloat? = nil
    ) {
        self.top = top
        self.bottom = bottom
        self.tail = tail
    }

    /// Combine two contributions; a later non-nil half wins its side.
    /// Merge order between the two emitters does not matter because each
    /// emitter only sets its own half.
    public func merging(_ other: Self) -> Self {
        Self(
            top: other.top ?? top,
            bottom: other.bottom ?? bottom,
            tail: other.tail ?? tail
        )
    }
}

/// Route-scoped value observed by SwiftUI scroll lifecycle handlers.
///
/// The scope is part of equality on purpose: message and render-row ids are
/// only unique inside a thread, so two threads can expose byte-identical value
/// arrays. Observing the value alone suppresses the switch callback and leaves
/// the first real prepend in the new thread indistinguishable from cross-thread
/// replay. Pairing both facts makes every scope transition observable while a
/// cold mount's first same-scope prepend still carries its real old scope.
public struct GaryxConversationScrollObservation<Value: Equatable>: Equatable {
    public let scopeIdentity: String
    public let value: Value
    public let localSendPresentation: GaryxConversationLocalSendPresentation?

    public init(
        scopeIdentity: String,
        value: Value,
        localSendPresentation: GaryxConversationLocalSendPresentation? = nil
    ) {
        self.scopeIdentity = scopeIdentity
        self.value = value
        self.localSendPresentation = localSendPresentation
    }
}

/// Fire-time facts used by the scroll-settlement state machine.
///
/// The adapter only reports these facts. Core owns the decision to authorize
/// a position write, including the existing reader-interaction policy and the
/// target-placement/geometry settlement policy.
public struct GaryxConversationScrollAttemptInput: Equatable {
    public enum TargetPlacement: Equatable {
        /// The scroll surface has not reported a complete geometry frame yet.
        case unknown
        /// The scroll surface currently holds the requested target placement.
        case satisfied
        /// The scroll surface is currently away from the requested placement.
        case unsatisfied
    }

    public let policyAllowsAttempt: Bool
    public let targetPlacement: TargetPlacement
    public let geometryEpoch: UInt64

    public init(
        policyAllowsAttempt: Bool,
        targetPlacement: TargetPlacement,
        geometryEpoch: UInt64
    ) {
        self.policyAllowsAttempt = policyAllowsAttempt
        self.targetPlacement = targetPlacement
        self.geometryEpoch = geometryEpoch
    }
}

// MARK: - Tail thinking presentation

public enum GaryxTailThinkingPresentationMode: Equatable, Sendable {
    case hidden
    /// Server-owned thinking keeps the existing appearance debounce.
    case debounced
    /// A local send presents its optimistic user row and thinking together.
    case immediate
}

/// Presentation-only debounce for the server-owned tail thinking state.
///
/// The raw `tailActivity == .thinking` value still comes from render_state.
/// This state only decides when the label should become visible, so quick
/// thinking-to-text transitions do not flash a stale label.
public struct GaryxTailThinkingPresentationState: Equatable {
    public static let defaultDelay: TimeInterval = 0.2

    public private(set) var isVisible: Bool = false
    private var thinkingStartedAt: TimeInterval?

    public init() {}

    @discardableResult
    public mutating func update(
        mode: GaryxTailThinkingPresentationMode,
        now: TimeInterval,
        delay: TimeInterval = Self.defaultDelay
    ) -> Bool {
        switch mode {
        case .hidden:
            thinkingStartedAt = nil
            isVisible = false
            return isVisible
        case .immediate:
            thinkingStartedAt = nil
            isVisible = true
            return isVisible
        case .debounced:
            // An optimistic immediate label stays mounted when the committed
            // server frame takes ownership; ACK must be visually silent.
            guard !isVisible else { return true }
            if thinkingStartedAt == nil {
                thinkingStartedAt = now
            }
            if let thinkingStartedAt, now - thinkingStartedAt >= delay {
                isVisible = true
            }
            return isVisible
        }
    }

    public func nextVisibilityCheck(
        now: TimeInterval,
        delay: TimeInterval = Self.defaultDelay
    ) -> TimeInterval? {
        guard !isVisible, let thinkingStartedAt else { return nil }
        return max(0, thinkingStartedAt + delay - now)
    }
}

// MARK: - State management

/// Unified scroll state machine for the conversation transcript.
///
/// The transcript is laid out top-down: a short conversation starts at the
/// top of the viewport and never sticks to the composer. Once content
/// overflows the viewport, `anchoring` decides every tail behavior:
///
/// - `.followingTail`: the viewport tracks the transcript tail. New
///   messages, streaming growth, tool activity, the thinking indicator,
///   keyboard appearance, and chrome resizes all keep the tail visible.
/// - `.sendAnchored`: one locally presented user row owns the viewport top.
///   Content grows below it without moving the viewport.
/// - `.browsingHistory`: the reader scrolled up; nothing moves the viewport,
///   and the scroll-to-bottom control is shown instead.
///
/// UI reads projections of this state (`showsScrollToBottomButton`,
/// `isFollowingTail`); the view feeds events in and executes the returned
/// `ScrollRequest`s. Position math lives in
/// `GaryxConversationLayoutMetrics`.
public struct GaryxConversationScrollState: Equatable {
    public enum Anchoring: Equatable {
        case followingTail
        case sendAnchored(anchorRowId: String)
        case browsingHistory
    }

    public enum ScrollReason: Equatable {
        case openingThread
        case localSend
        case tailUpdate
        case manual
        case repair

        public var retryHorizon: ScrollRetryHorizon {
            switch self {
            case .tailUpdate:
                .tailGrowth
            case .openingThread, .localSend, .manual, .repair:
                .settling
            }
        }

        /// Production retry clock consumed by the SwiftUI adapter.
        ///
        /// Keeping the clock beside the authorization policy makes the whole
        /// scroll-write timeline deterministic in SwiftPM tests. The adapter
        /// still owns `ScrollViewProxy.scrollTo`; Core owns when its queued
        /// attempts become eligible.
        public var retryDelayMilliseconds: [Int] {
            switch self {
            case .tailUpdate:
                // Ordinary tail growth during send/streaming should stay
                // pinned, but a long retry chain visibly wobbles the
                // transcript while composer geometry also settles.
                [0, 40, 140]
            case .localSend:
                // The zero-delay attempt can fire before the appended row has
                // laid out; the 50 ms slot catches that case while the send
                // haptic still reads as one moment (the animation and haptic
                // key off the first authorized write, not off index 0). Later
                // slots only re-check placement after the animation settled.
                [0, 50, 320, 650, 1_000]
            case .openingThread, .manual, .repair:
                [0, 16, 40, 140, 320, 650, 1_000]
            }
        }
    }

    public enum ScrollRetryHorizon: Equatable {
        case tailGrowth
        case settling
    }

    public enum ScrollTarget: Equatable {
        case transcriptTail
        case row(id: String)
    }

    public enum ScrollAlignment: Equatable {
        case top
        case bottom
    }

    public struct ScrollRequest: Equatable {
        public let reason: ScrollReason
        public let target: ScrollTarget
        public let alignment: ScrollAlignment
        public let animated: Bool

        public init(
            reason: ScrollReason,
            target: ScrollTarget = .transcriptTail,
            alignment: ScrollAlignment = .bottom,
            animated: Bool
        ) {
            self.reason = reason
            self.target = target
            self.alignment = alignment
            self.animated = animated
        }
    }

    // MARK: State

    public private(set) var anchoring: Anchoring = .followingTail
    public private(set) var metrics = GaryxConversationLayoutMetrics()
    /// Whether the transcript has any tail to follow (messages or a thinking
    /// indicator).
    public private(set) var hasTailContent = false
    /// Whether the reader ever moved toward older history in this thread —
    /// either scrolled away from the tail, or pulled the content top past the
    /// viewport top (the only gesture a non-overflowing transcript affords).
    /// Gates history prefetch so an untouched thread never pages backwards.
    public private(set) var hasMovedTowardOlderHistory = false
    /// Whether the reader's finger or fling currently drives the scroll
    /// view. Programmatic tail scrolls must never fight an active gesture.
    public private(set) var isUserScrollInteracting = false
    /// Whether the reader has scrolled at all since the thread opened.
    /// Until they do, drifting away from the bottom can only be late layout
    /// settling (markdown measuring, async thumbnails), so the tail re-pins
    /// instead of stranding the viewport mid-history. Set from the iOS 18
    /// scroll-phase report, or from sustained upward reading travel across
    /// stable-layout frames (the pure-geometry equivalent used before iOS 18,
    /// where no scroll-phase API exists).
    public private(set) var hasUserScrolledSinceOpen = false
    /// Cumulative upward content travel across frames whose content and
    /// viewport sizes held still. Layout settling always changes the content
    /// height and keyboard/chrome changes always change the viewport, so
    /// sustained stable-layout upward movement can only be the reader.
    private var upwardReadingTravel: CGFloat = 0
    /// Upward travel needed before it counts as a deliberate reader scroll.
    public static let upwardTravelIntentThreshold: CGFloat = 24
    /// Size jitter tolerated while still treating two frames as same-layout.
    private static let stableLayoutTolerance: CGFloat = 2
    /// Tracks the visible-tail-gap level so repairs fire on its rising edge
    /// only. A persistent gap (such as lazy-layout estimation drift around a
    /// collapsed tail row) must not regenerate a repair on every frame, or
    /// the reader can never scroll away from the tail.
    private var hadVisibleTailGap = false
    /// Monotonic signal for transcript geometry that can change the result of
    /// a bottom-anchor write. Content reducers advance it before their request
    /// is scheduled; measured content/viewport size changes advance it again
    /// when late Markdown, image, tool-row, keyboard, or chrome layout lands.
    ///
    /// Pure scrolling moves both content edges together and therefore leaves
    /// this epoch unchanged.
    public private(set) var tailGeometryEpoch: UInt64 = 0

    public init() {}

    // MARK: UI projections

    public var isFollowingTail: Bool {
        anchoring == .followingTail
    }

    /// True while a send-anchor session owns the viewport top. Run space
    /// (blank filler) and the suspended size-change anchor live exactly as
    /// long as this state: any reader gesture, run-space exhaustion, the
    /// scroll-to-bottom control, or a thread switch ends the session and the
    /// view collapses the filler in the same update (v2.1 — the boss's rule:
    /// once touched, the blank below must be gone and ordinary bottom
    /// semantics resume).
    public var isSendAnchored: Bool {
        sendAnchorRowId != nil
    }

    public var sendAnchorRowId: String? {
        guard case .sendAnchored(let anchorRowId) = anchoring else { return nil }
        return anchorRowId
    }

    /// The glass down-arrow above the composer: visible whenever the reader
    /// left the tail and there is a tail to return to.
    public var showsScrollToBottomButton: Bool {
        guard hasTailContent else { return false }
        switch anchoring {
        case .followingTail:
            return false
        case .sendAnchored:
            return metrics.isContentTailBelowViewport
        case .browsingHistory:
            return true
        }
    }

    // MARK: Events

    /// A thread was opened or switched: reset and jump straight to the tail.
    /// The measured viewport survives the reset — it belongs to the scroll
    /// surface, not the thread, and is not re-reported on switch.
    public mutating func threadOpened() -> ScrollRequest {
        let viewportHeight = metrics.viewportHeight
        self = GaryxConversationScrollState()
        metrics.viewportHeight = viewportHeight
        return ScrollRequest(reason: .openingThread, animated: false)
    }

    /// The optimistic user row was appended locally. This is the only event
    /// that starts a send-anchor request chain.
    public mutating func localSendPresented(anchorRowId: String) -> ScrollRequest {
        anchoring = .sendAnchored(anchorRowId: anchorRowId)
        hasTailContent = true
        markTailGeometryChanged()
        return ScrollRequest(
            reason: .localSend,
            target: .row(id: anchorRowId),
            alignment: .top,
            animated: true
        )
    }

    /// Transcript content changed.
    ///
    /// - Initial load jumps to the tail without animation.
    /// - Older-history prepends never move the viewport; the view preserves
    ///   the reading position.
    /// - Tail growth (new messages, streaming text, tool activity) follows
    ///   the tail only while `.followingTail`; a browsing reader is never
    ///   yanked.
    public mutating func contentChanged(
        isInitialLoad: Bool,
        isHistoryPrepend: Bool,
        hasTailContent: Bool
    ) -> ScrollRequest? {
        markTailGeometryChanged()
        self.hasTailContent = hasTailContent
        guard hasTailContent, !isHistoryPrepend else { return nil }
        if case .sendAnchored = anchoring {
            return nil
        }
        if isInitialLoad {
            anchoring = .followingTail
            return ScrollRequest(reason: .openingThread, animated: false)
        }
        guard isFollowingTail else { return nil }
        return ScrollRequest(reason: .tailUpdate, animated: false)
    }

    /// Route-scoped message geometry changed. The observed values deliberately
    /// exclude storage-only materialization fields, so an optimistic message
    /// becoming committed does not start another scroll chain when its
    /// visible layout stayed identical. Identity still comes from the values
    /// themselves, so prepends and cross-thread switches remain unambiguous.
    public mutating func messagesChanged<Layout: Equatable>(
        previous: [Layout],
        current: [Layout],
        id: (Layout) -> String,
        previousScopeIdentity: String,
        currentScopeIdentity: String,
        hasTailContent: Bool
    ) -> ScrollRequest? {
        let threadUnchanged = previousScopeIdentity == currentScopeIdentity
        self.hasTailContent = hasTailContent
        guard !threadUnchanged || previous != current else { return nil }
        let previousIds = previous.map(id)
        let currentIds = current.map(id)
        let isHistoryPrepend = Self.preservesScrollForPrependedHistory(
            previousIds: previousIds,
            currentIds: currentIds,
            threadUnchanged: threadUnchanged
        )
        return contentChanged(
            isInitialLoad: previousIds.isEmpty,
            isHistoryPrepend: isHistoryPrepend,
            hasTailContent: hasTailContent
        )
    }

    /// The tail thinking indicator appeared (run started with no visible
    /// activity yet).
    public mutating func thinkingIndicatorShown() -> ScrollRequest? {
        markTailGeometryChanged()
        hasTailContent = true
        if case .sendAnchored = anchoring {
            return nil
        }
        guard isFollowingTail else { return nil }
        return ScrollRequest(reason: .tailUpdate, animated: false)
    }

    /// Live measurement update from the scroll view. Derives the anchoring
    /// from the reader's position and requests a repair scroll when the tail
    /// drifted away while following.
    ///
    /// `hasTailContent` is re-asserted on every measurement so the state
    /// machine never depends on content-change event ordering around thread
    /// switches.
    public mutating func metricsChanged(
        _ metrics: GaryxConversationLayoutMetrics,
        hasTailContent: Bool
    ) -> ScrollRequest? {
        let previousMetrics = self.metrics
        if Self.tailLayoutGeometryChanged(from: previousMetrics, to: metrics) {
            markTailGeometryChanged()
        }
        self.metrics = metrics
        self.hasTailContent = hasTailContent
        guard metrics.viewportHeight > 0 else { return nil }
        trackUpwardReadingTravel(previous: previousMetrics, current: metrics)
        if metrics.isPulledPastTop {
            // A top rubber-band pull is a deliberate reach for older history.
            // Short transcripts can only express the intent this way; recording
            // it here keeps the signal purely geometric, so it also works before
            // iOS 18 where no scroll-phase gesture reporting exists.
            hasMovedTowardOlderHistory = true
        }
        if case .sendAnchored = anchoring {
            // Filler and intrinsic-tail measurements may change on every
            // streamed frame. They update projections only; the anchored row
            // is never re-scrolled from a content or metrics event.
            hadVisibleTailGap = metrics.hasVisibleTailGap
            return nil
        }
        if metrics.isNearBottom {
            anchoring = .followingTail
        } else if isFollowingTail, !hasUserScrolledSinceOpen, !isUserScrollInteracting {
            // The tail drifted away before the reader ever scrolled: late
            // layout settling pushed the content down (heavy markdown,
            // async thumbnails). Stay anchored and pull the tail back —
            // the reader's first real gesture disables this for good.
            return ScrollRequest(reason: .repair, animated: false)
        } else {
            anchoring = .browsingHistory
            hasMovedTowardOlderHistory = true
        }
        // Repairs are edge-triggered on the gap appearing and never start
        // while the reader is dragging: a level-triggered repair regenerates
        // a scroll-to-tail on every measurement frame whenever the gap
        // cannot be closed exactly (lazy layout estimation), which pins the
        // viewport to the bottom and makes scrolling up impossible.
        let gapAppeared = metrics.hasVisibleTailGap && !hadVisibleTailGap
        hadVisibleTailGap = metrics.hasVisibleTailGap
        if isFollowingTail, hasTailContent, gapAppeared, !isUserScrollInteracting {
            return ScrollRequest(reason: .repair, animated: false)
        }
        return nil
    }

    /// Accumulate upward reading travel between two measurement frames and
    /// promote it to a reader-scroll signal once it crosses the intent
    /// threshold. Pure geometry, so it works before iOS 18 (no scroll-phase
    /// API) and is harmlessly redundant with the phase report on iOS 18.
    ///
    /// Only frames whose content height and viewport held still count:
    /// layout settling changes the content height, and keyboard / bottom
    /// chrome changes change the viewport, so neither can masquerade as the
    /// reader. Downward movement (including tail-repair scrolls) resets the
    /// accumulator.
    private mutating func trackUpwardReadingTravel(
        previous: GaryxConversationLayoutMetrics,
        current: GaryxConversationLayoutMetrics
    ) {
        guard let previousTop = previous.contentTopOffset,
              let currentTop = current.contentTopOffset,
              let previousHeight = previous.contentHeight,
              let currentHeight = current.contentHeight,
              abs(currentHeight - previousHeight) <= Self.stableLayoutTolerance,
              abs(current.viewportHeight - previous.viewportHeight) <= Self.stableLayoutTolerance
        else {
            upwardReadingTravel = 0
            return
        }
        let delta = currentTop - previousTop
        if delta > 0 {
            upwardReadingTravel += delta
        } else if delta < 0 {
            upwardReadingTravel = 0
        }
        if upwardReadingTravel >= Self.upwardTravelIntentThreshold {
            hasUserScrolledSinceOpen = true
        }
    }

    /// The reader's scroll gesture started or ended (finger down, or a fling
    /// still decelerating). While interacting, no programmatic tail scroll
    /// may run. When the interaction ends over a visible tail gap while
    /// still following, one repair closes it.
    public mutating func userScrollInteractionChanged(isInteracting: Bool) -> ScrollRequest? {
        guard isUserScrollInteracting != isInteracting else { return nil }
        isUserScrollInteracting = isInteracting
        if isInteracting {
            hasUserScrolledSinceOpen = true
            if case .sendAnchored = anchoring {
                anchoring = .browsingHistory
                hasMovedTowardOlderHistory = true
            }
        }
        guard !isInteracting,
              isFollowingTail,
              hasTailContent,
              metrics.hasVisibleTailGap else {
            return nil
        }
        return ScrollRequest(reason: .repair, animated: false)
    }

    /// The composer gained focus. Keep the tail visible above the keyboard
    /// while following; never move a reader who is browsing history.
    public mutating func composerFocused() -> ScrollRequest? {
        if case .sendAnchored = anchoring {
            return nil
        }
        guard isFollowingTail, hasTailContent else { return nil }
        return ScrollRequest(reason: .manual, animated: true)
    }

    /// The floating bottom chrome (composer tray) changed height.
    ///
    /// While send-anchored this is a no-op: chrome height only trims the
    /// viewport's bottom edge, so the row anchored at the top edge does not
    /// move. v1 issued an unanimated re-anchor here, which visibly snapped
    /// the transcript on every keyboard/tray height change.
    public mutating func bottomChromeChanged() -> ScrollRequest? {
        markTailGeometryChanged()
        if case .sendAnchored = anchoring {
            return nil
        }
        guard isFollowingTail, hasTailContent else { return nil }
        return ScrollRequest(reason: .repair, animated: false)
    }

    /// The reader tapped the scroll-to-bottom control: resume following.
    /// Leaving a send-anchor session here collapses its filler in the same
    /// view update, so the tail this scroll lands on is the real content
    /// tail.
    public mutating func scrollToBottomTapped() -> ScrollRequest {
        anchoring = .followingTail
        return ScrollRequest(reason: .manual, animated: false)
    }

    /// The send-anchor run space is exhausted: intrinsic reply content below
    /// the anchored row filled the session floor, so the blank filler is
    /// already zero. A reply longer than one screen is followed (product
    /// decision 2026-07-24): the anchored session hands off seamlessly to
    /// tail following — the viewport is already effectively at the content
    /// bottom, so one short animated settle engages the system size-change
    /// pinning without a visible jump. Outside an anchored session this is
    /// a no-op (a reader gesture already ended the session, v2.1).
    public mutating func sendRunSpaceExhausted() -> ScrollRequest? {
        guard case .sendAnchored = anchoring else { return nil }
        anchoring = .followingTail
        markTailGeometryChanged()
        return ScrollRequest(reason: .tailUpdate, animated: true)
    }

    // MARK: Scheduled scroll retries

    /// Complete fire-time input for the Core-owned settlement scheduler.
    ///
    /// A complete layout frame is required before placement can settle. Until
    /// both content edges and the viewport are known, retries remain eligible
    /// so an early zero-delay attempt cannot terminate the chain before the
    /// transcript materializes.
    public func scrollAttemptInput(
        index: Int,
        request: ScrollRequest,
        rowTargetViewportOffset: CGFloat? = nil,
        chainHasWritten: Bool = false
    ) -> GaryxConversationScrollAttemptInput {
        let targetPlacement: GaryxConversationScrollAttemptInput.TargetPlacement
        let geometryEpoch: UInt64
        switch request.target {
        case .transcriptTail:
            geometryEpoch = tailGeometryEpoch
            if metrics.viewportHeight <= 0 || metrics.contentTopOffset == nil {
                targetPlacement = .unknown
            } else if metrics.isNearBottom {
                targetPlacement = .satisfied
            } else {
                targetPlacement = .unsatisfied
            }
        case .row:
            // Reply growth below a top-anchored row does not invalidate its
            // placement. A constant epoch lets the chain settle as soon as
            // the adapter observes the row at the viewport top.
            geometryEpoch = 0
            if let rowTargetViewportOffset {
                targetPlacement =
                    abs(rowTargetViewportOffset) <= Self.stableLayoutTolerance
                    ? .satisfied
                    : .unsatisfied
            } else {
                targetPlacement = .unknown
            }
        }
        return GaryxConversationScrollAttemptInput(
            policyAllowsAttempt: shouldRunScrollAttempt(
                index: index,
                request: request,
                chainHasWritten: chainHasWritten
            ),
            targetPlacement: targetPlacement,
            geometryEpoch: geometryEpoch
        )
    }

    /// Whether a delayed retry of a scheduled scroll should still run.
    ///
    /// Nothing but the reader's finger may move the viewport while a scroll
    /// gesture is active. After that, opening jumps and explicit manual
    /// scrolls always retry; tail updates and repairs are dropped as soon as
    /// the reader leaves the tail, so a streaming run can never pin a reader
    /// who is scrolling up toward history.
    public func shouldRunScrollAttempt(
        index: Int,
        request: ScrollRequest,
        chainHasWritten: Bool = false
    ) -> Bool {
        if isUserScrollInteracting, request.reason != .manual { return false }
        if case .row(let id) = request.target, request.reason == .localSend {
            guard sendAnchorRowId == id else { return false }
        }
        guard index > 0 else { return true }
        switch request.reason {
        case .openingThread, .manual:
            return true
        case .localSend:
            guard case .row(let id) = request.target else { return false }
            // The 50ms slot exists solely to catch a zero-delay attempt that
            // fired before the appended row laid out. Once the chain has
            // written (the animated anchor move is in flight), that slot must
            // not run — an early placement check reads mid-animation offsets
            // as "unsatisfied" and snaps the animation dead (review
            // #TASK-2698 finding). Post-animation slots (320ms+) remain the
            // placement checks.
            if chainHasWritten, index == 1 { return false }
            return sendAnchorRowId == id
        case .tailUpdate:
            return isFollowingTail
        case .repair:
            // Until the reader's first gesture, repairs chase late layout
            // settling across their whole retry window — single attempts
            // cannot catch up with a heavy transcript that keeps reflowing.
            if isFollowingTail, !hasUserScrolledSinceOpen {
                return true
            }
            return isFollowingTail && (metrics.isNearBottom || metrics.hasVisibleTailGap)
        }
    }

    private mutating func markTailGeometryChanged() {
        tailGeometryEpoch &+= 1
    }

    private static func tailLayoutGeometryChanged(
        from previous: GaryxConversationLayoutMetrics,
        to current: GaryxConversationLayoutMetrics
    ) -> Bool {
        let contentHeightChanged: Bool
        switch (previous.contentHeight, current.contentHeight) {
        case let (.some(previousHeight), .some(currentHeight)):
            contentHeightChanged =
                abs(currentHeight - previousHeight) > stableLayoutTolerance
        case (.none, .none):
            contentHeightChanged = false
        case (.some, .none), (.none, .some):
            contentHeightChanged = true
        }
        let viewportChanged =
            abs(current.viewportHeight - previous.viewportHeight) > stableLayoutTolerance
        return contentHeightChanged || viewportChanged
    }

    // MARK: History paging

    /// Automatic older-history loading: fires once the reader ever moved
    /// toward older history and the loaded start is within prefetch distance.
    /// Every page (or window reveal) grows the content above the reader, so
    /// the distance gate converges — loading stops as soon as the loaded
    /// start sits more than ~1.5 viewports above, and resumes when the reader
    /// scrolls near the top again. An untouched thread never pages backwards
    /// because `hasMovedTowardOlderHistory` requires a reader gesture.
    public func shouldPrefetchOlderHistory(
        hasMoreHistoryBefore: Bool,
        isLoadingOlderHistory: Bool,
        hasPendingPrefetch: Bool
    ) -> Bool {
        guard hasMoreHistoryBefore,
              !isLoadingOlderHistory,
              !hasPendingPrefetch,
              hasMovedTowardOlderHistory else {
            return false
        }
        return metrics.isNearLoadedHistoryStart
    }

    /// Reading-position restore after an older-history prepend.
    ///
    /// A plain SwiftUI scroll view keeps its offset relative to the content
    /// TOP when rows are inserted above (`defaultScrollAnchor(.bottom, for:
    /// .sizeChanges)` only pins a reader who is already at the bottom), so a
    /// prepend physically pushes the reading position out and parks the
    /// viewport over the just-loaded oldest rows.
    ///
    /// The view executes the restore with the anchor row's displacement in
    /// the transcript CONTENT coordinate space (`historyPrependTopGrowth`):
    /// content-space positions are scroll-invariant and only move when the
    /// layout itself changes, so the anchor row's displacement IS the exact
    /// height inserted above it — concurrent tail streaming below and
    /// concurrent reader scrolling both cancel out structurally. The shift is
    /// applied once on top of the CURRENT scroll offset, preserving whatever
    /// the reader did meanwhile.
    ///
    /// When row geometry is unavailable the restore degrades to scrolling
    /// the anchor row back to the viewport top (coarse: loses at most the
    /// prefetch distance, never parks the reader on the oldest rows).
    public struct ReadingAnchorRestore: Equatable {
        /// The row that was first before the prepend — the new content's
        /// lower boundary. `preservesScrollForPrependedHistory` guarantees it
        /// still exists in the new row set at index > 0.
        public let anchorRowId: String

        public init(anchorRowId: String) {
            self.anchorRowId = anchorRowId
        }
    }

    /// Exact height inserted above the anchor row by an older-history
    /// prepend: its content-space displacement between the pre-prepend
    /// capture and the post-prepend layout. Returns nil while geometry is
    /// missing or the layout has not grown yet (the caller retries on a
    /// later pass); a shrinking displacement is not a prepend.
    public static func historyPrependTopGrowth(
        capturedAnchorMinY: CGFloat?,
        currentAnchorMinY: CGFloat?
    ) -> CGFloat? {
        guard let capturedAnchorMinY, let currentAnchorMinY else { return nil }
        let growth = currentAnchorMinY - capturedAnchorMinY
        guard growth > 0.5 else { return nil }
        return growth
    }

    /// Visible render rows changed after a render snapshot update. This covers
    /// every visible prepend shape: the in-memory window reveal, and the
    /// network older page once the server row window lowers its floor.
    /// Returns a restore request the view must execute so the prepend does
    /// not move the reader (see `ReadingAnchorRestore`).
    public mutating func renderRowsChanged(
        previousIds: [String],
        currentIds: [String],
        previousScopeIdentity: String,
        currentScopeIdentity: String,
        hasTailContent: Bool
    ) -> ReadingAnchorRestore? {
        let threadUnchanged = previousScopeIdentity == currentScopeIdentity
        guard !threadUnchanged || previousIds != currentIds else { return nil }
        let isHistoryPrepend = Self.preservesScrollForPrependedHistory(
            previousIds: previousIds,
            currentIds: currentIds,
            threadUnchanged: threadUnchanged
        )
        guard isHistoryPrepend, let anchorRowId = previousIds.first else {
            // Render-only row changes can alter transcript height without a
            // corresponding message-body reducer event. They do not choose a
            // scroll action here, but they must qualify a still-attempting
            // token's next fire-time settlement check.
            markTailGeometryChanged()
            self.hasTailContent = hasTailContent
            return nil
        }
        // Keep the tail bookkeeping current; a history prepend never yields
        // a tail scroll (`contentChanged` returns nil for prepends).
        _ = contentChanged(
            isInitialLoad: false,
            isHistoryPrepend: true,
            hasTailContent: hasTailContent
        )
        return ReadingAnchorRestore(anchorRowId: anchorRowId)
    }

    /// Whether a messages change is an older-history prepend whose reading
    /// position must be preserved (the previous first message moved down).
    public static func preservesScrollForPrependedHistory(
        previousIds: [String],
        currentIds: [String],
        threadUnchanged: Bool
    ) -> Bool {
        guard threadUnchanged,
              currentIds.count > previousIds.count,
              let previousFirstId = previousIds.first,
              currentIds.first != previousFirstId,
              let previousFirstIndex = currentIds.firstIndex(of: previousFirstId) else {
            return false
        }
        return previousFirstIndex > 0
    }
}

/// Owns scroll retry-chain arbitration and target settlement.
///
/// The view owns only the delayed callbacks and the actual position write.
/// Every callback asks this state machine for authorization at fire time:
///
/// `requested -> attempting -> settled`
///                         `-> superseded`
///
/// A satisfied target with unchanged geometry settles the token permanently.
/// An unsatisfied target or geometry movement since the last authorized write
/// permits the next attempt. Scheduling a newer request preserves the existing
/// retry-horizon arbitration while explicitly superseding affected tokens.
public struct GaryxConversationScrollScheduler: Equatable {
    public enum Lifecycle: Equatable {
        case requested
        case attempting
        case settled
        case superseded
    }

    public struct Token: Equatable {
        fileprivate let retryHorizon: GaryxConversationScrollState.ScrollRetryHorizon
        fileprivate let generation: Int
    }

    private struct Chain: Equatable {
        let generation: Int
        var lifecycle: Lifecycle
        var lastAuthorizedGeometryEpoch: UInt64?
        /// Whether an attempt of this chain performed a REAL position write
        /// (target geometry was resolvable at execution time). Authorization
        /// alone must not count: a zero-delay attempt can be authorized
        /// before the appended row has laid out and then fail to position,
        /// in which case the next slot is still the chain's first true
        /// write and must carry the animation and haptic (#TASK-2698).
        var hasWritten = false
    }

    private var tailGrowthGeneration = 0
    private var settlingGeneration = 0
    private var tailGrowthChain: Chain?
    private var settlingChain: Chain?

    public init() {}

    public mutating func schedule(
        request: GaryxConversationScrollState.ScrollRequest
    ) -> Token {
        switch request.reason.retryHorizon {
        case .tailGrowth:
            // Coalesce ordinary streaming/tail-growth chains with each other,
            // but never let their short retry window truncate a still-live
            // opening/manual/repair chain whose late attempts are needed for
            // heavy transcript layout settling.
            tailGrowthGeneration &+= 1
            let token = Token(
                retryHorizon: .tailGrowth,
                generation: tailGrowthGeneration
            )
            tailGrowthChain = Chain(
                generation: token.generation,
                lifecycle: .requested,
                lastAuthorizedGeometryEpoch: nil
            )
            return token
        case .settling:
            // A fresh long-horizon chain covers every earlier attempt. Cancel
            // both lanes so stale short retries cannot outlive the new owner.
            settlingGeneration &+= 1
            tailGrowthGeneration &+= 1
            tailGrowthChain = nil
            let token = Token(
                retryHorizon: .settling,
                generation: settlingGeneration
            )
            settlingChain = Chain(
                generation: token.generation,
                lifecycle: .requested,
                lastAuthorizedGeometryEpoch: nil
            )
            return token
        }
    }

    public func isCurrent(_ token: Token) -> Bool {
        lifecycle(of: token) != .superseded
    }

    /// Whether this chain has performed a real position write yet. Fed back
    /// into `shouldRunScrollAttempt(chainHasWritten:)` and used to key the
    /// first-write animation and send haptic.
    public func hasWritten(_ token: Token) -> Bool {
        guard let chain = chain(for: token), chain.generation == token.generation else {
            return false
        }
        return chain.hasWritten
    }

    /// Record that an attempt of this chain performed a real position write.
    public mutating func markWrote(_ token: Token) {
        guard var chain = chain(for: token), chain.generation == token.generation else {
            return
        }
        chain.hasWritten = true
        setChain(chain, for: token.retryHorizon)
    }

    public func lifecycle(of token: Token) -> Lifecycle {
        guard let chain = chain(for: token), chain.generation == token.generation else {
            return .superseded
        }
        return chain.lifecycle
    }

    /// Authorize one queued attempt from pure fire-time inputs.
    ///
    /// The first policy-eligible attempt moves the request into `attempting`
    /// and is always authorized. Later attempts run only while the target is
    /// unsatisfied, geometry moved after the last write, or placement is not
    /// observable yet. Stable satisfied placement is terminal for the token.
    public mutating func authorizeAttempt(
        _ token: Token,
        input: GaryxConversationScrollAttemptInput
    ) -> Bool {
        guard var chain = chain(for: token),
              chain.generation == token.generation,
              chain.lifecycle != .settled,
              input.policyAllowsAttempt else {
            return false
        }

        if chain.lifecycle == .requested {
            chain.lifecycle = .attempting
            chain.lastAuthorizedGeometryEpoch = input.geometryEpoch
            setChain(chain, for: token.retryHorizon)
            return true
        }

        if input.targetPlacement == .satisfied,
           chain.lastAuthorizedGeometryEpoch == input.geometryEpoch {
            chain.lifecycle = .settled
            setChain(chain, for: token.retryHorizon)
            return false
        }

        chain.lastAuthorizedGeometryEpoch = input.geometryEpoch
        setChain(chain, for: token.retryHorizon)
        return true
    }

    private func chain(for token: Token) -> Chain? {
        switch token.retryHorizon {
        case .tailGrowth:
            tailGrowthChain
        case .settling:
            settlingChain
        }
    }

    private mutating func setChain(
        _ chain: Chain,
        for retryHorizon: GaryxConversationScrollState.ScrollRetryHorizon
    ) {
        switch retryHorizon {
        case .tailGrowth:
            tailGrowthChain = chain
        case .settling:
            settlingChain = chain
        }
    }
}
