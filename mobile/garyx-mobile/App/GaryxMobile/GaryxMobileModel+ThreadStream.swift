import Foundation

// S5 — resumable per-thread transcript stream for the open thread. Connects
// `/api/threads/{id}/stream?after_seq=<cursor>`: the replay (committed_message
// events) is the catch-up, the live tail follows on the same channel, and a
// reconnect resumes from the cursor (transfers only seq > cursor). committed_message
// rows are durable (the gateway emits them after the jsonl flush), so they always
// persist to the S2 cache and advance the cursor; transient deltas/tool events are
// rendered via the existing handler. While this stream owns a thread, the global
// stream skips that thread's transcript events (see handleGlobalStreamEvent).
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
    }

    /// Fall back to the S3 path (global stream + after_index + reconcile poll) when
    /// the per-thread stream cannot be sustained, so we never lose live updates.
    private func fallBackFromSelectedThreadStream(threadId: String) async {
        guard selectedThread?.id == threadId else { return }
        streamOwnedThreadId = nil
        selectedThreadStreamTask = nil
        selectedThreadStreamGeneration = nil
        await loadSelectedThreadHistory()
        startSelectedThreadReconcileLoop()
    }

    func runSelectedThreadStream(threadId: String, generation: UUID) async {
        var consecutiveFailures = 0
        while !Task.isCancelled, hasGatewaySettings {
            guard selectedThreadStreamGeneration == generation,
                  selectedThread?.id == threadId else { break }
            do {
                let cursor = selectedThreadStreamCursor(for: threadId)
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
                consecutiveFailures = 0
                // Each gateway event is a single `data:` line (committed_message has
                // a preceding `id:`; deltas have just `data:`). Process each `data:`
                // line immediately rather than buffering until a blank separator —
                // Swift's AsyncLineSequence does not yield the SSE blank lines, so a
                // blank-line flush would never fire.
                for try await line in bytes.lines {
                    if Task.isCancelled || selectedThreadStreamGeneration != generation { break }
                    guard line.hasPrefix("data:") else { continue }
                    var value = String(line.dropFirst(5))
                    if value.hasPrefix(" ") { value.removeFirst() }
                    if value.isEmpty { continue }
                    await handleSelectedThreadStreamPayload(value, threadId: threadId)
                }
            } catch {
                consecutiveFailures += 1
            }
            if Task.isCancelled || selectedThreadStreamGeneration != generation { break }
            // After repeated failures, fall back rather than spin.
            if consecutiveFailures >= 4 {
                await fallBackFromSelectedThreadStream(threadId: threadId)
                return
            }
            let delay = UInt64(min(consecutiveFailures, 5)) * 1_000_000_000
            try? await Task.sleep(nanoseconds: max(delay, 500_000_000))
        }
    }

    private func handleSelectedThreadStreamPayload(_ payload: String, threadId: String) async {
        let trimmed = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let data = trimmed.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return
        }
        let type = object["type"] as? String
        if type == "committed_message" {
            guard let seq = (object["seq"] as? NSNumber)?.intValue ?? (object["seq"] as? Int),
                  let messageObject = object["message"],
                  let messageData = try? JSONSerialization.data(withJSONObject: messageObject),
                  var message = try? JSONDecoder().decode(GaryxTranscriptMessage.self, from: messageData)
            else {
                return
            }
            // The committed row carries no index in its body; derive it from the
            // gapless seq so it dedups against history rows (id "history:N").
            message.index = seq - 1
            message.id = "history:\(seq - 1)"
            applyStreamedCommittedMessage(message, threadId: threadId)
            return
        }
        if type == "ping" { return }
        // Transient live events (deltas / tool / done / title) reuse the existing
        // per-event handler, bypassing the global-stream ownership gate.
        if let event = try? client().decodeStreamEvent(trimmed) {
            await handleGlobalStreamEvent(event, replay: false, bypassStreamOwnership: true)
        }
    }

    /// Merge one durable committed row into the S2 cache (always persisted — it is
    /// committed, not overlay) and re-render the thread.
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
        transcriptCacheStore.save(window)

        let threadRunActive = remoteBusyThreadIds.contains(threadId)
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
