import Foundation

// Selected-thread history loading (incremental open with retry) and
// older-history pagination over the committed transcript cache.
extension GaryxMobileModel {
    func loadSelectedThreadHistory() async {
        guard let selectedThread else {
            selectedThreadHistoryRetryTask?.cancel()
            selectedThreadHistoryRetryTask = nil
            selectedThreadHistoryRetryThreadId = nil
            selectedThreadHistoryRetryCount = 0
            messages = []
            selectedThreadHasMoreHistoryBefore = false
            selectedThreadNextHistoryBeforeIndex = nil
            isLoadingOlderThreadHistory = false
            return
        }
        let threadId = selectedThread.id
        let requestId = UUID()
        selectedThreadHistoryRequestId = requestId
        isLoadingSelectedThreadHistory = true
        if selectedThreadHistoryRetryThreadId != threadId {
            selectedThreadHistoryRetryCount = 0
            selectedThreadHistoryRetryThreadId = threadId
        }
        defer {
            if selectedThreadHistoryRequestId == requestId {
                isLoadingSelectedThreadHistory = false
            }
        }
        do {
            // Incremental open: when a committed window is cached, fetch only the
            // `after_index` delta and reconstruct the full window from cache ∪ delta;
            // otherwise load the most recent few turns. The persisted window was
            // already shown by the caller, so this just brings it current.
            let transcript = try await fetchThreadTranscriptIncrementally(threadId: threadId)
            guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
            await applySelectedThreadTranscript(transcript, threadId: threadId)
        } catch {
            guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
            if cachedMessages(for: threadId).isEmpty {
                messages = []
            }
            handleSelectedThreadHistoryLoadFailure(threadId: threadId, error: error)
        }
    }

    func applySelectedThreadTranscript(_ transcript: GaryxThreadTranscript, threadId: String) async {
        await applyThreadTranscriptToCache(
            transcript,
            threadId: threadId,
            preservingLoadedOlderPages: true,
            scheduleRecoveryIfSelected: true
        )
        startSelectedThreadReconcileLoop()
    }

    func applyThreadTranscriptToCache(
        _ transcript: GaryxThreadTranscript,
        threadId: String,
        preservingLoadedOlderPages: Bool,
        scheduleRecoveryIfSelected: Bool
    ) async {
        let prepared = await prepareSelectedThreadTranscriptUpdate(
            transcript,
            threadId: threadId
        )
        applyPreparedSelectedThreadTranscriptToCache(
            prepared,
            transcript: transcript,
            threadId: threadId,
            preservingLoadedOlderPages: preservingLoadedOlderPages,
            scheduleRecoveryIfSelected: scheduleRecoveryIfSelected
        )
    }

