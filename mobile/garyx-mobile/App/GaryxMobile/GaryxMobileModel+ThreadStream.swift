import Foundation

// S5 — resumable per-thread transcript stream for the open thread. Connects
// `/api/threads/{id}/stream?after_seq=<cursor>`: each data frame carries committed
// transcript events plus a server-rendered snapshot. Events are the catch-up/sync
// channel (cache, after_seq, run-state); render_state owns visible transcript rows.
//
// Self-heal: the broadcast bus is best-effort, so a slow consumer can miss events
// (tokio broadcast Lagged). committed_message carries a gapless seq; if a live row
// arrives non-contiguously we reconnect from the last contiguous seq and the file
// replay refills the hole. A dropped connection is recovered by the reconnect loop
// (URLSession surfaces it as an error / request timeout), and persistent failure
// falls back to after_index history plus the selected-thread reconcile loop.
extension GaryxMobileModel {
    func applySelectedThreadStreamPolicy(previousThreadId: String?, selectedThreadId: String?) {
        switch GaryxSelectedThreadStreamPolicy.action(
            previousThreadId: previousThreadId,
            selectedThreadId: selectedThreadId
        ) {
        case .none:
            break
        case .start(let threadId):
            startSelectedThreadStream(for: threadId)
        case .stop:
            cancelSelectedThreadReconcileLoop()
            stopSelectedThreadStream()
        }
    }

    /// Cursor the per-thread stream resumes from: the highest committed index we
    /// already hold (cache window or rendered history), as a seq (index + 1).
    func selectedThreadStreamCursor(for threadId: String) async -> Int {
        let snapshot = await transcriptSnapshotAsync(for: threadId)
        let hasRenderSnapshot = renderSnapshotsByThread[threadId] != nil || snapshot?.renderSnapshot != nil
        guard hasRenderSnapshot else {
            return 0
        }
        return GaryxStreamSeqPlanner.resumeCursor(
            afterCursor: snapshot?.afterCursor,
            fallbackMaxIndex: cachedMessages(for: threadId).compactMap(\.historyIndex).max()
        )
    }

    func startSelectedThreadStream(for threadId: String) {
        guard hasGatewaySettings, case .ready = connectionState else { return }
        let trimmed = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if streamOwnedThreadId == trimmed, selectedThreadStreamTask != nil {
            return
        }
        stopSelectedThreadStream()
        // The resumable stream supersedes the 1.5s reconcile poll for this thread.
        cancelSelectedThreadReconcileLoop()
        // Take ownership immediately; the stream's replay backfills anything that
        // raced the selected-thread handoff.
        streamOwnedThreadId = trimmed
        let generation = UUID()
        selectedThreadStreamGeneration = generation
        selectedThreadStreamTask = Task { [weak self] in
            await self?.runSelectedThreadStream(threadId: trimmed, generation: generation)
        }
    }

    func ensureSelectedThreadStreamForVisibleConversation() {
        let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard GaryxVisibleConversationStreamPolicy.shouldStart(
            isConversationVisible: navigationState.presentsContent,
            selectedThreadId: threadId,
            streamOwnedThreadId: streamOwnedThreadId,
            hasStreamTask: selectedThreadStreamTask != nil
        ),
              !threadId.isEmpty else { return }
        startSelectedThreadStream(for: threadId)
    }

    func stopSelectedThreadStream() {
        selectedThreadStreamTask?.cancel()
        selectedThreadStreamTask = nil
        selectedThreadStreamGeneration = nil
        streamOwnedThreadId = nil
        cancelSelectedThreadStreamFlushWindow()
        selectedThreadStreamDrainTask?.cancel()
        selectedThreadStreamDrainTask = nil
    }

    func stopSelectedThreadStreamForHome() {
        let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        selectedThreadStreamTask?.cancel()
        selectedThreadStreamTask = nil
        selectedThreadStreamGeneration = nil
        streamOwnedThreadId = nil
        guard !threadId.isEmpty else {
            cancelSelectedThreadStreamFlushWindow()
            selectedThreadStreamDrainTask?.cancel()
            selectedThreadStreamDrainTask = nil
            return
        }

        cancelSelectedThreadStreamFlushWindow()
        selectedThreadStreamDrainTask?.cancel()
        selectedThreadStreamDrainTask = Task { [weak self] in
            await self?.drainSelectedThreadStreamWindowForHome(threadId: threadId)
        }
    }

