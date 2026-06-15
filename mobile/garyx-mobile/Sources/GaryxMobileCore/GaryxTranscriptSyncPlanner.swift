import Foundation

// Pure, UI-free decision logic for mobile transcript syncing ("message pulling").
// The app target reads cache/network state and performs the side effects; the
// decisions themselves live here so they are unit-testable without a simulator.

/// What to do with one `after_index` history page during an incremental open.
public enum GaryxTranscriptPageAction: Equatable, Sendable {
    /// The server returned the bounded newest window because the cursor was older than
    /// the newest user-turn window (`reset`): overwrite the cache with this page.
    case reset
    /// The cursor is at or beyond the server's total (the cache is ahead — the thread
    /// was cleared or truncated): drop the cache and do a fresh full fetch.
    case shrinkRefetch
    /// Merge this page forward into the cache. `committedOnly` marks a pure-committed
    /// page (safe to persist + advance the cursor mid-run); `continuePaging` is true
    /// while more committed rows remain after this page.
    case mergeForward(committedOnly: Bool, continuePaging: Bool)
}

public enum GaryxTranscriptFetchPlanner {
    /// Decide what to do with one `after_index` page, given the cache cursor and the
    /// page's metadata. `reset` takes precedence (server bounded the catch-up), then a
    /// shrink (cache ahead of the server), otherwise a forward merge.
    public static func pageAction(
        cursor: Int,
        reset: Bool,
        hasMoreAfter: Bool,
        totalMessagesInThread: Int?
    ) -> GaryxTranscriptPageAction {
        if reset {
            return .reset
        }
        if let total = totalMessagesInThread, cursor >= total {
            return .shrinkRefetch
        }
        return .mergeForward(committedOnly: hasMoreAfter, continuePaging: hasMoreAfter)
    }
}

/// What to do with one streamed `committed_message` seq on the per-thread stream.
public enum GaryxStreamSeqDecision: Equatable, Sendable {
    /// A mid-stream seq hole (a dropped broadcast event): reconnect from the last
    /// contiguous seq so the file replay refills the gap.
    case gapReconnect(resumeAfterSeq: Int)
    /// Already applied on this connection (replay/live overlap, or out-of-order): skip.
    case stale
    /// Contiguous with what was applied: apply and advance the connection cursor.
    case apply
}

public enum GaryxStreamSeqPlanner {
    /// Decide how to handle an incoming committed seq relative to the highest seq
    /// applied on the current connection (`0` = none yet). The first row of a
    /// connection always applies — the replay may legitimately start above the cursor
    /// when the far-behind window was reset.
    public static func decide(incomingSeq: Int, connectionLastSeq: Int) -> GaryxStreamSeqDecision {
        if connectionLastSeq > 0, incomingSeq > connectionLastSeq + 1 {
            return .gapReconnect(resumeAfterSeq: connectionLastSeq)
        }
        if incomingSeq <= connectionLastSeq {
            return .stale
        }
        return .apply
    }

    /// Resume cursor (as a seq) for (re)connecting the per-thread stream: one past the
    /// highest committed index already held (cache window or rendered history), or 0
    /// when nothing is cached.
    public static func resumeCursor(afterCursor: Int?, fallbackMaxIndex: Int?) -> Int {
        if let afterCursor {
            return afterCursor + 1
        }
        if let fallbackMaxIndex {
            return fallbackMaxIndex + 1
        }
        return 0
    }
}
