import Foundation

// Committed transcript run-state derivation applied to thread summaries,
// selected-thread recovery and reconcile loops, and snapshot-payload
// transcript decode.
extension GaryxMobileModel {
    func rebuildThreadRunState(threadId: String, messages: [GaryxTranscriptMessage]) {
        let state = GaryxTranscriptRunStateReducer.reduce(messages)
        applyTranscriptRunState(state, threadId: threadId)
    }

    func applyCommittedTranscriptMessage(_ message: GaryxTranscriptMessage, threadId: String) {
        var state = runStateByThread[threadId] ?? GaryxTranscriptRunState()
        GaryxTranscriptRunStateReducer.apply(message: message, to: &state)
        applyTranscriptRunState(state, threadId: threadId)
    }

    func applyTranscriptRunState(_ state: GaryxTranscriptRunState, threadId: String) {
        let previous = runStateByThread[threadId] ?? GaryxTranscriptRunState()
        if previous == state {
            return
        }
        runStateByThread[threadId] = state
        if previous.busy != state.busy {
            refreshResidentThreadListStores()
        }
        emitCommittedRunStateProjectionDelta(threadId: threadId, state: state)
        applyThreadRunStateSummary(threadId: threadId, state: state)

        if previous.lastUserAckSeq != state.lastUserAckSeq
            || previous.lastUserAckPendingInputId != state.lastUserAckPendingInputId {
            runTracker.acknowledgeProviderInput(
                threadId: threadId,
                pendingInputId: state.lastUserAckPendingInputId
            )
            let nextAssistantId = moveNextPendingDirectFollowUpToAckBoundary(threadId: threadId)
            markActiveAssistantSegmentComplete(for: threadId)
            activeAssistantMessageIdsByThread[threadId] = nextAssistantId
        }

        if previous.title != state.title,
           let title = state.title?.trimmingCharacters(in: .whitespacesAndNewlines),
           !title.isEmpty {
            applyThreadTitleUpdate(threadId: threadId, title: title)
        }

        let observedTerminal = !(state.terminalStatus?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true)
        guard !state.busy, (previous.busy || observedTerminal) else { return }
        pendingDirectFollowUpsByThread[threadId] = nil
        activeAssistantMessageIdsByThread[threadId] = nil
        markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        cancelSelectedThreadRecoveryIfNeeded(threadId: threadId)
        if state.terminalStatus == "interrupted" {
            runTracker.interruptConfirmed(threadId: threadId)
        } else {
            runTracker.completeCommittedRun(threadId: threadId)
        }
        refreshHomeThreadsAfterLocalRunStateChange()
    }

    func replaceRunStateByThread(_ next: [String: GaryxTranscriptRunState]) {
        guard runStateByThread != next else { return }
        let previousBusy = remoteBusyThreadIds
        runStateByThread = next
        if previousBusy != remoteBusyThreadIds {
            refreshResidentThreadListStores()
        }
        emitHomeProjectionSnapshot()
    }

    func emitCommittedRunStateProjectionDelta(threadId: String, state: GaryxTranscriptRunState) {
        homeProjectionGateway.captureCommittedRunStateDelta(threadId: threadId, isRunning: state.busy)
        if !HomeProjectionLiveSourceConfiguration.usesActorSnapshots {
            emitHomeProjectionSnapshot()
        }
    }

    func summaryWithCommittedRunState(_ thread: GaryxThreadSummary) -> GaryxThreadSummary {
        GaryxThreadSummaryCommittedRunStateProjector.summary(
            thread,
            committedState: runStateByThread[thread.id]
        )
    }

    func summary(_ thread: GaryxThreadSummary, applying state: GaryxTranscriptRunState) -> GaryxThreadSummary {
        GaryxThreadSummaryCommittedRunStateProjector.summary(thread, applying: state)
    }

    func applyThreadRunStateSummary(threadId: String, state: GaryxTranscriptRunState) {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return }

