import Foundation

// S5 — resumable per-thread transcript stream for the open thread. Connects
// `/api/threads/{id}/stream?after_seq=<cursor>`: the replay (committed_message
// events) is the catch-up, the live tail follows on the same channel, and a
// reconnect resumes from the cursor (transfers only seq > cursor). committed_message
// rows are durable (the gateway emits them after the jsonl flush), so they advance
// the cursor; transient deltas/tool events are rendered via the existing handler.
// While this stream owns a thread, the global stream skips that thread's transcript
// events (see handleGlobalStreamEvent).
//
// Self-heal: the broadcast bus is best-effort, so a slow consumer can miss events
// (tokio broadcast Lagged). committed_message carries a gapless seq; if a live row
// arrives non-contiguously we reconnect from the last contiguous seq and the file
// replay refills the hole. A dropped connection is recovered by the reconnect loop
// (URLSession surfaces it as an error / request timeout), and persistent failure
// falls back to the S3 global-stream + reconcile-poll path.
extension GaryxMobileModel {
    /// Cursor the per-thread stream resumes from: the highest committed index we
    /// already hold (cache window or rendered history), as a seq (index + 1).
    func selectedThreadStreamCursor(for threadId: String) -> Int {
        if let afterCursor = transcriptSnapshot(for: threadId)?.afterCursor {
            return afterCursor + 1
        }
        let maxIndex = cachedMessages(for: threadId).compactMap(\.historyIndex).max()
        return maxIndex.map { $0 + 1 } ?? 0
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
        // Take ownership immediately so the global stream stops applying this
        // thread's transcript events; the stream's replay backfills anything that
        // raced the handoff.
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
    }

    /// Fall back to the S3 path (global stream + after_index + reconcile poll) when
    /// the per-thread stream cannot be sustained, so we never lose live updates.
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
                // (committed_message has a preceding `id:`; deltas have just `data:`).
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
        if type == "committed_message" {
            guard let seq = (object["seq"] as? NSNumber)?.intValue ?? (object["seq"] as? Int),
                  let messageObject = object["message"],
                  let messageData = try? JSONSerialization.data(withJSONObject: messageObject),
                  var message = try? JSONDecoder().decode(GaryxTranscriptMessage.self, from: messageData)
            else {
                return false
            }
            // Mid-stream seq hole (a dropped broadcast event): resume the reconnect
            // from the last contiguous seq so the file replay refills it. The first row
            // of a connection (lastSeq == 0) is exempt — the replay may legitimately
            // start above the cursor when the far-behind cap truncated older rows.
            if selectedThreadStreamConnectionLastSeq > 0,
               seq > selectedThreadStreamConnectionLastSeq + 1 {
                selectedThreadStreamResumeOverride = selectedThreadStreamConnectionLastSeq
                return true
            }
            // Stale/duplicate already applied on this connection (replay/live overlap).
            if seq <= selectedThreadStreamConnectionLastSeq {
                return false
            }
            // The committed row carries no index in its body; derive it from the
            // gapless seq so it dedups against history rows (id "history:N").
            message.index = seq - 1
            message.id = "history:\(seq - 1)"
            applyStreamedCommittedMessage(message, threadId: threadId)
            selectedThreadStreamConnectionLastSeq = seq
            return false
        }
        if type == "ping" { return false }
        // Transient live events (deltas / tool / done / title) reuse the existing
        // per-event handler, bypassing the global-stream ownership gate.
        if let event = try? client().decodeStreamEvent(trimmed) {
            await handleGlobalStreamEvent(event, replay: false, bypassStreamOwnership: true)
        }
        return false
    }

    /// Merge one durable committed row into the S2 cache and re-render the thread.
    /// Disk persistence happens only when the thread's run is idle: during an active
    /// run the in-memory window stays current (the cursor reads from it), and if the
    /// app dies mid-run the rows are re-fetched via after_index from the last persisted
    /// cursor — so we avoid a full-window disk write per streamed row (the run-end
    /// reconcile and the next idle merge persist the final window).
    func applyStreamedCommittedMessage(_ message: GaryxTranscriptMessage, threadId: String) {
        guard selectedThread?.id == threadId else { return }
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

        let threadRunActive = remoteBusyThreadIds.contains(threadId)
        if !threadRunActive {
            transcriptCacheStore.save(window)
        }
        let remoteMessages = mobileMessages(from: window.messages, live: threadRunActive)
        setMessages(
            mergedMessages(
                remoteMessages,
                withLocal: cachedMessages(for: threadId),
                preserveRemoteBeforeIndex: window.firstIndex,
                threadRunActive: threadRunActive
            ),
            for: threadId,
            reconcileActiveAssistant: true
        )
        markThreadHistoryLoaded(threadId)
    }
}
