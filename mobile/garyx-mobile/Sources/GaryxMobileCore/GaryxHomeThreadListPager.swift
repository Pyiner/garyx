import Foundation

// Home recent-thread list paging state machine (TASK-1802, see
// docs/design/task-1802-ios-home-list-refresh-loadmore.md).
//
// Pure, synchronous, Equatable state + decision functions. No IO, no
// Combine, no timers: GaryxMobileModel asks for decisions (tickets) and
// reports results. Head refresh and load-more are two independent
// in-flight tracks that self-coalesce and never block each other; the
// final (gate, loadMoreFailureRevision, cursor, hasMore) state is
// independent of the completion order of concurrent tracks.

/// Why a refresh was requested. Owns failure presentation only —
/// truncation is nobody's (refresh never truncates) and the skeleton is
/// derived from list emptiness.
public enum GaryxThreadListRefreshSource: Equatable, Sendable {
    /// The user dragged the list (pull-to-refresh).
    case userPullToRefresh
    /// An explicit user/runtime action wants a fresh list: run
    /// completion, archive, interrupt, delete, foreground sync,
    /// open-thread fallback, connect.
    case userAction
    /// Timer-driven refreshes: the 10s visible loop and the 15s
    /// background committed-run reconcile loop.
    case backgroundLoop
}

/// How a failed refresh is surfaced to the user.
public enum GaryxThreadListRefreshFailurePresentation: Equatable, Sendable {
    /// Global error toast (`lastError`).
    case toast
    /// Low-key status line ("Waiting to sync with gateway").
    case transientStatus
}

public enum GaryxThreadListRefreshPolicy {
    /// A user-initiated refresh failure toasts (the user asked and
    /// deserves the answer); a timer-initiated failure never toasts —
    /// offline means one status line, not a toast per 10s cycle. A
    /// non-transient error surfaces on the user's next explicit action.
    public static func failurePresentation(
        source: GaryxThreadListRefreshSource
    ) -> GaryxThreadListRefreshFailurePresentation {
        switch source {
        case .userPullToRefresh, .userAction:
            return .toast
        case .backgroundLoop:
            return .transientStatus
        }
    }

    /// The loading skeleton is a first-load presentation concern derived
    /// from list content, independent of who started the refresh.
    public static func showsSkeleton(listIsEmpty: Bool) -> Bool {
        listIsEmpty
    }
}

/// What made a load-more request fire. Both triggers go through the same
/// pager gate, so duplicate fires are free.
public enum GaryxThreadListLoadMoreTrigger: Equatable, Sendable {
    /// The prefetch sentinel row (K rows from the tail) appeared.
    case nearTail
    /// The footer row itself appeared (short lists / fast flings).
    case footer
}

/// Issued by `requestRefresh`. Records the failure revision and local
/// mutation sequence observed at issue time: a successful refresh may only
/// forgive failures it already knew about, and may only commit if no local
/// list surgery happened while it was in flight (see `completeRefresh`).
public struct GaryxThreadListRefreshTicket: Equatable, Sendable {
    public let epoch: Int
    public let observedLoadMoreFailureRevision: Int
    public let observedLocalMutationSequence: Int
}

/// Issued by `requestLoadMore`/`retryLoadMore`; carries the request the
/// caller must perform.
public struct GaryxThreadListLoadMoreTicket: Equatable, Sendable {
    public let epoch: Int
    /// Same staleness rule as refresh tickets: a page fetched before local
    /// list surgery must not commit — dedup-append against the
    /// post-surgery list would resurrect the removed row as a "new" id.
    public let observedLocalMutationSequence: Int
    /// Overlapped offset: `max(0, nextOffset - overlap)`. The re-fetched
    /// seen rows dedup away; removal drift up to `overlap` rows is
    /// absorbed (mitigation, not a full fix — see design §4).
    public let offset: Int
    public let limit: Int
}