    private func drainSelectedThreadStreamWindowForHome(threadId: String) async {
        await flushSelectedThreadStreamWindow(for: threadId, respectingTaskCancellation: true)
        guard !Task.isCancelled, selectedThread?.id == threadId else { return }
        selectedThreadStreamDrainTask = nil
    }

    /// Fall back to the after_index + reconcile poll path when the per-thread stream
    /// cannot be sustained, so we still converge from committed transcript history.
    private func fallBackFromSelectedThreadStream(threadId: String) async {
        guard selectedThread?.id == threadId else { return }
        streamOwnedThreadId = nil
        selectedThreadStreamTask = nil
        selectedThreadStreamGeneration = nil
        await loadSelectedThreadHistory()
        startSelectedThreadReconcileLoop()
    }

    func runSelectedThreadStream(threadId: String, generation: UUID) async {
        let configuration: GaryxGatewayConfiguration
        do {
            configuration = try client().configuration
        } catch {
            await fallBackFromSelectedThreadStream(threadId: threadId)
            return
        }
        let actor = GatewayStreamActor(endpoint: GatewayStreamEndpoint(configuration: configuration))
        await actor.run(
            threadId: threadId,
            requestProvider: { [weak self] in
                await self?.selectedThreadStreamRequestForActor(threadId: threadId, generation: generation)
                    ?? .resume(afterSeq: 0)
            },
            shouldContinue: { [weak self] in
                await self?.isSelectedThreadStreamCurrent(threadId: threadId, generation: generation) ?? false
            },
            actionHandler: { [weak self] action in
                await self?.applySelectedThreadStreamAction(action, threadId: threadId, generation: generation) ?? .none
            }
        )
    }

    private func selectedThreadStreamRequestForActor(
        threadId: String,
        generation: UUID
    ) async -> GatewayThreadStreamRequestState {
        guard isSelectedThreadStreamCurrent(threadId: threadId, generation: generation) else {
            return .resume(afterSeq: 0)
        }
        let snapshot = await transcriptSnapshotAsync(for: threadId)
        let renderSnapshot = renderSnapshotsByThread[threadId] ?? snapshot?.renderSnapshot
        let renderFloor = selectedThreadRenderFloorByThread[threadId]
            ?? GaryxThreadWindowPlanner.floorSeq(from: renderSnapshot)
        let afterSeq = GaryxStreamSeqPlanner.resumeCursor(
            afterCursor: snapshot?.afterCursor,
            fallbackMaxIndex: cachedMessages(for: threadId).compactMap(\.historyIndex).max()
        )
        return GaryxThreadWindowPlanner.streamRequest(
            afterSeq: afterSeq,
            renderFloor: renderFloor,
            hasWindowedRenderSnapshot: renderSnapshot?.window != nil
        )
    }

    private func isSelectedThreadStreamCurrent(threadId: String, generation: UUID) -> Bool {
        selectedThreadStreamGeneration == generation
            && selectedThread?.id == threadId
            && hasGatewaySettings
    }

    private func applySelectedThreadStreamAction(
        _ action: GatewayStreamAction,
        threadId: String,
        generation: UUID
    ) async -> GatewayStreamActionResult {
        guard isSelectedThreadStreamCurrent(threadId: threadId, generation: generation) else {
            return .none
        }
        switch action {
        case .applyCommittedMessages(let messages):
            await applyStreamedCommittedMessages(messages, threadId: threadId)
            return .none
        case .resetCommittedCacheBelow(let floorSeq):
            dropCommittedCacheBelow(floorSeq: floorSeq, threadId: threadId)
            return .none
        case .applyRenderSnapshot(let snapshot):
            applyThreadRenderSnapshot(snapshot, threadId: threadId)
            return .none
        case .refetchAfterControlRewrite:
            let cursor = await refetchSelectedThreadAfterTranscriptRewrite(threadId: threadId)
            return .resumeCursor(cursor)
        case .fallback:
            await fallBackFromSelectedThreadStream(threadId: threadId)
            return .none
        }
    }

    private func refetchSelectedThreadAfterTranscriptRewrite(threadId: String) async -> Int {
        cancelSelectedThreadStreamFlushWindow()
        clearTranscriptCache(for: threadId)
        resetSelectedThreadHistoryPagination()
        clearMessages(for: threadId)
        await loadSelectedThreadHistory()
        return await selectedThreadStreamCursor(for: threadId)
    }

