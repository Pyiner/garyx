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

    func transcriptSnapshotAsync(for threadId: String) async -> GaryxCachedTranscript? {
        if let cached = cachedTranscriptSnapshots[threadId] {
            return cached
        }
        let store = transcriptCacheStore
        guard let loaded = await GaryxTranscriptCachePersistenceQueue.shared.load(
            threadId: threadId,
            store: store
        ) else {
            return nil
        }
        cachedTranscriptSnapshots[threadId] = loaded
        return loaded
    }

    /// Forward cursor for the next incremental open, or nil to do a full fetch.
    func transcriptAfterCursor(for threadId: String) -> Int? {
        transcriptSnapshot(for: threadId)?.afterCursor
    }

    func transcriptAfterCursorAsync(for threadId: String) async -> Int? {
        await transcriptSnapshotAsync(for: threadId)?.afterCursor
    }

    /// Rendered committed window for instant cold-start display before the network
    /// fetch returns. Empty when nothing is cached.
    func restoredCachedMessages(for threadId: String) -> [GaryxMobileMessage] {
        guard let snapshot = transcriptSnapshot(for: threadId) else {
            return []
        }
        if let renderSnapshot = snapshot.renderSnapshot {
            setRenderSnapshot(renderSnapshot, for: threadId)
        }
        guard !snapshot.messages.isEmpty else { return [] }
        return mobileMessages(from: snapshot.messages, live: false)
    }

    /// Merge a fetched page into the cached window. Persists to disk and advances
    /// the mirror when the page is `committedOnly` — meaning it cannot contain a
    /// transient live row — or when the thread is idle. A `committedOnly` page is
    /// a forward page with `has_more_after` (the committed tail is still being
    /// drained) or any `before_index` (older) page. A live row can carry a
    /// positional index, so this guard — not an index check — is what keeps
    /// transient rows out of the durable cache; during an active run the final
    /// live page is returned for display only and the cursor stays frozen.
    @discardableResult
    func updateTranscriptCache(
        threadId: String,
        fetched: GaryxThreadTranscript,
        direction: GaryxTranscriptCacheMergeDirection,
        committedOnly: Bool
    ) async -> GaryxCachedTranscript {
        let existing = await transcriptSnapshotAsync(for: threadId)
        let savedAt = Date()
        let prepared = await Task.detached(priority: .utility) {
            let window = GaryxTranscriptCacheLogic.merged(
                into: existing,
                threadId: threadId,
                fetched: fetched.messages,
                pageInfo: fetched.pageInfo,
                direction: direction,
                savedAt: savedAt
            )
            let fetchedRunState = GaryxTranscriptRunStateReducer.reduce(fetched.messages)
            return (window, !fetchedRunState.busy)
        }.value
        let window = prepared.0
        if committedOnly || prepared.1 {
            cachedTranscriptSnapshots[threadId] = window
            persistTranscriptCacheWindowInBackground(window)
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
    /// committed tail is fully drained and any live rows are reached (otherwise
    /// a long active run with >1 page of committed rows would
    /// freeze the displayed tail). Falls back to the full recent-turns window when
    /// there is no cache. Returns a full-window transcript (cache ∪ delta).
    func fetchThreadTranscriptIncrementally(threadId: String) async throws -> GaryxThreadTranscript {
        guard await transcriptAfterCursorAsync(for: threadId) != nil else {
            return try await fullThreadTranscript(threadId: threadId)
        }
        var lastPage: GaryxThreadTranscript?
        var window: GaryxCachedTranscript?
        var previousCursor = -1
        pageLoop: for _ in 0..<Self.threadHistoryMaxForwardPages {
            guard let cursor = await transcriptAfterCursorAsync(for: threadId), cursor != previousCursor else {
                break
            }
            previousCursor = cursor
            // Send the cursor AND the user-turn bound: the gateway returns the forward
            // delta when the cursor is within the newest threadHistoryUserQueryLimit
            // turns, or the bounded newest window with `reset` when it is older.
            let page = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                afterIndex: cursor,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            // Decide overwrite (reset) / shrink-refetch / forward-merge from the page
            // metadata (pure logic in GaryxTranscriptFetchPlanner, unit-tested).
            switch GaryxTranscriptFetchPlanner.pageAction(
                cursor: cursor,
                reset: page.pageInfo?.reset ?? false,
                hasMoreAfter: page.pageInfo?.hasMoreAfter ?? false,
                totalMessagesInThread: page.pageInfo?.totalMessagesInThread
            ) {
            case .reset:
                // Far behind: the server returned the bounded newest window; overwrite
                // the cache with it (older history pages in on scroll-up) rather than
                // merging the skipped gap.
                clearTranscriptCache(for: threadId)
                window = await updateTranscriptCache(
                    threadId: threadId,
                    fetched: page,
                    direction: .replaceLatest,
                    committedOnly: false
                )
                lastPage = page
                break pageLoop
            case .shrinkRefetch:
                // Cache is ahead of the server (thread cleared / truncated): drop it and
                // rebuild from a full fetch instead of showing the stale window forever.
                clearTranscriptCache(for: threadId)
                return try await fullThreadTranscript(threadId: threadId)
            case .mergeForward(let committedOnly, let continuePaging):
                // A committed-only (has_more_after) page withholds the overlay until the
                // committed tail drains, so it persists + advances the cursor even
                // mid-run; the final live page persists only when idle.
                window = await updateTranscriptCache(
                    threadId: threadId,
                    fetched: page,
                    direction: .forward,
                    committedOnly: committedOnly
                )
                lastPage = page
                if !continuePaging { break pageLoop }
            }
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
        let window = await updateTranscriptCache(
            threadId: threadId,
            fetched: full,
            direction: .replaceLatest,
            committedOnly: false
        )
        return transcriptForDisplay(window: window, fetched: full)
    }

    func clearTranscriptCache(for threadId: String) {
        cachedTranscriptSnapshots[threadId] = nil
        renderSnapshotsByThread[threadId] = nil
        selectedThreadRenderFloorByThread[threadId] = nil
        removeTranscriptCacheInBackground(threadId: threadId)
    }

    func persistTranscriptCacheWindowInBackground(_ window: GaryxCachedTranscript) {
        let store = transcriptCacheStore
        let generation = nextTranscriptCachePersistenceGeneration(for: window.threadId)
        Task.detached(priority: .utility) {
            await GaryxTranscriptCachePersistenceQueue.shared.save(
                window,
                generation: generation,
                store: store
            )
        }
    }

    func removeTranscriptCacheInBackground(threadId: String) {
        let store = transcriptCacheStore
        let generation = nextTranscriptCachePersistenceGeneration(for: threadId)
        Task.detached(priority: .utility) {
            await GaryxTranscriptCachePersistenceQueue.shared.remove(
                threadId: threadId,
                generation: generation,
                store: store
            )
        }
    }

    private func nextTranscriptCachePersistenceGeneration(for threadId: String) -> UInt64 {
        let next = (transcriptCachePersistenceGenerations[threadId] ?? 0) &+ 1
        transcriptCachePersistenceGenerations[threadId] = next
        return next
    }
}

private actor GaryxTranscriptCachePersistenceQueue {
    static let shared = GaryxTranscriptCachePersistenceQueue()

    private var latestGenerationByThread: [String: UInt64] = [:]

    func load(threadId: String, store: GaryxTranscriptCacheStore) -> GaryxCachedTranscript? {
        store.load(threadId: threadId)
    }

    func save(
        _ snapshot: GaryxCachedTranscript,
        generation: UInt64,
        store: GaryxTranscriptCacheStore
    ) {
        guard acceptGeneration(generation, threadId: snapshot.threadId) else { return }
        store.save(snapshot)
    }

    func remove(threadId: String, generation: UInt64, store: GaryxTranscriptCacheStore) {
        guard acceptGeneration(generation, threadId: threadId) else { return }
        store.remove(threadId: threadId)
    }

    private func acceptGeneration(_ generation: UInt64, threadId: String) -> Bool {
        let latest = latestGenerationByThread[threadId] ?? 0
        guard generation >= latest else {
            return false
        }
        latestGenerationByThread[threadId] = generation
        return true
    }
}