        if selectedThread?.id == normalizedThreadId,
           let selectedThread {
            let nextSelectedThread = summary(selectedThread, applying: state)
            if self.selectedThread != nextSelectedThread {
                self.selectedThread = nextSelectedThread
            }
        }
    }

    func applyThreadRuntimeSummary(
        _ runtime: GaryxThreadRuntimeSummary,
        threadId: String,
        mutationId existingMutationId: GaryxThreadMutationID? = nil
    ) {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return }

        func mergedRuntimeSummary(_ thread: GaryxThreadSummary) -> GaryxThreadSummary {
            var updated = thread
            var runtimeMetadata = runtime
            runtimeMetadata.activeRun = nil
            updated.threadRuntime = runtimeMetadata
            if let agentId = runtime.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
               !agentId.isEmpty {
                updated.agentId = agentId
            }
            if let providerType = runtime.providerType?.trimmingCharacters(in: .whitespacesAndNewlines),
               !providerType.isEmpty {
                updated.providerType = providerType
            }
            return updated
        }

        if let cached = threadSummaryCache.summary(for: normalizedThreadId) {
            let nextThread = mergedRuntimeSummary(cached)
            if cached != nextThread {
                let mutationId = existingMutationId
                    ?? nextThreadMutationId(kind: "runtime", threadId: normalizedThreadId)
                if existingMutationId == nil {
                    _ = threadMutationHubStore.value.began(
                        mutationId: mutationId,
                        kind: .runtime(threadId: normalizedThreadId),
                        gatewayRuntimeEpoch: threadMutationHubStore.value.gatewayRuntimeEpoch
                    )
                }
                cacheThreadSummaries([nextThread])
                if existingMutationId == nil {
                    _ = threadMutationHubStore.value.committed(
                        mutationId: mutationId,
                        gatewayRuntimeEpoch: threadMutationHubStore.value.gatewayRuntimeEpoch,
                        authority: GaryxThreadMutationAuthority(summary: nextThread)
                    )
                }
            }
        }
        if selectedThread?.id == normalizedThreadId,
           let selectedThread {
            let nextSelectedThread = mergedRuntimeSummary(selectedThread)
            if self.selectedThread != nextSelectedThread {
                self.selectedThread = nextSelectedThread
            }
        }
    }

    func scheduleSelectedThreadRecoveryIfNeeded(threadId: String) {
        guard selectedThread?.id == threadId,
              remoteBusyThreadIds.contains(threadId),
              selectedThreadRecoveryTask == nil else {
            return
        }
        selectedThreadRecoveryThreadId = threadId
        selectedThreadRecoveryTask = Task { [weak self] in
            var delay: UInt64 = 1_200_000_000
            for _ in 0..<8 {
                try? await Task.sleep(nanoseconds: delay)
                guard !Task.isCancelled else { break }
                await self?.refreshSelectedThreadRuntimeSnapshot(threadId: threadId)
                let shouldContinue = self?.shouldContinueRecoveringSelectedThread(threadId: threadId) ?? false
                if !shouldContinue {
                    break
                }
                delay = min(delay * 2, 5_000_000_000)
            }
            self?.clearSelectedThreadRecoveryTask(threadId: threadId)
        }
    }

    func shouldContinueRecoveringSelectedThread(threadId: String) -> Bool {
        GaryxSelectedThreadRecoveryPolicy.shouldContinueRecovering(
            threadId: threadId,
            selectedThreadId: selectedThread?.id,
            remoteBusyThreadIds: remoteBusyThreadIds
        )
    }

    func clearSelectedThreadRecoveryTask(threadId: String) {
        if selectedThreadRecoveryThreadId == threadId {
            selectedThreadRecoveryTask = nil
            selectedThreadRecoveryThreadId = nil
        }
    }

    func refreshSelectedThreadRuntimeSnapshot(threadId: String) async {
        guard selectedThread?.id == threadId else { return }
        let observedHistoryRequestId = selectedThreadHistoryRequestId
        do {
            let transcript = try await fetchThreadTranscriptIncrementally(threadId: threadId)
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            let prepared = await prepareSelectedThreadTranscriptUpdate(
                transcript,
                threadId: threadId
            )
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            let threadRunActive = applyPreparedSelectedThreadTranscriptToCache(
                prepared,
                transcript: transcript,
                threadId: threadId,
                preservingLoadedOlderPages: true,
                scheduleRecoveryIfSelected: false
            )
            if !threadRunActive {
                await refreshThreads(source: .userAction)
            }
        } catch {
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            let message = displayMessage(for: error)
            if Self.isTransientGatewayErrorMessage(message) {
                gatewaySettingsStatus = "Waiting to sync with gateway"
            } else {
                lastError = message
            }
        }
    }

    func startSelectedThreadReconcileLoop() {
        // The resumable per-thread stream owns liveness for the thread it holds; don't
        // run the 1.5s reconcile poll alongside it (that would re-fetch every 1.5s and
        // again on every run-end). The stream falls back to this poll when it cannot be
        // sustained (see fallBackFromSelectedThreadStream).
        if let owned = streamOwnedThreadId,
           let current = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
           owned == current {
            return
        }
        guard hasGatewaySettings,
              case .ready = connectionState,
              let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            cancelSelectedThreadReconcileLoop()
            return
        }
        if selectedThreadReconcileThreadId == threadId, selectedThreadReconcileTask != nil {
            return
        }
        cancelSelectedThreadReconcileLoop()
        selectedThreadReconcileThreadId = threadId
        selectedThreadReconcileTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Self.selectedThreadReconcileIntervalNanos)
                if Task.isCancelled { break }
                await reconcileSelectedThreadFromGatewayIfChanged(threadId: threadId)
            }
        }
    }

    func cancelSelectedThreadReconcileLoop() {
        selectedThreadReconcileTask?.cancel()
        selectedThreadReconcileTask = nil
        selectedThreadReconcileThreadId = nil
    }

    func reconcileSelectedThreadFromGatewayIfChanged(threadId: String) async {
        guard selectedThread?.id == threadId,
              hasGatewaySettings,
              case .ready = connectionState,
              !isLoadingSelectedThreadHistory else {
            return
        }
        let observedHistoryRequestId = selectedThreadHistoryRequestId
        do {
            // Incremental reconcile: a forward `after_index` delta (usually empty
            // when idle) instead of re-pulling the full window every 1.5s.
            let transcript = try await fetchThreadTranscriptIncrementally(threadId: threadId)
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            let prepared = await prepareSelectedThreadTranscriptUpdate(
                transcript,
                threadId: threadId
            )
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            markThreadHistoryLoaded(threadId)
            if selectedThreadActivitySignatures[threadId] == prepared.activitySignature {
                applyTranscriptRunState(prepared.runState, threadId: threadId)
                return
            }
            let threadRunActive = applyPreparedSelectedThreadTranscriptToCache(
                prepared,
                transcript: transcript,
                threadId: threadId,
                preservingLoadedOlderPages: true,
                scheduleRecoveryIfSelected: false
            )
            if !threadRunActive {
                await refreshThreads(source: .userAction)
            }
        } catch {
            guard selectedThread?.id == threadId else { return }
            let message = displayMessage(for: error)
            if Self.isTransientGatewayErrorMessage(message) {
                gatewaySettingsStatus = "Waiting to sync with gateway"
            } else {
                lastError = message
            }
        }
    }

    func transcript(fromSnapshotPayload payload: [String: GaryxJSONValue]) throws -> GaryxThreadTranscript? {
        try GaryxThreadTranscript.fromSnapshotPayload(payload)
    }
}