    /// Windowed-resume reset (server degraded a stale resume): cached
    /// committed rows below the window floor are no longer contiguous with
    /// this connection. Drop them before the window's rows merge so the
    /// cache keeps its contiguous invariant (parity with the desktop
    /// mirror's dropCommittedBelow).
    private func dropCommittedCacheBelow(floorSeq: Int, threadId: String) {
        guard selectedThread?.id == threadId else { return }
        if let window = GaryxTranscriptCacheLogic.droppingCommittedBelow(
            floorSeq: floorSeq,
            in: transcriptSnapshot(for: threadId)
        ) {
            cachedTranscriptSnapshots[threadId] = window
        }
        // On-screen rows below the floor must go too: the prepared-flush
        // preserve step (preserveRemoteBeforeIndex = window.firstIndex)
        // would otherwise re-attach the stale prefix in front of the
        // window (#TASK-1701 re-review). Optimistic rows (no historyIndex)
        // are kept.
        let pruned = GaryxTranscriptCacheLogic.droppingLocalRowsBelow(
            floorSeq: floorSeq,
            in: cachedMessages(for: threadId)
        )
        messagesByThread[threadId] = pruned
        selectedThreadStreamFlushGate.recordVisibleChange()
    }

    /// Merge one durable committed row batch into the S2 cache (in-memory, cheap —
    /// keeps the cursor current per row). Rows alone never change visible content
    /// (rows render from the frame's snapshot), so this only marks a pending
    /// visible change; the frame-final snapshot apply settles the flush gate.
    func applyStreamedCommittedMessages(_ messages: [GaryxTranscriptMessage], threadId: String) async {
        guard selectedThread?.id == threadId else { return }
        let base = transcriptSnapshot(for: threadId)
        let savedAt = Date()
        let window = await Task.detached(priority: .utility) {
            GaryxTranscriptCacheLogic.merged(
                into: base,
                threadId: threadId,
                fetched: messages,
                pageInfo: nil,
                direction: .forward,
                savedAt: savedAt
            )
        }.value
        guard selectedThread?.id == threadId else { return }
        cachedTranscriptSnapshots[threadId] = window
        selectedThreadStreamFlushGate.recordVisibleChange()
    }

    private func applyThreadRenderSnapshot(_ snapshot: GaryxRenderSnapshot, threadId: String) {
        guard selectedThread?.id == threadId else { return }
        let appliedBefore = renderSnapshotsByThread[threadId]
            ?? transcriptSnapshot(for: threadId)?.renderSnapshot
        setRenderSnapshot(snapshot, for: threadId)
        let pagination = applyRenderWindowPagination(snapshot.window, threadId: threadId)
        let base = transcriptSnapshot(for: threadId)
        let windowHasMoreBefore: Bool
        let windowNextBeforeIndex: Int?
        if let pagination {
            windowHasMoreBefore = pagination.hasMoreBefore
            windowNextBeforeIndex = pagination.nextBeforeIndex
        } else {
            windowHasMoreBefore = base?.hasMoreBefore ?? false
            windowNextBeforeIndex = base?.nextBeforeIndex
        }
        let window = GaryxCachedTranscript(
            threadId: threadId,
            savedAt: Date(),
            messages: base?.messages ?? [],
            renderSnapshot: snapshot,
            hasMoreBefore: windowHasMoreBefore,
            nextBeforeIndex: windowNextBeforeIndex
        )
        cachedTranscriptSnapshots[threadId] = window
        if !isThreadBusy(threadId) {
            persistTranscriptCacheWindowInBackground(window)
        }
        markThreadHistoryLoaded(threadId)
        settleSelectedThreadStreamFrame(
            snapshotChanged: GaryxStreamFlushGate.snapshotChanged(snapshot, appliedBefore: appliedBefore),
            for: threadId
        )
    }

    @discardableResult
    private func applyRenderWindowPagination(
        _ renderWindow: GaryxRenderWindow?,
        threadId: String
    ) -> GaryxHistoryPaginationState? {
        guard selectedThread?.id == threadId else { return nil }
        guard let renderWindow else {
            selectedThreadRenderFloorByThread[threadId] = nil
            return nil
        }
        selectedThreadRenderFloorByThread[threadId] = renderWindow.floorSeq
        let next = GaryxHistoryPaginationPlanner.applyingRenderWindow(
            renderWindow,
            current: selectedHistoryPaginationState(),
            cached: cachedHistoryPaginationState(for: threadId)
        )
        applySelectedThreadHistoryPagination(next)
        return next
    }