/// How a completed head refresh should be applied to `recentThreadIds`.
public enum GaryxThreadListRefreshApplication: Equatable, Sendable {
    /// Nothing loaded beyond this page: the head page owns the list.
    case replaceHead
    /// Pages loaded beyond the head: merge (new head wins, loaded tail
    /// keeps its order) and leave pagination untouched.
    case mergeBeyondHead
}

/// Outcome of `completeRefresh`.
public enum GaryxThreadListRefreshCompletion: Equatable, Sendable {
    /// The page is current: commit it with the given application.
    case apply(GaryxThreadListRefreshApplication)
    /// The ticket pre-dates a `reset()` (gateway switch): drop the page
    /// silently — it belongs to the previous gateway.
    case abandonedStaleEpoch
    /// Local list surgery (archive/delete/pin) happened while this refresh
    /// was in flight: its pre-await snapshots are stale even though the
    /// gateway call succeeded. Drop the page and run a follow-up refresh
    /// to pick up fresh pages.
    case abandonedLocalMutation
}

/// Outcome of `completeLoadMore` — same staleness taxonomy as refresh.
public enum GaryxThreadListLoadMoreCompletion: Equatable, Sendable {
    /// The page is current: append it (cursor and gate were advanced).
    case apply
    /// The ticket pre-dates a `reset()`: drop the page silently.
    case abandonedStaleEpoch
    /// Local list surgery raced this page: dedup-append against the
    /// post-surgery list would resurrect removed rows as "new" ids. The
    /// cursor and gate are untouched; the caller re-requests the page.
    case abandonedLocalMutation
}

public struct GaryxHomeThreadListPager: Equatable, Sendable {
    public enum LoadMoreGate: Equatable, Sendable {
        case ready
        case exhausted   // server said has_more == false
        case failed      // last load-more failed
        // No attempt counter: nothing consumes it (the footer shows the
        // same retry affordance regardless, and there is deliberately no
        // automatic backoff). Failure freshness lives in
        // loadMoreFailureRevision, which is order-commutative; a counter
        // derived from "current gate + 1" is not.
    }

    public private(set) var epoch: Int = 0
    public private(set) var isRefreshingHead = false
    public private(set) var isLoadingMore = false
    public private(set) var gate: LoadMoreGate = .ready
    /// Bumped by every `failLoadMore`; refresh tickets record the value
    /// they observed so forgiveness is revision-scoped.
    public private(set) var loadMoreFailureRevision: Int = 0
    /// Bumped by `noteLocalMutation()` whenever the model performs local
    /// list surgery (archive tombstone flips, local removals, pin edits).
    /// A refresh whose ticket observed an older value must not commit its
    /// pre-await snapshots — regardless of whether the surgery's own
    /// tombstone still exists at commit time.
    public private(set) var localMutationSequence: Int = 0
    /// Server cursor: end of the last adopted page. 0 = not primed (no
    /// head page yet), so load-more is meaningless.
    public private(set) var nextOffset: Int = 0
    public let pageLimit: Int
    public let overlap: Int

    public init(pageLimit: Int, overlap: Int) {
        self.pageLimit = pageLimit
        self.overlap = overlap
    }

    // MARK: Decisions

    /// Marks local list surgery (archive/delete/pin edits). In-flight
    /// refresh tickets become stale: their completion returns
    /// `.abandonedLocalMutation` instead of applying pre-surgery snapshots.
    public mutating func noteLocalMutation() {
        localMutationSequence &+= 1
    }

    /// nil → a refresh is already in flight (concurrent entry points
    /// coalesce). A load-more in flight does not block refresh.
    public mutating func requestRefresh() -> GaryxThreadListRefreshTicket? {
        guard !isRefreshingHead else { return nil }
        isRefreshingHead = true
        return GaryxThreadListRefreshTicket(
            epoch: epoch,
            observedLoadMoreFailureRevision: loadMoreFailureRevision,
            observedLocalMutationSequence: localMutationSequence
        )
    }

