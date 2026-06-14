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

    /// Merge a fetched page into the cached window. Persists to disk and advances
    /// the mirror when the page is `committedOnly` — meaning it cannot contain an
    /// in-flight overlay row — or when the thread is idle. A `committedOnly` page is
    /// a forward page with `has_more_after` (the gateway withholds the overlay until
    /// the committed tail is drained) or any `before_index` (older) page. An overlay
    /// row carries a positional index, so this guard — not an index check — is what
    /// keeps transient rows out of the durable cache; during an active run the final
    /// overlay-bearing page is returned for display only and the cursor stays frozen.
    @discardableResult
    func updateTranscriptCache(
        threadId: String,
        fetched: GaryxThreadTranscript,
        direction: GaryxTranscriptCacheMergeDirection,
        committedOnly: Bool
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
        if committedOnly || idle {
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

    /// Fetch a thread's history incrementally: forward `after_index` delta pages
    /// when a cache cursor exists, paging until `has_more_after` is false so the
    /// committed tail is fully drained and the gateway's in-flight overlay is
    /// reached (otherwise a long active run with >1 page of committed rows would
    /// freeze the displayed tail). Falls back to the full recent-turns window when
    /// there is no cache. Returns a full-window transcript (cache ∪ delta).
    func fetchThreadTranscriptIncrementally(threadId: String) async throws -> GaryxThreadTranscript {
        guard transcriptAfterCursor(for: threadId) != nil else {
            return try await fullThreadTranscript(threadId: threadId)
        }
        var lastPage: GaryxThreadTranscript?
        var window: GaryxCachedTranscript?
        var previousCursor = -1
        for _ in 0..<Self.threadHistoryMaxForwardPages {
            guard let cursor = transcriptAfterCursor(for: threadId), cursor != previousCursor else {
                break
            }
            previousCursor = cursor
            let page = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                afterIndex: cursor
            )
            // Server shrank below our cursor (thread cleared / truncated / reset):
            // the cache is ahead of the server, so drop it and rebuild from a full
            // fetch instead of showing the stale window forever. Uses the
            // authoritative total (the cursor is the max cached index = total-1 when
            // in sync, so `cursor >= total` means the cache holds an index the
            // server no longer has). An empty page reports returned_end_index == 0,
            // so the total — not the page bounds — must drive this.
            if let total = page.pageInfo?.totalMessagesInThread, cursor >= total {
                clearTranscriptCache(for: threadId)
                return try await fullThreadTranscript(threadId: threadId)
            }
            let hasMoreAfter = page.pageInfo?.hasMoreAfter == true
            // A `has_more_after` page is pure committed (the overlay is withheld
            // until the committed tail drains), so persist + advance the cursor even
            // mid-run; the final page may carry the overlay and is persisted only
            // when idle.
            window = updateTranscriptCache(
                threadId: threadId,
                fetched: page,
                direction: .forward,
                committedOnly: hasMoreAfter
            )
            lastPage = page
            if !hasMoreAfter { break }
        }
        if let window, let lastPage {
            return transcriptForDisplay(window: window, fetched: lastPage)
        }
        return try await fullThreadTranscript(threadId: threadId)
    }

    private func fullThreadTranscript(threadId: String) async throws -> GaryxThreadTranscript {
        let full = try await client().threadHistory(
            threadId: threadId,
            limit: Self.threadHistoryPageLimit,
            userQueryLimit: Self.threadHistoryUserQueryLimit
        )
        let window = updateTranscriptCache(
            threadId: threadId,
            fetched: full,
            direction: .replaceLatest,
            committedOnly: false
        )
        return transcriptForDisplay(window: window, fetched: full)
    }

    func clearTranscriptCache(for threadId: String) {
        cachedTranscriptSnapshots[threadId] = nil
        transcriptCacheStore.remove(threadId: threadId)
    }
}
