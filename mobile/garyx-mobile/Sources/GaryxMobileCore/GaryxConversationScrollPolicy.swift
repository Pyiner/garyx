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
        return TailScrollRequest(reason: .tailUpdate, animated: true)
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
        } else {
            anchoring = .browsingHistory
            hasMovedTowardOlderHistory = true
        }
        if isFollowingTail, hasTailContent, metrics.hasVisibleTailGap {
            return TailScrollRequest(reason: .repair, animated: false)
        }
        return nil
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
    /// First attempts always run; later attempts of repair scrolls are
    /// dropped once the reader left the tail or the gap closed.
    public func shouldRunTailScrollAttempt(index: Int, reason: TailScrollReason) -> Bool {
        guard index > 0 else { return true }
        switch reason {
        case .openingThread, .tailUpdate, .manual:
            return true
        case .repair:
            return isFollowingTail && (metrics.isNearBottom || metrics.hasVisibleTailGap)
        }
    }

    // MARK: History paging

    public func shouldPrefetchOlderHistory(
        hasMoreHistoryBefore: Bool,
        isLoadingOlderHistory: Bool,
        hasPendingPrefetch: Bool,
        ignoreDistance: Bool
    ) -> Bool {
        guard hasMoreHistoryBefore,
              !isLoadingOlderHistory,
              !hasPendingPrefetch,
              hasMovedTowardOlderHistory else {
            return false
        }
        return ignoreDistance || metrics.isNearLoadedHistoryStart
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
