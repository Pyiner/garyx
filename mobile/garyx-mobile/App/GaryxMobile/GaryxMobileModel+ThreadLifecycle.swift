import Foundation

// Thread selection and open, new-thread drafts and creation, bot-group
// open, archive/delete/rename, and per-thread runtime settings updates.
extension GaryxMobileModel {
    func isThreadSummaryRunning(_ thread: GaryxThreadSummary) -> Bool {
        GaryxThreadSummaryRunStateResolver.isRunning(thread)
    }

    func selectThread(
        _ thread: GaryxThreadSummary,
        invalidatesPendingThreadOpen: Bool = true,
        source: GaryxMobilePanelOpenSource = .replace
    ) async {
        // Home-list baseline (docs/agents/mobile-ui.md): the stream is
        // ensured at show time; the bounded history refresh races it. The
        // M3-era same-thread home-reopen deferral (suppress the stream until
        // the history refresh returned) is gone — it delayed live output by a
        // full history roundtrip (TASK-1786), and starting at show can never
        // tear down a live stream (startSelectedThreadStream early-returns
        // for an owned, alive stream).
        showSelectedThread(
            thread,
            invalidatesPendingThreadOpen: invalidatesPendingThreadOpen,
            source: source
        )
        // Bound the open to the newest ~threadHistoryUserQueryLimit user turns: always
        // refresh from the gateway, which returns the forward delta when the cached
        // cursor is within that window, or the newest window + `reset` when the cursor
        // is older (the client overwrites its cache). With no cache it seeds the newest
        // window. The stream then resumes near the tail (live only); older history
        // pages in on scroll-up. The stream supersedes the reconcile poll and falls
        // back to it (and the after_index HTTP path) on failure.
        await loadSelectedThreadHistory()
        // Recovery net, not the primary start: no-op while the stream is
        // owned and alive; picks the stream up when the show-time start was
        // skipped (e.g. connection not yet ready at show).
        ensureSelectedThreadStreamForVisibleConversation()
    }

    func showSelectedThread(
        _ thread: GaryxThreadSummary,
        invalidatesPendingThreadOpen: Bool = true,
        source: GaryxMobilePanelOpenSource = .replace
    ) {
        if invalidatesPendingThreadOpen {
            invalidatePendingThreadOpen()
        }
        if isWorkflowRunSurfaceActive {
            clearWorkflowRunSurface()
        }
        let previousThreadId = selectedThread?.id
        if previousThreadId != thread.id {
            advanceSelectedThreadDraftGeneration()
            // Bump the cold-open generation so any in-flight restore task for the
            // previous thread (or a prior open of this one) aborts (TASK-1751 P1).
            selectedThreadColdOpenGeneration &+= 1
            // Reset the render window to the newest page for the new thread
            // (TASK-1751 P3); event-driven, before any body eval.
            resetSelectedTurnRowsWindow()
            switchComposerDraft(to: thread.id)
            selectedThreadRecoveryTask?.cancel()
            selectedThreadRecoveryTask = nil
            selectedThreadRecoveryThreadId = nil
            selectedThreadHistoryRetryTask?.cancel()
            selectedThreadHistoryRetryTask = nil
            selectedThreadHistoryRetryThreadId = nil
            selectedThreadHistoryRetryCount = 0
            cancelSelectedThreadReconcileLoop()
            resetSelectedThreadHistoryPagination()
        }
        selectedThread = thread
        if !thread.excludeFromRecent {
            persistOpenedThreadDestination(.chat(threadId: thread.id))
        }
        clearPendingNewThreadAgentTarget()
        clearPendingBotDraft()
        draftThreadTitle = thread.title
        openConversation(
            source: source,
            invalidatesPendingThreadOpen: false
        )
        if previousThreadId != thread.id {
            let inMemory = cachedMessages(for: thread.id)
            if inMemory.isEmpty {
                // Cold start / first open this session. Do NOT decode the persisted
                // window synchronously on the main actor — a large cache blocks the
                // whole UI (TASK-1751 P1). Show the loading state (with no messages,
                // no render snapshot and history not yet loaded,
                // isAwaitingInitialHistory is true) and restore asynchronously; the
                // network refresh below (loadSelectedThreadHistory) races and wins.
                messages = []
                spawnColdOpenTranscriptRestore(threadId: thread.id)
            } else {
                messages = inMemory
                // Warm open: lock the window floor for the already-present rows.
                lockSelectedTurnRowsWindowFloorIfNeeded()
            }
        }
    }

