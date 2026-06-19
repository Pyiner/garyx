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
    func selectedThreadStreamCursor(for threadId: String) -> Int {
        let snapshot = transcriptSnapshot(for: threadId)
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

    func stopSelectedThreadStream() {
        selectedThreadStreamTask?.cancel()
        selectedThreadStreamTask = nil
        selectedThreadStreamGeneration = nil
        streamOwnedThreadId = nil
        selectedThreadStreamResumeOverride = nil
        selectedThreadStreamFlushTask?.cancel()
        selectedThreadStreamFlushTask = nil
    }

    /// Fall back to the after_index + reconcile poll path when the per-thread stream
    /// cannot be sustained, so we still converge from committed transcript history.
    private func fallBackFromSelectedThreadStream(threadId: String) async {
        guard selectedThread?.id == threadId else { return }
        streamOwnedThreadId = nil
        selectedThreadStreamTask = nil
        selectedThreadStreamGeneration = nil
        selectedThreadStreamResumeOverride = nil
        await loadSelectedThreadHistory()
        startSelectedThreadReconcileLoop()
    }

    func runSelectedThreadStream(threadId: String, generation: UUID) async {
        var consecutiveFailures = 0
        while !Task.isCancelled, hasGatewaySettings {
            guard selectedThreadStreamGeneration == generation,
                  selectedThread?.id == threadId else { break }
            // Reset per-connection progress before (re)connecting.
            selectedThreadStreamConnectionLastSeq = 0
            do {
                let cursor = selectedThreadStreamResumeOverride
                    ?? selectedThreadStreamCursor(for: threadId)
                selectedThreadStreamResumeOverride = nil
                let request = try client().threadStreamRequest(threadId: threadId, afterSeq: cursor)
                let (bytes, response) = try await URLSession.shared.bytes(for: request)
                guard let http = response as? HTTPURLResponse else {
                    throw GaryxGatewayError.invalidHTTPResponse
                }
                if http.statusCode == 404 {
                    // Gateway without the per-thread stream endpoint → permanent
                    // fallback (don't retry).
                    await fallBackFromSelectedThreadStream(threadId: threadId)
                    return
                }
                guard (200..<300).contains(http.statusCode) else {
                    throw GaryxGatewayError.invalidHTTPResponse
                }
                guard selectedThreadStreamGeneration == generation else { break }
                // The gateway emits each event as one compact-JSON `data:` line
                // (thread_render_frame has a preceding `id:`; pings have just `data:`).
                // Process each `data:` line immediately rather than buffering until a
                // blank separator — Swift's AsyncLineSequence does not yield the SSE
                // blank lines, and the `:` keepalive / `id:` lines are skipped by the
                // `data:` prefix check.
                for try await line in bytes.lines {
                    if Task.isCancelled || selectedThreadStreamGeneration != generation { break }
                    guard line.hasPrefix("data:") else { continue }
                    var value = String(line.dropFirst(5))
                    if value.hasPrefix(" ") { value.removeFirst() }
                    if value.isEmpty { continue }
                    if await handleSelectedThreadStreamPayload(value, threadId: threadId) {
                        // A live seq gap was detected (resume override set): end this
                        // connection so the loop reconnects and the replay refills it.
                        break
                    }
                }
            } catch {
                consecutiveFailures += 1
            }
            if Task.isCancelled || selectedThreadStreamGeneration != generation { break }
            // A connection that delivered committed rows is healthy — a seq gap that
            // broke the read self-heals on the next connect via the resume override.
            // Only a connection that never made progress counts toward the fallback.
            if selectedThreadStreamConnectionLastSeq > 0 {
                consecutiveFailures = 0
            }
            if consecutiveFailures >= 4 {
                await fallBackFromSelectedThreadStream(threadId: threadId)
                return
            }
            let delay = UInt64(min(consecutiveFailures, 5)) * 1_000_000_000
            try? await Task.sleep(nanoseconds: max(delay, 500_000_000))
        }
    }

    /// Processes one SSE `data:` payload. Returns `true` when a live committed-seq gap
    /// is detected and the caller should reconnect (the resume override is set to the
    /// last contiguous seq so the replay refills the hole).
    private func handleSelectedThreadStreamPayload(_ payload: String, threadId: String) async -> Bool {
        let trimmed = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let data = trimmed.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return false
        }
        let type = object["type"] as? String
        if type == "thread_render_frame" {
            guard let frame = try? JSONDecoder().decode(GaryxThreadRenderFrame.self, from: data) else {
                return false
            }
            return await handleSelectedThreadRenderFrame(frame, threadId: threadId)
        }
        if type == "committed_message" {
            // Block 3 requires render_state for rendering. A bare legacy event is not
            // a UI fallback; the reconnect/reconcile paths remain the sync fallback.
            return false
        }
        if type == "ping" { return false }
        return false
    }

    private func handleSelectedThreadRenderFrame(_ frame: GaryxThreadRenderFrame, threadId: String) async -> Bool {
        let frameThreadId = frame.threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard frameThreadId.isEmpty || frameThreadId == threadId else { return false }
        for event in frame.events {
            guard event.type == "committed_message",
                  let seq = event.seq,
                  var message = event.message else {
                continue
            }
            switch GaryxStreamSeqPlanner.decide(
                incomingSeq: seq,
                connectionLastSeq: selectedThreadStreamConnectionLastSeq
            ) {
            case .gapReconnect(let resumeAfterSeq):
                selectedThreadStreamResumeOverride = resumeAfterSeq
                return true
            case .stale:
                continue
            case .apply:
                message.index = seq - 1
                message.id = "history:\(seq - 1)"
                if GaryxTranscriptControlRewritePlanner.action(for: message) == .refetchAuthoritativeTranscript {
                    selectedThreadStreamConnectionLastSeq = max(selectedThreadStreamConnectionLastSeq, seq)
                    await refetchSelectedThreadAfterTranscriptRewrite(threadId: threadId)
                    return true
                }
                applyStreamedCommittedMessage(message, threadId: threadId)
                selectedThreadStreamConnectionLastSeq = seq
            }
        }
        selectedThreadStreamConnectionLastSeq = max(
            selectedThreadStreamConnectionLastSeq,
            frame.renderState.basedOnSeq
        )
        applyThreadRenderSnapshot(frame.renderState, threadId: threadId)
        return false
    }

    private func refetchSelectedThreadAfterTranscriptRewrite(threadId: String) async {
        selectedThreadStreamFlushTask?.cancel()
        selectedThreadStreamFlushTask = nil
        selectedThreadStreamResumeOverride = 0
        clearTranscriptCache(for: threadId)
        resetSelectedThreadHistoryPagination()
        clearMessages(for: threadId)
        await loadSelectedThreadHistory()
        selectedThreadStreamResumeOverride = selectedThreadStreamCursor(for: threadId)
    }

    /// Merge one durable committed row into the S2 cache (in-memory, cheap — keeps the
    /// cursor current per row) and coalesce the view render + disk persist into one
    /// flush per interval. A large catch-up replays many committed rows back-to-back;
    /// rendering/persisting each one would rebuild the whole list and rewrite the whole
    /// window per row, flickering the page. The flush shows the accumulated window a
    /// few times instead of N.
    func applyStreamedCommittedMessage(_ message: GaryxTranscriptMessage, threadId: String) {
        guard selectedThread?.id == threadId else { return }
        applyCommittedTranscriptMessage(message, threadId: threadId)
        let base = transcriptSnapshot(for: threadId)
        let window = GaryxTranscriptCacheLogic.merged(
            into: base,
            threadId: threadId,
            fetched: [message],
            pageInfo: nil,
            direction: .forward,
            savedAt: Date()
        )
        cachedTranscriptSnapshots[threadId] = window
        scheduleSelectedThreadStreamFlush(for: threadId)
    }

    private func applyThreadRenderSnapshot(_ snapshot: GaryxRenderSnapshot, threadId: String) {
        guard selectedThread?.id == threadId else { return }
        setRenderSnapshot(snapshot, for: threadId)
        let base = transcriptSnapshot(for: threadId)
        let window = GaryxCachedTranscript(
            threadId: threadId,
            savedAt: Date(),
            messages: base?.messages ?? [],
            renderSnapshot: snapshot,
            hasMoreBefore: base?.hasMoreBefore ?? false,
            nextBeforeIndex: base?.nextBeforeIndex
        )
        cachedTranscriptSnapshots[threadId] = window
        if !isThreadBusy(threadId) {
            transcriptCacheStore.save(window)
        }
        markThreadHistoryLoaded(threadId)
        scheduleSelectedThreadStreamFlush(for: threadId)
    }

    /// Leading-throttle (mirrors scheduleAssistantDeltaFlush): the first row schedules
    /// a flush; rows arriving within the interval are absorbed (the flush reads the
    /// latest window), so a catch-up burst folds into one render + persist. The final
    /// row always lands in a flush because the last scheduled flush reads the latest
    /// in-memory window.
    private func scheduleSelectedThreadStreamFlush(for threadId: String) {
        guard selectedThreadStreamFlushTask == nil else { return }
        selectedThreadStreamFlushTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.streamedCommittedFlushDelayNanos)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                self?.flushSelectedThreadStreamWindow(for: threadId)
            }
        }
    }

    /// Render the accumulated committed window once and, when the run is idle, persist
    /// it (the in-memory window already advanced the cursor per row; if the app dies
    /// mid-run the rows are re-fetched via after_index from the last persisted cursor).
    private func flushSelectedThreadStreamWindow(for threadId: String) {
        selectedThreadStreamFlushTask?.cancel()
        selectedThreadStreamFlushTask = nil
        guard selectedThread?.id == threadId,
              let window = cachedTranscriptSnapshots[threadId] else { return }
        let threadRunActive = isThreadBusy(threadId)
        if !threadRunActive {
            transcriptCacheStore.save(window)
        }
        let remoteMessages = mobileMessages(from: window.messages, live: threadRunActive)
        setMessages(
            mergedMessages(
                remoteMessages,
                withLocal: cachedMessages(for: threadId),
                preserveRemoteBeforeIndex: window.firstIndex
            ),
            for: threadId,
            reconcileActiveAssistant: true
        )
        markThreadHistoryLoaded(threadId)
    }
}
