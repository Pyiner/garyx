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
    /// applied on the current connection. Initial-window replay may legitimately
    /// start above the cursor; resume replay must stay contiguous with the body
    /// frontier already held by the client.
    public static func decide(
        incomingSeq: Int,
        connectionLastSeq: Int,
        allowsNonContiguousFirstSeq: Bool = true
    ) -> GaryxStreamSeqDecision {
        if incomingSeq > connectionLastSeq + 1 {
            if connectionLastSeq == 0, allowsNonContiguousFirstSeq {
                return .apply
            }
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

/// Whether the selected-thread recovery poll should keep running: only while
/// the recovered thread is still the open conversation and still remote-busy.
public enum GaryxSelectedThreadRecoveryPolicy {
    public static func shouldContinueRecovering(
        threadId: String,
        selectedThreadId: String?,
        remoteBusyThreadIds: Set<String>
    ) -> Bool {
        selectedThreadId == threadId
            && remoteBusyThreadIds.contains(threadId)
    }
}

public enum GatewayThreadStreamReplayScope: String, Equatable, Sendable {
    case resume
    case initial
}

public struct GatewayThreadStreamRequestState: Equatable, Sendable {
    public var afterSeq: Int
    public var replayScope: GatewayThreadStreamReplayScope?
    public var initialUserTurns: Int?
    public var renderFloor: Int?

    public init(
        afterSeq: Int,
        replayScope: GatewayThreadStreamReplayScope? = nil,
        initialUserTurns: Int? = nil,
        renderFloor: Int? = nil
    ) {
        self.afterSeq = max(afterSeq, 0)
        self.replayScope = replayScope
        self.initialUserTurns = initialUserTurns.map { max($0, 0) }
        self.renderFloor = renderFloor.map { max($0, 0) }
    }

    public static func resume(afterSeq: Int, renderFloor: Int? = nil) -> Self {
        Self(afterSeq: afterSeq, replayScope: .resume, renderFloor: renderFloor)
    }

    public static func initial(initialUserTurns: Int = 3) -> Self {
        Self(afterSeq: 0, replayScope: .initial, initialUserTurns: initialUserTurns)
    }

    public func resuming(afterSeq nextAfterSeq: Int) -> Self {
        Self(afterSeq: nextAfterSeq, replayScope: .resume, renderFloor: renderFloor)
    }
}

public enum GaryxThreadWindowPlanner {
    public static let initialUserTurns = 3

    public static func streamRequest(
        afterSeq: Int,
        renderFloor: Int?,
        hasWindowedRenderSnapshot: Bool
    ) -> GatewayThreadStreamRequestState {
        guard hasWindowedRenderSnapshot else {
            return .initial(initialUserTurns: initialUserTurns)
        }
        return .resume(afterSeq: afterSeq, renderFloor: renderFloor)
    }

    public static func floorSeq(from snapshot: GaryxRenderSnapshot?) -> Int? {
        snapshot?.window?.floorSeq
    }

    public static func floorSeqForOlderPage(firstIndex: Int?) -> Int? {
        firstIndex.map { max($0, 0) + 1 }
    }
}

public struct GaryxHistoryPaginationState: Equatable, Sendable {
    public static let empty = GaryxHistoryPaginationState(
        hasMoreBefore: false,
        nextBeforeIndex: nil
    )

    public var hasMoreBefore: Bool
    public var nextBeforeIndex: Int?

    public init(hasMoreBefore: Bool, nextBeforeIndex: Int?) {
        self.hasMoreBefore = hasMoreBefore
        self.nextBeforeIndex = nextBeforeIndex
    }
}

public struct GaryxHistoryPaginationPage: Equatable, Sendable {
    public var hasMoreBefore: Bool
    public var nextBeforeIndex: Int?
    public var oldestLoadedIndex: Int?
    public var latestPageStartIndex: Int?

    public init(
        hasMoreBefore: Bool,
        nextBeforeIndex: Int?,
        oldestLoadedIndex: Int?,
        latestPageStartIndex: Int?
    ) {
        self.hasMoreBefore = hasMoreBefore
        self.nextBeforeIndex = nextBeforeIndex
        self.oldestLoadedIndex = oldestLoadedIndex
        self.latestPageStartIndex = latestPageStartIndex
    }
}

public enum GaryxHistoryPaginationPlanner {
    public static func applyingRenderWindow(
        _ window: GaryxRenderWindow?,
        current: GaryxHistoryPaginationState,
        cached: GaryxHistoryPaginationState?
    ) -> GaryxHistoryPaginationState {
        guard let window else {
            return current
        }
        if window.hasMoreAbove, window.floorSeq > 1 {
            return GaryxHistoryPaginationState(
                hasMoreBefore: true,
                nextBeforeIndex: window.floorSeq - 1
            )
        }
        guard let cached else {
            return current
        }
        if cached.hasMoreBefore {
            return cached
        }
        return .empty
    }

    public static func applyingTranscriptPage(
        _ page: GaryxHistoryPaginationPage,
        current: GaryxHistoryPaginationState,
        preservingLoadedOlderPages: Bool
    ) -> GaryxHistoryPaginationState {
        if preservingLoadedOlderPages,
           let oldestLoadedIndex = page.oldestLoadedIndex,
           let latestPageStartIndex = page.latestPageStartIndex,
           oldestLoadedIndex < latestPageStartIndex {
            guard oldestLoadedIndex > 0 else {
                return .empty
            }
            return GaryxHistoryPaginationState(
                hasMoreBefore: true,
                nextBeforeIndex: oldestLoadedIndex
            )
        }

        if page.hasMoreBefore {
            return GaryxHistoryPaginationState(
                hasMoreBefore: true,
                nextBeforeIndex: page.nextBeforeIndex
            )
        }
        return .empty
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
