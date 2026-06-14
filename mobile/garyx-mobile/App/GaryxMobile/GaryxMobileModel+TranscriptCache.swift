import Foundation

// S2/S3 — persistent committed-transcript cache + incremental (`after_index`)
// opening. The cache holds only durable committed rows as a contiguous window;
// opening restores it for instant display, then fetches just the delta beyond the
// stored forward cursor. The merge/cursor logic lives in GaryxMobileCore
// (GaryxTranscriptCacheLogic, unit-tested); this layer is the side-effecting glue.
extension GaryxMobileModel {
    /// The persisted committed window for a thread (in-memory mirror first, then
    /// disk), or nil when nothing is cached yet.
    func transcriptSnapshot(for threadId: String) -> GaryxCachedTranscript? {
        if let cached = cachedTranscriptSnapshots[threadId] {
            return cached
        }
        guard let loaded = transcriptCacheStore.load(threadId: threadId) else {
            return nil
        }
        cachedTranscriptSnapshots[threadId] = loaded
        return loaded
    }

    /// Forward cursor for the next incremental open, or nil to do a full fetch.
    func transcriptAfterCursor(for threadId: String) -> Int? {
        transcriptSnapshot(for: threadId)?.afterCursor
    }

    /// Rendered committed window for instant cold-start display before the network
    /// fetch returns. Empty when nothing is cached.
    func restoredCachedMessages(for threadId: String) -> [GaryxMobileMessage] {
        guard let snapshot = transcriptSnapshot(for: threadId), !snapshot.messages.isEmpty else {
            return []
        }
        return mobileMessages(from: snapshot.messages, live: false)
    }

    /// Merge a fetched page (full / delta / older) into the cached window. Persists
    /// to disk and advances the mirror ONLY when the thread is idle, so an in-flight
    /// overlay row (which also carries a positional index) is never written to the
    /// durable cache — during an active run the returned window is used for display
    /// only and the stored cursor stays frozen at the last committed index.
    @discardableResult
    func updateTranscriptCache(
        threadId: String,
        fetched: GaryxThreadTranscript,
        direction: GaryxTranscriptCacheMergeDirection
    ) -> GaryxCachedTranscript {
        let window = GaryxTranscriptCacheLogic.merged(
            into: transcriptSnapshot(for: threadId),
            threadId: threadId,
            fetched: fetched.messages,
            pageInfo: fetched.pageInfo,
            direction: direction,
            savedAt: Date()
        )
        let idle = !remoteBusyThreadIds.contains(threadId)
            && fetched.threadRuntime?.activeRun == nil
        if idle {
            cachedTranscriptSnapshots[threadId] = window
            transcriptCacheStore.save(window)
        }
        return window
    }

    /// Wrap a merged window as a full-window transcript so the existing
    /// render/merge path (`applyThreadTranscriptToCache`) sees the same shape it
    /// gets from a full fetch, regardless of whether the network call was a delta.
    func transcriptForDisplay(
        window: GaryxCachedTranscript,
        fetched: GaryxThreadTranscript
    ) -> GaryxThreadTranscript {
        let pageInfo = GaryxThreadTranscriptPageInfo(
            returnedMessages: window.messages.count,
            returnedStartIndex: window.firstIndex,
            returnedEndIndex: window.afterCursor,
            hasMoreBefore: window.hasMoreBefore,
            nextBeforeIndex: window.nextBeforeIndex
        )
        return GaryxThreadTranscript(
            ok: fetched.ok,
            messages: window.messages,
            pendingUserInputs: fetched.pendingUserInputs,
            threadRuntime: fetched.threadRuntime,
            pageInfo: pageInfo
        )
    }

    /// Fetch a thread's history incrementally: a forward `after_index` delta when a
    /// cache cursor exists, otherwise the full recent-turns window. Returns a
    /// full-window transcript (cache ∪ delta) for the existing render path.
    func fetchThreadTranscriptIncrementally(threadId: String) async throws -> GaryxThreadTranscript {
        if let cursor = transcriptAfterCursor(for: threadId) {
            let delta = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                afterIndex: cursor
            )
            let window = updateTranscriptCache(threadId: threadId, fetched: delta, direction: .forward)
            return transcriptForDisplay(window: window, fetched: delta)
        }
        let full = try await client().threadHistory(
            threadId: threadId,
            limit: Self.threadHistoryPageLimit,
            userQueryLimit: Self.threadHistoryUserQueryLimit
        )
        let window = updateTranscriptCache(threadId: threadId, fetched: full, direction: .replaceLatest)
        return transcriptForDisplay(window: window, fetched: full)
    }

    func clearTranscriptCache(for threadId: String) {
        cachedTranscriptSnapshots[threadId] = nil
        transcriptCacheStore.remove(threadId: threadId)
    }
}