    /// nil → not primed / already loading more / exhausted / failed
    /// (automatic triggers are rejected after a failure; only
    /// `retryLoadMore` or a refresh issued after the failure re-arms).
    /// A refresh in flight does not block load-more.
    public mutating func requestLoadMore(
        trigger _: GaryxThreadListLoadMoreTrigger
    ) -> GaryxThreadListLoadMoreTicket? {
        guard !isLoadingMore, nextOffset > 0, gate == .ready else { return nil }
        return issueLoadMoreTicket()
    }

    /// The explicit footer tap; bypasses only the `.failed` gate.
    public mutating func retryLoadMore() -> GaryxThreadListLoadMoreTicket? {
        guard !isLoadingMore, nextOffset > 0, gate == .failed else { return nil }
        return issueLoadMoreTicket()
    }

    private mutating func issueLoadMoreTicket() -> GaryxThreadListLoadMoreTicket {
        isLoadingMore = true
        return GaryxThreadListLoadMoreTicket(
            epoch: epoch,
            observedLocalMutationSequence: localMutationSequence,
            offset: max(0, nextOffset - overlap),
            limit: pageLimit
        )
    }

    // MARK: Results

    /// Resolves a finished refresh into apply-or-abandon (see
    /// `GaryxThreadListRefreshCompletion`). Both abandon cases leave gate,
    /// cursor, and revisions untouched; `.abandonedLocalMutation` expects
    /// the caller to run a follow-up refresh with fresh pages.
    @discardableResult
    public mutating func completeRefresh(
        _ ticket: GaryxThreadListRefreshTicket,
        pageOffset: Int,
        pageCount: Int,
        hasMore: Bool
    ) -> GaryxThreadListRefreshCompletion {
        guard ticket.epoch == epoch else { return .abandonedStaleEpoch }
        isRefreshingHead = false
        guard ticket.observedLocalMutationSequence == localMutationSequence else {
            return .abandonedLocalMutation
        }

        let returnedEnd = pageOffset + pageCount
        let isBeyondHead = nextOffset > returnedEnd

        // Failure forgiveness is revision-scoped: a refresh proves the
        // gateway reachable *after* the failures it observed at issue
        // time. A failure produced by a load-more that was still in
        // flight beside this refresh has a newer revision and survives —
        // that keeps the final gate independent of completion order.
        let forgivesFailure = gate == .failed
            && ticket.observedLoadMoreFailureRevision == loadMoreFailureRevision

        if gate == .failed && !forgivesFailure {
            // Hard-keep .failed: hasMore from a refresh issued before the
            // failure must not overwrite it (in either completion order).
        } else if isBeyondHead {
            if forgivesFailure {
                gate = .ready
            }
            // .ready / .exhausted beyond head: pagination untouched.
        } else {
            gate = hasMore ? .ready : .exhausted
        }

        if !isBeyondHead {
            nextOffset = returnedEnd
        }
        return .apply(isBeyondHead ? .mergeBeyondHead : .replaceHead)
    }

    /// Commits a seq range-fill chain. A replacement owns a fresh window and
    /// advances the feed epoch; a normal fill preserves the loaded tail cursor.
    /// The wrapper enforces the design's single lane, so no load-more ticket can
    /// coexist with this epoch transition.
    @discardableResult
    public mutating func completeRangeRefresh(
        _ ticket: GaryxThreadListRefreshTicket,
        committedCount: Int,
        hasMore: Bool,
        replacementCommitted: Bool
    ) -> GaryxThreadListRefreshCompletion {
        guard ticket.epoch == epoch else { return .abandonedStaleEpoch }
        isRefreshingHead = false
        guard ticket.observedLocalMutationSequence == localMutationSequence else {
            return .abandonedLocalMutation
        }

        let forgivesFailure = gate == .failed
            && ticket.observedLoadMoreFailureRevision == loadMoreFailureRevision
        if replacementCommitted {
            epoch += 1
            nextOffset = committedCount
            gate = hasMore ? .ready : .exhausted
            return .apply(.replaceHead)
        }
        if forgivesFailure {
            gate = nextOffset > 0 ? .ready : .exhausted
        }
        return .apply(.mergeBeyondHead)
    }

