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

public enum GaryxStreamUpdateCadence {
    /// The mobile transcript stream batches committed SSE rows for three seconds.
    /// Bursty catch-up traffic should publish one consolidated UI state per window,
    /// not rebuild the SwiftUI message list for each event.
    public static let committedMessageBatchWindowNanos: UInt64 = 3_000_000_000
}

/// What to do with one streamed `committed_message` seq on the per-thread stream.
public enum GaryxStreamSeqDecision: Equatable, Sendable {
    /// A mid-stream seq hole (a dropped broadcast event): reconnect from the last
    /// contiguous seq so the file replay refills the gap.
    case gapReconnect(resumeAfterSeq: Int)
    /// Already applied on this connection (replay/live overlap, or out-of-order): skip.
    case stale
    /// Contiguous with what was applied, or a same-seq authoritative replacement:
    /// apply and advance or keep the connection cursor.
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
        if incomingSeq < connectionLastSeq {
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

public enum GaryxTranscriptControlRewriteAction: Equatable, Sendable {
    /// Ordinary content/control row: merge it through the normal committed path.
    case none
    /// The row declares that earlier transcript indexes changed. The mobile
    /// `after_index` cursor cannot replay those lower indexes, so the reader must
    /// discard its local window and fetch the authoritative transcript again.
    case refetchAuthoritativeTranscript
}

public enum GaryxTranscriptControlRewritePlanner {
    public static func action(for message: GaryxTranscriptMessage) -> GaryxTranscriptControlRewriteAction {
        switch controlKind(for: message) {
        case "range_rewrite", "transcript_reset":
            return .refetchAuthoritativeTranscript
        default:
            return .none
        }
    }

    public static func controlKind(for message: GaryxTranscriptMessage) -> String? {
        guard message.kind == "control" || message.role == .system else { return nil }
        if let kind = controlKind(in: message.message) {
            return kind
        }
        return controlKind(in: message.content)
    }

    private static func controlKind(in value: GaryxJSONValue?) -> String? {
        guard case let .object(object)? = value else { return nil }
        if case let .object(control)? = object["control"],
           case let .string(kind)? = control["kind"] {
            return normalizedControlKind(kind)
        }
        if case let .string(kind)? = object["kind"] {
            return normalizedControlKind(kind)
        }
        return nil
    }

    private static func normalizedControlKind(_ kind: String) -> String? {
        let normalized = kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized.isEmpty ? nil : normalized
    }
}