    @discardableResult
    func applyPreparedSelectedThreadTranscriptToCache(
        _ prepared: GaryxPreparedSelectedThreadTranscriptUpdate,
        transcript: GaryxThreadTranscript,
        threadId: String,
        preservingLoadedOlderPages: Bool,
        scheduleRecoveryIfSelected: Bool
    ) -> Bool {
        markThreadHistoryLoaded(threadId)
        selectedThreadActivitySignatures[threadId] = prepared.activitySignature
        applyTranscriptRunState(prepared.runState, threadId: threadId)
        if let runtime = transcript.threadRuntime {
            applyThreadRuntimeSummary(runtime, threadId: threadId)
        }
        if selectedThread?.id == threadId {
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: preservingLoadedOlderPages
            )
        }
        setPreparedMessages(prepared.messages, for: threadId)
        if scheduleRecoveryIfSelected {
            scheduleSelectedThreadRecoveryIfNeeded(threadId: threadId)
        }
        return prepared.threadRunActive
    }

    func prepareSelectedThreadTranscriptUpdate(
        _ transcript: GaryxThreadTranscript,
        threadId: String
    ) async -> GaryxPreparedSelectedThreadTranscriptUpdate {
        let localMessages = cachedMessages(for: threadId)
        let localRunTrackerBusy = runTracker.isThreadBusy(threadId)
        let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId]
        return await Task.detached(priority: .utility) {
            GaryxPreparedSelectedThreadTranscriptUpdate.make(
                from: transcript,
                localMessages: localMessages,
                localRunTrackerBusy: localRunTrackerBusy,
                activeAssistantMessageId: activeAssistantMessageId
            )
        }.value
    }

    @discardableResult
    func applyPreparedThreadTranscriptToCache(
        _ prepared: GaryxPreparedThreadTranscriptUpdate,
        transcript: GaryxThreadTranscript,
        threadId: String,
        preservingLoadedOlderPages: Bool,
        scheduleRecoveryIfSelected: Bool
    ) -> Bool {
        markThreadHistoryLoaded(threadId)
        selectedThreadActivitySignatures[threadId] = prepared.activitySignature
        applyTranscriptRunState(prepared.runState, threadId: threadId)
        if let runtime = transcript.threadRuntime {
            applyThreadRuntimeSummary(runtime, threadId: threadId)
        }
        if selectedThread?.id == threadId {
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: preservingLoadedOlderPages
            )
        }
        setMessages(
            mergedMessages(
                prepared.remoteMessages,
                withLocal: cachedMessages(for: threadId),
                preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
            ),
            for: threadId,
            reconcileActiveAssistant: true
        )
        if scheduleRecoveryIfSelected {
            scheduleSelectedThreadRecoveryIfNeeded(threadId: threadId)
        }
        return prepared.runState.busy
    }

    func markThreadHistoryLoaded(_ threadId: String) {
        threadHistoryLoadedIds.insert(threadId)
        if selectedThreadHistoryRetryThreadId == threadId {
            selectedThreadHistoryRetryTask?.cancel()
            selectedThreadHistoryRetryTask = nil
            selectedThreadHistoryRetryThreadId = nil
            selectedThreadHistoryRetryCount = 0
        }
        completePendingThreadLink(threadId)
    }

    func handleSelectedThreadHistoryLoadFailure(threadId: String, error: Error) {
        let message = displayMessage(for: error)
        guard cachedMessages(for: threadId).isEmpty,
              !threadHistoryLoadedIds.contains(threadId) else {
            lastError = message
            return
        }
        if selectedThreadHistoryRetryCount < Self.selectedThreadHistoryRetryLimit {
            scheduleSelectedThreadHistoryRetry(threadId: threadId)
            if Self.isTransientGatewayErrorMessage(message) {
                gatewaySettingsStatus = "Loading thread messages"
                return
            }
        } else {
            threadHistoryLoadedIds.insert(threadId)
        }
        lastError = message
    }

    func scheduleSelectedThreadHistoryRetry(threadId: String) {
        guard selectedThread?.id == threadId,
              selectedThreadHistoryRetryTask == nil,
              case .ready = connectionState else {
            return
        }
        selectedThreadHistoryRetryThreadId = threadId
        selectedThreadHistoryRetryCount += 1
        let retryIndex = selectedThreadHistoryRetryCount
        let delay = min(
            700_000_000 * UInt64(1 << min(retryIndex - 1, 3)),
            5_000_000_000
        )
        selectedThreadHistoryRetryTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: delay)
            guard !Task.isCancelled else { return }
            await self?.runSelectedThreadHistoryRetry(threadId: threadId)
        }
    }

    func runSelectedThreadHistoryRetry(threadId: String) async {
        guard selectedThread?.id == threadId else {
            selectedThreadHistoryRetryTask = nil
            return
        }
        selectedThreadHistoryRetryTask = nil
        await loadSelectedThreadHistory()
    }

    func loadOlderSelectedThreadHistory() async {
        guard let selectedThread,
              selectedThreadHasMoreHistoryBefore,
              !isLoadingOlderThreadHistory,
              let beforeIndex = selectedThreadNextHistoryBeforeIndex else {
            return
        }
        let threadId = selectedThread.id
        isLoadingOlderThreadHistory = true
        defer {
            if self.selectedThread?.id == threadId {
                isLoadingOlderThreadHistory = false
            }
        }
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                beforeIndex: beforeIndex,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard self.selectedThread?.id == threadId else { return }
            // Extend the cached committed window backward so older pages persist
            // and survive a cold start, not just this session's memory. A
            // `before_index` page can never contain a transient live row, so it is
            // committed-only and safe to persist even while the run is active.
            let window = await updateTranscriptCache(
                threadId: threadId,
                fetched: transcript,
                direction: .older,
                committedOnly: true
            )
            if let floorSeq = GaryxThreadWindowPlanner.floorSeqForOlderPage(firstIndex: window.firstIndex) {
                selectedThreadRenderFloorByThread[threadId] = floorSeq
            }
            updateSelectedThreadHistoryPagination(threadId: threadId, transcript: transcript)
            prependOlderMessages(
                mobileMessages(from: transcript.messages, live: false),
                for: threadId
            )
            if self.selectedThread?.id == threadId {
                stopSelectedThreadStream()
                startSelectedThreadStream(for: threadId)
            }
        } catch {
            guard self.selectedThread?.id == threadId else { return }
            lastError = displayMessage(for: error)
        }
    }

    func updateSelectedThreadHistoryPagination(
        threadId: String,
        transcript: GaryxThreadTranscript,
        preservingLoadedOlderPages: Bool = false
    ) {
        guard selectedThread?.id == threadId else { return }
        let page = GaryxHistoryPaginationPage(
            hasMoreBefore: transcript.pageInfo?.hasMoreBefore ?? false,
            nextBeforeIndex: transcript.pageInfo?.nextBeforeIndex,
            oldestLoadedIndex: oldestLoadedHistoryIndex(for: threadId),
            latestPageStartIndex: preserveRemoteBeforeIndex(from: transcript)
        )
        let next = GaryxHistoryPaginationPlanner.applyingTranscriptPage(
            page,
            current: selectedHistoryPaginationState(),
            preservingLoadedOlderPages: preservingLoadedOlderPages
        )
        applySelectedThreadHistoryPagination(next)
    }

    func selectedHistoryPaginationState() -> GaryxHistoryPaginationState {
        GaryxHistoryPaginationState(
            hasMoreBefore: selectedThreadHasMoreHistoryBefore,
            nextBeforeIndex: selectedThreadNextHistoryBeforeIndex
        )
    }

    func cachedHistoryPaginationState(for threadId: String) -> GaryxHistoryPaginationState? {
        guard let snapshot = transcriptSnapshot(for: threadId) else {
            return nil
        }
        return GaryxHistoryPaginationState(
            hasMoreBefore: snapshot.hasMoreBefore,
            nextBeforeIndex: snapshot.nextBeforeIndex
        )
    }

    func applySelectedThreadHistoryPagination(_ state: GaryxHistoryPaginationState) {
        if selectedThreadHasMoreHistoryBefore != state.hasMoreBefore {
            selectedThreadHasMoreHistoryBefore = state.hasMoreBefore
        }
        if selectedThreadNextHistoryBeforeIndex != state.nextBeforeIndex {
            selectedThreadNextHistoryBeforeIndex = state.nextBeforeIndex
        }
    }

    func oldestLoadedHistoryIndex(for threadId: String) -> Int? {
        cachedMessages(for: threadId)
            .compactMap(\.historyIndex)
            .min()
    }

    func prependOlderMessages(_ olderMessages: [GaryxMobileMessage], for threadId: String) {
        guard !olderMessages.isEmpty else { return }
        let existingMessages = cachedMessages(for: threadId)
        let existingIds = Set(existingMessages.map(\.id))
        let dedupedOlderMessages = olderMessages.filter { !existingIds.contains($0.id) }
        guard !dedupedOlderMessages.isEmpty else { return }
        setMessages(dedupedOlderMessages + existingMessages, for: threadId)
    }
}
