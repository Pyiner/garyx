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
    public var viewportHeight: CGFloat

    public init(
        contentTopOffset: CGFloat? = nil,
        contentBottomOffset: CGFloat = 0,
        viewportHeight: CGFloat = 0
    ) {
        self.contentTopOffset = contentTopOffset
        self.contentBottomOffset = contentBottomOffset
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

/// One transcript content-edge measurement carrying BOTH edges.
///
/// The top sentinel and the bottom anchor each contribute their half; the
/// view layer reduces every contribution into a single value per layout pass
/// (one SwiftUI preference key), so the scroll state machine only ever
/// observes atomic frames. Feeding the edges through two separate callbacks
/// made every real scroll step look like a content-height change (top moved,
/// bottom not yet), which permanently reset the upward-travel accumulator
/// and broke the pre-iOS 18 reader-intent path (#TASK-2073 P2).
public struct GaryxConversationContentEdges: Equatable {
    public var top: CGFloat?
    public var bottom: CGFloat?

    public init(top: CGFloat? = nil, bottom: CGFloat? = nil) {
        self.top = top
        self.bottom = bottom
    }

    /// Combine two contributions; a later non-nil half wins its side.
    /// Merge order between the two emitters does not matter because each
    /// emitter only sets its own half.
    public func merging(_ other: Self) -> Self {
        Self(top: other.top ?? top, bottom: other.bottom ?? bottom)
    }
}

// MARK: - Tail thinking presentation

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
        isThinking: Bool,
        now: TimeInterval,
        delay: TimeInterval = Self.defaultDelay
    ) -> Bool {
        if !isThinking {
            thinkingStartedAt = nil
            isVisible = false
            return isVisible
        }

        if thinkingStartedAt == nil {
            thinkingStartedAt = now
        }
        if let thinkingStartedAt, now - thinkingStartedAt >= delay {
            isVisible = true
        }
        return isVisible
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
/// - `.browsingHistory`: the reader scrolled up; nothing moves the viewport,
///   and the scroll-to-bottom control is shown instead.
///
/// UI reads projections of this state (`showsScrollToBottomButton`,
/// `isFollowingTail`); the view feeds events in and executes the returned
/// `TailScrollRequest`s. Position math lives in
/// `GaryxConversationLayoutMetrics`.
public struct GaryxConversationScrollState: Equatable {
    public enum Anchoring: Equatable {
        case followingTail
        case browsingHistory
    }

    public enum TailScrollReason: Equatable {
        case openingThread
        case tailUpdate
        case manual
        case repair
    }

    public struct TailScrollRequest: Equatable {
        public let reason: TailScrollReason
        public let animated: Bool

        public init(reason: TailScrollReason, animated: Bool) {
            self.reason = reason
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
    /// The thread whose render-rows snapshot this state last observed.
    ///
    /// `renderRowsChanged` compares the incoming thread against this to tell a
    /// same-thread older-history prepend (preserve the reading position) from
    /// a thread switch that merely replays another thread's rows (never
    /// restore, even when row ids collide across threads — they are
    /// message-reference based, so `user_turn:history:0` recurs in every
    /// thread). It is established when the thread opens, so the very first
    /// prepend after a cold mount — whose cached rows predate the mount and
    /// therefore never fired a row-id change before it — is still recognized
    /// as same-thread (#TASK-2488).
    private var renderRowsThreadIdentity: String?

    public init() {}

    // MARK: UI projections

    public var isFollowingTail: Bool {
        anchoring == .followingTail
    }

    /// The glass down-arrow above the composer: visible whenever the reader
    /// left the tail and there is a tail to return to.
    public var showsScrollToBottomButton: Bool {
        anchoring == .browsingHistory && hasTailContent
    }

    // MARK: Events

    /// A thread was opened or switched: reset and jump straight to the tail.
    /// The measured viewport survives the reset — it belongs to the scroll
    /// surface, not the thread, and is not re-reported on switch.
    ///
    /// `threadIdentity` is the thread now shown here (nil for a draft). It
    /// anchors the render-rows preservation lifecycle at the open, which is the
    /// only lifecycle point a cold mount reaches before its first prepend.
    public mutating func threadOpened(threadIdentity: String?) -> TailScrollRequest {
        let viewportHeight = metrics.viewportHeight
        // Carry the last-observed render-rows thread across the reset. A cold
        // mount (nil) adopts the opening thread so its very first prepend is
        // read as same-thread; a switch keeps the OUTGOING thread so the first
        // post-switch row change is still rejected as cross-thread — regardless
        // of whether the open or the row change is delivered first (#TASK-2488).
        let carriedRenderRowsThreadIdentity = renderRowsThreadIdentity ?? threadIdentity
        self = GaryxConversationScrollState()
        metrics.viewportHeight = viewportHeight
        renderRowsThreadIdentity = carriedRenderRowsThreadIdentity
        return TailScrollRequest(reason: .openingThread, animated: false)
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
    ) -> TailScrollRequest? {
        self.hasTailContent = hasTailContent
        guard hasTailContent, !isHistoryPrepend else { return nil }
        if isInitialLoad {
            anchoring = .followingTail
            return TailScrollRequest(reason: .openingThread, animated: false)
        }
        guard isFollowingTail else { return nil }
        return TailScrollRequest(reason: .tailUpdate, animated: false)
    }

    /// The tail thinking indicator appeared (run started with no visible
    /// activity yet).
    public mutating func thinkingIndicatorShown() -> TailScrollRequest? {
        hasTailContent = true
        guard isFollowingTail else { return nil }
        return TailScrollRequest(reason: .tailUpdate, animated: false)
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
    ) -> TailScrollRequest? {
        let previousMetrics = self.metrics
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
        if metrics.isNearBottom {
            anchoring = .followingTail
        } else if isFollowingTail, !hasUserScrolledSinceOpen, !isUserScrollInteracting {
            // The tail drifted away before the reader ever scrolled: late
            // layout settling pushed the content down (heavy markdown,
            // async thumbnails). Stay anchored and pull the tail back —
            // the reader's first real gesture disables this for good.
            return TailScrollRequest(reason: .repair, animated: false)
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
            return TailScrollRequest(reason: .repair, animated: false)
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
    public mutating func userScrollInteractionChanged(isInteracting: Bool) -> TailScrollRequest? {
        guard isUserScrollInteracting != isInteracting else { return nil }
        isUserScrollInteracting = isInteracting
        if isInteracting {
            hasUserScrolledSinceOpen = true
        }
        guard !isInteracting,
              isFollowingTail,
              hasTailContent,
              metrics.hasVisibleTailGap else {
            return nil
        }
        return TailScrollRequest(reason: .repair, animated: false)
    }

    /// The composer gained focus. Keep the tail visible above the keyboard
    /// while following; never move a reader who is browsing history.
    public mutating func composerFocused() -> TailScrollRequest? {
        guard isFollowingTail, hasTailContent else { return nil }
        return TailScrollRequest(reason: .manual, animated: true)
    }

    /// The floating bottom chrome (composer tray) changed height.
    public mutating func bottomChromeChanged() -> TailScrollRequest? {
        guard isFollowingTail, hasTailContent else { return nil }
        return TailScrollRequest(reason: .repair, animated: false)
    }

    /// The reader tapped the scroll-to-bottom control: resume following.
    public mutating func scrollToBottomTapped() -> TailScrollRequest {
        anchoring = .followingTail
        return TailScrollRequest(reason: .manual, animated: false)
    }

    // MARK: Scheduled scroll retries

    /// Whether a delayed retry of a scheduled tail scroll should still run.
    ///
    /// Nothing but the reader's finger may move the viewport while a scroll
    /// gesture is active. After that, opening jumps and explicit manual
    /// scrolls always retry; tail updates and repairs are dropped as soon as
    /// the reader leaves the tail, so a streaming run can never pin a reader
    /// who is scrolling up toward history.
    public func shouldRunTailScrollAttempt(index: Int, reason: TailScrollReason) -> Bool {
        if isUserScrollInteracting, reason != .manual { return false }
        guard index > 0 else { return true }
        switch reason {
        case .openingThread, .manual:
            return true
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
        threadIdentity: String?,
        hasTailContent: Bool
    ) -> ReadingAnchorRestore? {
        // Same-thread when the incoming rows belong to the thread of the last
        // observed snapshot. Advance the anchor either way so the next change
        // compares against the thread now on screen.
        let threadUnchanged = renderRowsThreadIdentity == threadIdentity
        renderRowsThreadIdentity = threadIdentity
        let isHistoryPrepend = Self.preservesScrollForPrependedHistory(
            previousIds: previousIds,
            currentIds: currentIds,
            threadUnchanged: threadUnchanged
        )
        guard isHistoryPrepend, let anchorRowId = previousIds.first else { return nil }
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