    /// Leading-edge flush cadence (`GaryxStreamFlushGate` owns the decisions):
    /// a changing frame on the quiet edge renders immediately and arms the
    /// window timer; changing frames inside the window coalesce (the window
    /// end drains them); no-op frames are inert. Called once per applied
    /// render frame from `applyThreadRenderSnapshot`.
    private func settleSelectedThreadStreamFrame(snapshotChanged: Bool, for threadId: String) {
        switch selectedThreadStreamFlushGate.settleFrame(snapshotChanged: snapshotChanged) {
        case .flushNowAndArmWindow:
            armSelectedThreadStreamFlushWindow(for: threadId)
            Task { [weak self] in
                await self?.flushSelectedThreadStreamWindow(for: threadId)
            }
        case .absorb, .skip:
            break
        }
    }

    private func armSelectedThreadStreamFlushWindow(for threadId: String) {
        selectedThreadStreamFlushTask?.cancel()
        selectedThreadStreamFlushTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.streamedCommittedFlushDelayNanos)
            guard !Task.isCancelled else { return }
            await self?.selectedThreadStreamFlushWindowDidElapse(for: threadId)
        }
    }

    private func selectedThreadStreamFlushWindowDidElapse(for threadId: String) async {
        // Re-check on the MainActor: a timer cancelled after its post-sleep
        // check must not nil a newer timer handle or double-drive the gate.
        guard !Task.isCancelled else { return }
        selectedThreadStreamFlushTask = nil
        switch selectedThreadStreamFlushGate.windowElapsed() {
        case .flushBacklogAndRearmWindow:
            armSelectedThreadStreamFlushWindow(for: threadId)
            await flushSelectedThreadStreamWindow(for: threadId)
        case .closeWindow:
            break
        }
    }

    /// Timer lifecycle lives exclusively here and in arm/elapse so the gate
    /// can never disagree with the timer (armed iff gate is in its window).
    private func cancelSelectedThreadStreamFlushWindow() {
        selectedThreadStreamFlushTask?.cancel()
        selectedThreadStreamFlushTask = nil
        selectedThreadStreamFlushGate.reset()
    }

    /// Render the accumulated committed window once and, when the run is idle, persist
    /// it (the in-memory window already advanced the cursor per row; if the app dies
    /// mid-run the rows are re-fetched via after_index from the last persisted cursor).
    private func flushSelectedThreadStreamWindow(
        for threadId: String,
        respectingTaskCancellation: Bool = false
    ) async {
        guard !respectingTaskCancellation || !Task.isCancelled else { return }
        guard selectedThread?.id == threadId,
              let window = cachedTranscriptSnapshots[threadId] else { return }
        let prepared = await prepareSelectedThreadStreamWindowFlush(window, threadId: threadId)
        guard !respectingTaskCancellation || !Task.isCancelled else { return }
        // Abort only when the prepared output could differ (a no-op re-apply
        // refreshes savedAt without changing render inputs and must not kill
        // the render — the no-op settled `.skip`, so no window-end flush
        // would cover it). A content change did settle/mark pending, so the
        // window-end flush re-renders after this abort.
        guard selectedThread?.id == threadId,
              cachedTranscriptSnapshots[threadId]?.renderEquivalent(to: window) == true else { return }
        applyTranscriptRunState(prepared.runState, threadId: threadId)
        if !prepared.threadRunActive {
            persistTranscriptCacheWindowInBackground(window)
        }
        setPreparedMessages(prepared.messages, for: threadId)
        markThreadHistoryLoaded(threadId)
    }

    private func prepareSelectedThreadStreamWindowFlush(
        _ window: GaryxCachedTranscript,
        threadId: String
    ) async -> GaryxPreparedSelectedThreadTranscriptUpdate {
        let localMessages = cachedMessages(for: threadId)
        let localRunTrackerBusy = runTracker.isThreadBusy(threadId)
        let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId]
        return await Task.detached(priority: .utility) {
            GaryxPreparedSelectedThreadTranscriptUpdate.make(
                from: window,
                localMessages: localMessages,
                localRunTrackerBusy: localRunTrackerBusy,
                activeAssistantMessageId: activeAssistantMessageId
            )
        }.value
    }

}
