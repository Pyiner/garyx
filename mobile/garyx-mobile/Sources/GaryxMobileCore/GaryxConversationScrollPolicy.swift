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

    /// A tiny cold-open transcript can place the loaded-start row on screen
    /// immediately. Automatic history prefetch only arms after the measured
    /// content has at least one viewport of scrollable overflow.
    public var isLargeEnoughForAutomaticHistoryPrefetch: Bool {
        guard let contentHeight, viewportHeight > 0 else { return false }
        let requiredOverflow = max(Self.historyPrefetchMinDistance, viewportHeight)
        return contentHeight - viewportHeight >= requiredOverflow
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
    /// Whether the reader ever scrolled toward older history in this thread.
    /// Gates history prefetch so an untouched thread never pages backwards.
    public private(set) var hasMovedTowardOlderHistory = false
    /// Whether the reader's finger or fling currently drives the scroll
    /// view. Programmatic tail scrolls must never fight an active gesture.
    public private(set) var isUserScrollInteracting = false
    /// Whether the reader has scrolled at all since the thread opened.
    /// Until they do, drifting away from the bottom can only be late layout
    /// settling (markdown measuring, async thumbnails), so the tail re-pins
    /// instead of stranding the viewport mid-history.
    public private(set) var hasUserScrolledSinceOpen = false
    /// Tracks the visible-tail-gap level so repairs fire on its rising edge
    /// only. A persistent gap (such as lazy-layout estimation drift around a
    /// collapsed tail row) must not regenerate a repair on every frame, or
    /// the reader can never scroll away from the tail.
    private var hadVisibleTailGap = false

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
    public mutating func threadOpened() -> TailScrollRequest {
        let viewportHeight = metrics.viewportHeight
        self = GaryxConversationScrollState()
        metrics.viewportHeight = viewportHeight
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
        self.metrics = metrics
        self.hasTailContent = hasTailContent
        guard metrics.viewportHeight > 0 else { return nil }
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
        return metrics.isLargeEnoughForAutomaticHistoryPrefetch
            && metrics.isNearLoadedHistoryStart
    }

    /// Visible render rows changed after a render snapshot update. This covers
    /// older-history expansion where cached messages were already prepended, but
    /// the server row window only lowered its floor on the next stream frame.
    public mutating func renderRowsChanged(
        previousIds: [String],
        currentIds: [String],
        threadUnchanged: Bool,
        hasTailContent: Bool
    ) -> TailScrollRequest? {
        let isHistoryPrepend = Self.preservesScrollForPrependedHistory(
            previousIds: previousIds,
            currentIds: currentIds,
            threadUnchanged: threadUnchanged
        )
        guard isHistoryPrepend else { return nil }
        return contentChanged(
            isInitialLoad: false,
            isHistoryPrepend: true,
            hasTailContent: hasTailContent
        )
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