    /// Asynchronously restore the persisted committed window for a cold open
    /// without blocking the main actor. Loads + decodes + maps off-main, then
    /// applies only if `GaryxColdOpenRestorePolicy` still says the result is
    /// fresh (TASK-1751 P1).
    private func spawnColdOpenTranscriptRestore(threadId: String) {
        let capturedGeneration = selectedThreadColdOpenGeneration
        let capturedMirrorGeneration = transcriptMirror.generation(for: threadId)
        Task { [weak self] in
            guard let self else { return }
            guard let snapshot = await self.loadTranscriptSnapshotFromDiskAsync(for: threadId) else { return }
            let mapped: [GaryxMobileMessage]
            if snapshot.messages.isEmpty {
                mapped = []
            } else {
                mapped = await Task.detached(priority: .utility) {
                    GaryxMobileTranscriptMapper.mobileMessages(from: snapshot.messages, live: false)
                }.value
            }
            self.applyColdOpenTranscriptRestore(
                threadId: threadId,
                snapshot: snapshot,
                mapped: mapped,
                capturedGeneration: capturedGeneration,
                capturedMirrorGeneration: capturedMirrorGeneration
            )
        }
    }

    private func applyColdOpenTranscriptRestore(
        threadId: String,
        snapshot: GaryxCachedTranscript,
        mapped: [GaryxMobileMessage],
        capturedGeneration: UInt64,
        capturedMirrorGeneration: UInt64
    ) {
        let state = GaryxColdOpenRestorePolicy.State(
            restoredThreadId: threadId,
            selectedThreadId: selectedThread?.id,
            capturedGeneration: capturedGeneration,
            currentGeneration: selectedThreadColdOpenGeneration,
            capturedMirrorGeneration: capturedMirrorGeneration,
            currentMirrorGeneration: transcriptMirror.generation(for: threadId),
            threadHistoryLoaded: threadHistoryLoadedIds.contains(threadId),
            hasRenderSnapshot: renderSnapshotsByThread[threadId] != nil,
            hasMessages: !cachedMessages(for: threadId).isEmpty
        )
        // Seed the mirror (advances the forward cursor for the incremental fetch)
        // when the looser mirror gate passes and the mirror is still absent.
        if GaryxColdOpenRestorePolicy.shouldSeedMirror(state), !transcriptMirror.contains(threadId) {
            setTranscriptMirror(snapshot, for: threadId)
        }
        guard GaryxColdOpenRestorePolicy.shouldApply(state) else { return }
        if let renderSnapshot = snapshot.renderSnapshot {
            setRenderSnapshot(renderSnapshot, for: threadId)
        }
        guard !mapped.isEmpty else { return }
        setMessages(mapped, for: threadId)
    }

    func openNewThreadDraft(agentTargetOverride: String? = nil) {
        invalidatePendingThreadOpen()
        advanceSelectedThreadDraftGeneration()
        selectedThreadColdOpenGeneration &+= 1
        resetSelectedTurnRowsWindow()
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
        selectedThreadHistoryRetryTask?.cancel()
        selectedThreadHistoryRetryTask = nil
        selectedThreadHistoryRetryThreadId = nil
        selectedThreadHistoryRetryCount = 0
        cancelSelectedThreadReconcileLoop()
        stopSelectedThreadStream()
        clearWorkflowRunSurface()
        selectedThreadHistoryRequestId = nil
        isLoadingSelectedThreadHistory = false
        resetSelectedThreadHistoryPagination()
        clearPendingBotDraft()
        selectedThread = nil
        draftThreadTitle = ""
        setPendingNewThreadAgentTarget(agentTargetOverride)
        clearNewThreadModelOverride()
        switchComposerDraft(to: newThreadComposerDraftKey)
        messages = []
        activePanel = .chat
        setSidebarVisible(false)
        lastError = nil
    }

    func createThread() async {
        invalidatePendingThreadOpen()
        clearPendingBotDraft()
        await createThread(workspaceOverride: nil)
    }