    public mutating func failRefresh(_ ticket: GaryxThreadListRefreshTicket) {
        guard ticket.epoch == epoch else { return }
        isRefreshingHead = false
        // Gate and cursor untouched: a refresh failure says nothing new
        // about load-more; presentation is the caller's policy concern.
    }

    /// Resolves a finished load-more into apply-or-abandon. Both abandon
    /// cases leave the cursor and gate untouched; `.abandonedLocalMutation`
    /// expects the caller to re-request the page.
    @discardableResult
    public mutating func completeLoadMore(
        _ ticket: GaryxThreadListLoadMoreTicket,
        pageOffset: Int,
        pageCount: Int,
        hasMore: Bool
    ) -> GaryxThreadListLoadMoreCompletion {
        guard ticket.epoch == epoch else { return .abandonedStaleEpoch }
        isLoadingMore = false
        guard ticket.observedLocalMutationSequence == localMutationSequence else {
            return .abandonedLocalMutation
        }
        nextOffset = pageOffset + pageCount
        gate = hasMore ? .ready : .exhausted
        return .apply
    }

    public mutating func failLoadMore(_ ticket: GaryxThreadListLoadMoreTicket) {
        guard ticket.epoch == epoch else { return }
        isLoadingMore = false
        gate = .failed
        loadMoreFailureRevision += 1
    }

    /// Gateway switch: everything back to initial; in-flight tickets from
    /// the previous epoch become structural no-ops.
    public mutating func reset() {
        epoch += 1
        isRefreshingHead = false
        isLoadingMore = false
        gate = .ready
        loadMoreFailureRevision = 0
        localMutationSequence = 0
        nextOffset = 0
    }

    // MARK: Derived

    public var footerState: GaryxHomeLoadMoreFooterState {
        if isLoadingMore { return .loading }
        switch gate {
        case .failed:
            return .failed
        case .exhausted:
            return .hidden
        case .ready:
            return nextOffset > 0 ? .idle : .hidden
        }
    }

    /// Legacy observation-store bridge: whether more rows are believed to
    /// exist below the loaded window.
    public var hasMoreThreadSummaries: Bool {
        nextOffset > 0 && gate != .exhausted
    }
}

/// What the auto-load footer renders. Fixed 44pt row for
/// idle/loading/failed — state flips change content, never geometry;
/// `hidden` removes the row.
public enum GaryxHomeLoadMoreFooterState: Equatable, Sendable {
    case hidden    // exhausted, or nothing loaded yet
    case idle      // more available; sentinel row, visually empty
    case loading   // spinner + "Loading more"
    case failed    // "Couldn't load more · Tap to retry"
}

/// Pure id-merge rules for the recent list (moved out of
/// GaryxMobileModel so they are testable).
public enum GaryxThreadListPageMerge {
    public static let defaultPrefetchDistance = 6

    /// Head refresh applied beyond loaded pages: the new first page wins
    /// the head, the previously loaded tail keeps its order minus ids
    /// that moved into the head.
    public static func mergeHead(pageIds: [String], existingIds: [String]) -> [String] {
        let pageIdSet = Set(pageIds)
        return pageIds + existingIds.filter { !pageIdSet.contains($0) }
    }

    /// Load-more: append page ids not already present (absorbs the
    /// overlap window and head-insert drift duplicates).
    public static func appendPage(pageIds: [String], existingIds: [String]) -> [String] {
        var seen = Set(existingIds)
        return existingIds + pageIds.filter { seen.insert($0).inserted }
    }

    /// Prefetch sentinel: the row whose appearance should trigger a
    /// near-tail load, `prefetchDistance` rows from the end; nil for
    /// lists shorter than the distance (the footer trigger covers them).
    public static func prefetchTriggerRowId(
        recentIds: [String],
        prefetchDistance: Int = defaultPrefetchDistance
    ) -> String? {
        guard prefetchDistance > 0, recentIds.count >= prefetchDistance else { return nil }
        return recentIds[recentIds.count - prefetchDistance]
    }
}