    func createThreadFromCurrentDraft() async {
        invalidatePendingThreadOpen()
        guard currentPendingBotDraft() != nil else {
            await createThread()
            return
        }
        do {
            saveGatewaySettings()
            let existingThreadId = selectedThread?.id
            let thread = try await ensureSelectedThread()
            activePanel = .chat
            draftThreadTitle = thread.title
            if existingThreadId == nil {
                clearMessages(for: thread.id)
            }
            setSidebarVisible(false)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createThread(workspaceOverride: String?, agentOverride: String? = nil) async {
        invalidatePendingThreadOpen()
        do {
            saveGatewaySettings()
            let workspace = (workspaceOverride ?? newThreadWorkspace).trimmingCharacters(in: .whitespacesAndNewlines)
            let agentId = newThreadAgentTargetId(agentOverride: agentOverride)
            let workspaceMode = workspaceModeForNewThread(workspace: workspace)
            let modelOverride = newThreadModelOverride.trimmingCharacters(in: .whitespacesAndNewlines)
            let reasoningEffortOverride = newThreadReasoningEffortOverride
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let serviceTierOverride = newThreadServiceTierOverride
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let thread = try await client().createThread(
                GaryxCreateThreadRequest(
                    workspaceDir: workspace.isEmpty ? nil : workspace,
                    workspaceMode: workspaceMode,
                    agentId: agentId.isEmpty ? nil : agentId,
                    model: modelOverride.isEmpty ? nil : modelOverride,
                    modelReasoningEffort: reasoningEffortOverride.isEmpty ? nil : reasoningEffortOverride,
                    modelServiceTier: serviceTierOverride.isEmpty ? nil : serviceTierOverride,
                    metadata: ["client": "garyx-mobile"]
                )
            )
            threads.insert(thread, at: 0)
            threadHistoryLoadedIds.insert(thread.id)
            selectedThread = thread
            clearPendingNewThreadAgentTarget()
            clearNewThreadModelOverride()
            clearPendingBotDraft()
            switchComposerDraft(to: thread.id)
            draftThreadTitle = thread.title
            activePanel = .chat
            clearMessages(for: thread.id)
            setSidebarVisible(false)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func workspaceModeForNewThread(workspace: String) -> String {
        GaryxNewThreadWorkspaceModePolicy.workspaceMode(
            workspace: workspace,
            preferredMode: newThreadWorkspaceMode,
            gitStatuses: workspaceGitStatuses
        )
    }

    func createThread(inWorkspace workspacePath: String) async {
        invalidatePendingThreadOpen()
        clearPendingBotDraft()
        await createThread(workspaceOverride: workspacePath)
    }

    func openBotGroup(_ group: GaryxMobileBotGroup) async {
        let openThreadId = group.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines).garyxTrimmedNilIfEmpty
            ?? group.defaultOpenThreadId?.trimmingCharacters(in: .whitespacesAndNewlines).garyxTrimmedNilIfEmpty
        if let openThreadId {
            await openThread(id: openThreadId)
            return
        }

        let workspace = group.workspaceDir?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let agentId = group.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        invalidatePendingThreadOpen()
        advanceSelectedThreadDraftGeneration()
        pendingBotId = Self.botSelectorId(channel: group.channel, accountId: group.accountId)
        pendingBotWorkspace = workspace.isEmpty ? nil : workspace
        pendingBotAgentId = agentId.isEmpty ? nil : agentId
        pendingBotDraftGeneration = selectedThreadDraftGeneration
        clearPendingNewThreadAgentTarget()
        cancelSelectedThreadReconcileLoop()
        selectedThread = nil
        resetSelectedThreadHistoryPagination()
        draftThreadTitle = ""
        switchComposerDraft(to: newThreadComposerDraftKey)
        messages = []
        activePanel = .chat
        setSidebarVisible(false)
        lastError = nil
    }

    func deleteSelectedThread() async {
        guard let selectedThread else { return }
        await archiveThread(selectedThread)
    }

    func archiveThread(_ thread: GaryxThreadSummary) async {
        let threadId = thread.id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canArchiveThreadId(threadId) else {
            lastError = "This thread is active or managed by an automation."
            return
        }
        await archiveThreadRecord(threadId: threadId)
    }

    func deleteThread(_ thread: GaryxThreadSummary) async {
        guard canDeleteThread(thread) else {
            lastError = "This thread is active or managed by an automation or channel."
            return
        }
        do {
            _ = try await client().deleteThread(threadId: thread.id)
            removeArchivedThreadLocally(thread.id)
            if selectedThread?.id == thread.id {
                self.selectedThread = nil
                draftThreadTitle = ""
                discardComposerDraft(forThread: thread.id)
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
            messagesByThread[thread.id] = nil
            messageSignaturesByThread[thread.id] = nil
            activeAssistantMessageIdsByThread[thread.id] = nil
            threadResidencyTracker.remove(thread.id)
            clearTranscriptCache(for: thread.id)
            await refreshThreads()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func renameSelectedThread(to proposedTitle: String? = nil) async {
        guard let selectedThread else { return }
        let title = (proposedTitle ?? draftThreadTitle).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, title != selectedThread.title else { return }
        do {
            let updated = try await client().updateThread(threadId: selectedThread.id, label: title)
            self.selectedThread = updated
            draftThreadTitle = updated.title
            if let index = threads.firstIndex(where: { $0.id == updated.id }) {
                threads[index] = updated
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateSelectedThreadRuntimeSettings(
        model: String? = nil,
        reasoningEffort: String? = nil,
        serviceTier: String? = nil
    ) async {
        guard let selectedThread else { return }
        let threadId = selectedThread.id
        let mutationId = UUID()
        let previousSelectedRuntime = selectedThread.threadRuntime
        let previousListRuntime = threads.first(where: { $0.id == threadId })?.threadRuntime
        threadRuntimeMutationIds[threadId] = mutationId
        applyOptimisticThreadRuntimeSettings(
            threadId: threadId,
            model: model,
            reasoningEffort: reasoningEffort,
            serviceTier: serviceTier
        )
        do {
            let updated = try await client().updateThread(
                threadId: threadId,
                model: model,
                modelReasoningEffort: reasoningEffort,
                modelServiceTier: serviceTier
            )
            guard threadRuntimeMutationIds[threadId] == mutationId else { return }
            threadRuntimeMutationIds[threadId] = nil
            if self.selectedThread?.id == threadId {
                var next = updated
                next.threadRuntime = updated.threadRuntime ?? self.selectedThread?.threadRuntime
                self.selectedThread = next
                draftThreadTitle = next.title
            }
            if let index = threads.firstIndex(where: { $0.id == threadId }) {
                var next = updated
                next.threadRuntime = updated.threadRuntime ?? threads[index].threadRuntime
                threads[index] = next
            }
            await loadSelectedThreadHistory()
        } catch {
            guard threadRuntimeMutationIds[threadId] == mutationId else { return }
            threadRuntimeMutationIds[threadId] = nil
            restoreThreadRuntimeSettings(
                threadId: threadId,
                selectedRuntime: previousSelectedRuntime,
                listRuntime: previousListRuntime
            )
            lastError = displayMessage(for: error)
        }
    }

    private func applyOptimisticThreadRuntimeSettings(
        threadId: String,
        model: String?,
        reasoningEffort: String?,
        serviceTier: String? = nil
    ) {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return }
        guard let base = selectedThread?.id == normalizedThreadId
            ? selectedThread
            : threads.first(where: { $0.id == normalizedThreadId }) else {
            return
        }
        var runtime = base.threadRuntime ?? GaryxThreadRuntimeSummary(
            agentId: base.agentId,
            providerType: base.providerType
        )
        if let model {
            let value = model.garyxTrimmedNilIfEmpty
            runtime.modelOverride = value
            runtime.model = value
        }
        if let reasoningEffort {
            let value = reasoningEffort.garyxTrimmedNilIfEmpty
            runtime.modelReasoningEffortOverride = value
            runtime.modelReasoningEffort = value
        }
        if let serviceTier {
            let value = serviceTier.garyxTrimmedNilIfEmpty
            runtime.modelServiceTierOverride = value
            runtime.modelServiceTier = value
        }
        applyThreadRuntimeSummary(runtime, threadId: normalizedThreadId)
    }

    private func restoreThreadRuntimeSettings(
        threadId: String,
        selectedRuntime: GaryxThreadRuntimeSummary?,
        listRuntime: GaryxThreadRuntimeSummary?
    ) {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return }
        if selectedThread?.id == normalizedThreadId,
           var selectedThread {
            selectedThread.threadRuntime = selectedRuntime
            self.selectedThread = selectedThread
        }
        if let index = threads.firstIndex(where: { $0.id == normalizedThreadId }) {
            threads[index].threadRuntime = listRuntime
        }
    }
}
