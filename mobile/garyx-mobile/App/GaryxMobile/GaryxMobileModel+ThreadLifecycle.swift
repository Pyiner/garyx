import Foundation
import os

private let lifecycleMutationLogger = Logger(
    subsystem: "com.garyx.mobile",
    category: "thread-lifecycle"
)

enum GaryxMobileLifecycleCompletion<Response: Sendable>: Sendable {
    case applied(Response)
    case rejected(code: String, message: String)
    case operationIdConflict(message: String)
    case exhausted(message: String)
    case cancelled
}

// Thread selection and open, new-thread drafts and creation, bot-group
// open, archive/delete/rename, and per-thread runtime settings updates.
extension GaryxMobileModel {
    func makeLifecycleMutationRequest(
        kind: GaryxLifecycleMutationKind,
        threadId: String,
        endpointKeys: [String] = []
    ) -> GaryxLifecycleMutationRequest? {
        let identities = Set(
            [
                recentThreadFeeds.allFeed.storeIncarnationId,
                recentThreadFeeds.nonTaskFeed.storeIncarnationId,
                threadFavoritesState.storeIncarnationId,
            ]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        )
        guard identities.count == 1, let incarnation = identities.first else {
            return nil
        }
        return GaryxLifecycleMutationRequest(
            kind: kind,
            threadId: threadId,
            endpointKeys: endpointKeys,
            expectedStoreIncarnation: incarnation,
            gatewayScope: currentGatewayScopeId,
            gatewayRequestToken: gatewayRequestToken
        )
    }

    func performLifecycleMutation<Response: Decodable & Sendable>(
        request: GaryxLifecycleMutationRequest,
        dispatch: (GaryxLifecycleMutationAttempt) async -> GaryxGatewayMutationResult<Response>
    ) async -> GaryxMobileLifecycleCompletion<Response> {
        var state = GaryxLifecycleMutationState(request: request)
        while let attempt = state.nextAttempt() {
            guard lifecycleRequestIsCurrent(request), !Task.isCancelled else {
                return .cancelled
            }
            let result = await dispatch(attempt)
            guard lifecycleRequestIsCurrent(request), !Task.isCancelled else {
                return .cancelled
            }
            switch state.settle(result) {
            case .applied(let response):
                return .applied(response)
            case .rejected(let code, let message):
                return .rejected(code: code, message: message)
            case .operationIdConflict(let message):
                lifecycleMutationLogger.error(
                    "operation_id conflict operation=\(request.operationId, privacy: .public) thread=\(request.threadId, privacy: .public)"
                )
                return .operationIdConflict(message: message)
            case .exhausted(let message):
                return .exhausted(message: message)
            case .retry(let policyDelay):
                let delay = lifecycleRetryDelayOverrideNanoseconds ?? policyDelay
                if delay > 0 {
                    do {
                        try await Task.sleep(nanoseconds: delay)
                    } catch {
                        return .cancelled
                    }
                }
            }
        }
        return .cancelled
    }

    private func lifecycleRequestIsCurrent(
        _ request: GaryxLifecycleMutationRequest
    ) -> Bool {
        request.gatewayRequestToken == gatewayRequestToken
            && request.gatewayScope == currentGatewayScopeId
    }

    func recoverLifecycleIdentity() async {
        lastError = "Thread storage identity is unavailable. Refresh and try again."
        await forceReplaceThreadFeedsAfterAmbiguousLifecycle()
    }

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
        let entry = showSelectedThread(
            thread,
            invalidatesPendingThreadOpen: invalidatesPendingThreadOpen,
            source: source
        )
        // History, persisted-window restore, and stream activation are started
        // by `conversationRouteContentPreparationBegan` after the Core route
        // policy has delivered terminal placeholder frames. This keeps every
        // response mapping and subscription callback out of the push window.
        if productionRouteStore.isAttached {
            await waitForConversationContentActivation(entry.id)
        } else {
            await loadSelectedThreadHistory()
            ensureSelectedThreadStreamForVisibleConversation()
        }
    }

    /// Completes a deep-link open after the container has committed and made
    /// the prepared occurrence visible. It intentionally performs no route
    /// write; the NavigationIntent transaction already owns that mutation.
    func activatePreparedThread(_ thread: GaryxThreadSummary) async {
        let resolvedThread = summaryWithCommittedRunState(thread)
        cacheThreadSummaries([resolvedThread])
        applySelectedThreadRouteProjection(resolvedThread, preparesContent: false)
    }

    @discardableResult
    func showSelectedThread(
        _ thread: GaryxThreadSummary,
        invalidatesPendingThreadOpen: Bool = true,
        source: GaryxMobilePanelOpenSource = .replace
    ) -> GaryxRouteEntry {
        if invalidatesPendingThreadOpen {
            invalidatePendingThreadOpen()
        }
        cacheThreadSummaries([thread])
        let entry = productionRouteStore.open(
            .conversation(threadID: thread.id),
            source: source
        )
        // Unit-test and cold-start fallback before the UIKit owner attaches.
        if !productionRouteStore.isAttached {
            applySelectedThreadRouteProjection(thread)
        }
        return entry
    }

    /// Selection is a compatibility projection of the canonical route top.
    /// It is never consulted to decide which route the container renders.
    func applySelectedThreadRouteProjection(
        _ thread: GaryxThreadSummary,
        preparesContent: Bool = true
    ) {
        let previousThreadId = selectedThread?.id
        if previousThreadId != thread.id {
            cancelConversationContentActivation()
            advanceSelectedThreadDraftGeneration()
            // Bump the cold-open generation so any in-flight restore task for the
            // previous thread (or a prior open of this one) aborts (TASK-1751 P1).
            selectedThreadColdOpenGeneration &+= 1
            // Reset the render window to the newest page for the new thread
            // (TASK-1751 P3); event-driven, before any body eval.
            resetSelectedTurnRowsWindow()
            if !productionRouteStore.isAttached {
                activateComposerPayload(for: .thread(thread.id))
            }
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
        if GaryxLastOpenedThreadRestorationPolicy.shouldPersistLastOpenedThread() {
            persistLastOpenedThreadId(thread.id)
        }
        clearPendingNewThreadAgentTarget()
        clearPendingBotDraft()
        draftThreadTitle = thread.title
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
                if preparesContent {
                    spawnColdOpenTranscriptRestore(threadId: thread.id)
                }
            } else {
                messages = inMemory
                // Warm open: lock the window floor for the already-present rows.
                lockSelectedTurnRowsWindowFloorIfNeeded()
            }
        }
    }

    /// Starts conversation runtime work only after the route's Core-owned
    /// staged presentation reaches `.preparingLiveContent`. At this point the
    /// navigation settle is terminal and an opaque UIKit snapshot masks first
    /// transcript mapping and render-pipeline materialization.
    func conversationRouteContentPreparationBegan(
        _ entry: GaryxRouteEntry,
        contentDidBecomeReady: @escaping @MainActor () -> Void
    ) {
        guard case .conversation(let threadID) = entry.destination,
              selectedThread?.id == threadID,
              conversationContentActivationOccurrenceID != entry.id
        else { return }

        let supersededOccurrenceID = conversationContentActivationOccurrenceID
        conversationContentActivationTask?.cancel()
        if let supersededOccurrenceID {
            resumeConversationContentActivationWaiters(for: supersededOccurrenceID)
        }
        conversationContentActivationOccurrenceID = entry.id
        completedConversationContentActivationOccurrenceID = nil
        conversationContentActivationTask = Task { @MainActor [weak self] in
            guard let self else { return }
            defer { self.finishConversationContentActivation(entry.id) }
            guard !Task.isCancelled,
                  self.selectedThread?.id == threadID,
                  self.productionRouteStore.path.last?.id == entry.id
            else { return }

            let hasRenderableCachedSnapshot = !self.cachedMessages(for: threadID).isEmpty
            if !hasRenderableCachedSnapshot {
                self.spawnColdOpenTranscriptRestore(threadId: threadID)
            }
            self.ensureSelectedThreadStreamForVisibleConversation()

            // Bound the open to the newest committed window. Mapping remains
            // off-main; applying its result is now safely behind the snapshot.
            // Even with a renderable cache, keep the cover until this initial
            // refresh is applied. Revealing the cache first lets the refresh's
            // deferred AttributeGraph preference pass hitch a later visible
            // frame.
            await self.loadSelectedThreadHistory()
            guard !Task.isCancelled,
                  self.selectedThread?.id == threadID,
                  self.productionRouteStore.path.last?.id == entry.id
            else { return }
            self.ensureSelectedThreadStreamForVisibleConversation()
            contentDidBecomeReady()
        }
    }

    func cancelConversationContentActivation() {
        conversationContentActivationTask?.cancel()
        conversationContentActivationTask = nil
        conversationContentActivationOccurrenceID = nil
        completedConversationContentActivationOccurrenceID = nil
        let pendingWaiters = conversationContentActivationWaiters.values.flatMap { $0 }
        conversationContentActivationWaiters.removeAll(keepingCapacity: false)
        for waiter in pendingWaiters {
            waiter.resume()
        }
    }

    private func waitForConversationContentActivation(
        _ occurrenceID: GaryxRouteInstanceID
    ) async {
        guard completedConversationContentActivationOccurrenceID != occurrenceID else {
            return
        }
        await withCheckedContinuation { continuation in
            conversationContentActivationWaiters[occurrenceID, default: []].append(continuation)
        }
    }

    private func finishConversationContentActivation(
        _ occurrenceID: GaryxRouteInstanceID
    ) {
        if conversationContentActivationOccurrenceID == occurrenceID {
            conversationContentActivationTask = nil
            completedConversationContentActivationOccurrenceID = occurrenceID
        }
        resumeConversationContentActivationWaiters(for: occurrenceID)
    }

    private func resumeConversationContentActivationWaiters(
        for occurrenceID: GaryxRouteInstanceID
    ) {
        let waiters = conversationContentActivationWaiters.removeValue(forKey: occurrenceID) ?? []
        for waiter in waiters {
            waiter.resume()
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
        activatePreparedNewThreadDraft(agentTargetOverride: agentTargetOverride)
        let destination = GaryxRouteDestination.conversationDraft(
            draftID: newThreadComposerPayloadKey.draftRouteID
        )
        if !productionRouteStore.isAttached {
            activateComposerPayload(for: newThreadComposerPayloadKey)
        }
        _ = productionRouteStore.open(destination, source: .replace)
        if !productionRouteStore.isAttached {
            applyCanonicalRouteProjection(productionRouteStore.path)
        }
    }

    /// Applies only the content state associated with an already prepared
    /// draft route. This is also shared by the synchronous direct-open path.
    func activatePreparedNewThreadDraft(
        agentTargetOverride: String? = nil,
        freezesAgentTarget: Bool = false
    ) {
        invalidatePendingThreadOpen()
        cancelConversationContentActivation()
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
        selectedThreadHistoryRequestId = nil
        isLoadingSelectedThreadHistory = false
        resetSelectedThreadHistoryPagination()
        clearPendingBotDraft()
        draftThreadTitle = ""
        setPendingNewThreadAgentTarget(
            agentTargetOverride,
            freezesSelection: freezesAgentTarget
        )
        clearNewThreadModelOverride()
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
        let runtimeGeneration = gatewayRequestToken
        do {
            saveGatewaySettings()
            let existingThreadId = selectedThread?.id
            let thread = try await ensureSelectedThread()
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else { return }
            if case .some(.draft) = composerPayloadCoordinator.activeKey {
                try await promoteActiveComposerPayload(to: thread.id)
            }
            activePanel = .chat
            draftThreadTitle = thread.title
            if existingThreadId == nil {
                clearMessages(for: thread.id)
            }
            setSidebarVisible(false)
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func createThread(workspaceOverride: String?, agentOverride: String? = nil) async {
        invalidatePendingThreadOpen()
        let runtimeGeneration = gatewayRequestToken
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
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else { return }
            let mutationId = nextThreadMutationId(kind: "insert", threadId: thread.id)
            let affectedStoreIds = threadMutationHubStore.value
                .residentStoreIdsAffectedByInsert(workspacePath: thread.workspacePath)
            _ = threadMutationHubStore.value.began(
                mutationId: mutationId,
                kind: .insert(threadId: thread.id),
                gatewayRuntimeEpoch: threadMutationHubStore.value.gatewayRuntimeEpoch,
                affectedStoreIds: affectedStoreIds
            )
            let authority = GaryxThreadMutationAuthority(
                membership: .upsertAtHead(threadId: thread.id),
                summary: thread
            )
            _ = threadMutationHubStore.value.committed(
                mutationId: mutationId,
                gatewayRuntimeEpoch: threadMutationHubStore.value.gatewayRuntimeEpoch,
                authority: authority
            )
            applyThreadMutationAuthorityToResidentProviders(authority)
            threadHistoryLoadedIds.insert(thread.id)
            if !productionRouteStore.isAttached {
                selectedThread = thread
            }
            clearPendingNewThreadAgentTarget()
            clearNewThreadModelOverride()
            clearPendingBotDraft()
            if case .some(.draft) = composerPayloadCoordinator.activeKey {
                try await promoteActiveComposerPayload(to: thread.id)
            } else if !productionRouteStore.isAttached {
                activateComposerPayload(for: .thread(thread.id))
            }
            draftThreadTitle = thread.title
            activePanel = .chat
            clearMessages(for: thread.id)
            setSidebarVisible(false)
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
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
        resetSelectedThreadHistoryPagination()
        draftThreadTitle = ""
        let destination = GaryxRouteDestination.conversationDraft(
            draftID: newThreadComposerPayloadKey.draftRouteID
        )
        if !productionRouteStore.isAttached {
            activateComposerPayload(for: newThreadComposerPayloadKey)
        }
        _ = productionRouteStore.open(destination, source: .replace)
        if !productionRouteStore.isAttached {
            applyCanonicalRouteProjection(productionRouteStore.path)
        }
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
        let runtimeGeneration = gatewayRequestToken
        let gatewayClient: GaryxGatewayClient
        do {
            gatewayClient = try client()
        } catch {
            lastError = displayMessage(for: error)
            return
        }
        guard let request = makeLifecycleMutationRequest(
            kind: .delete,
            threadId: thread.id
        ) else {
            await recoverLifecycleIdentity()
            return
        }
        let mutationEpoch = threadMutationHubStore.value.gatewayRuntimeEpoch
        let mutationId = nextThreadMutationId(kind: "delete", threadId: thread.id)
        _ = threadMutationHubStore.value.began(
            mutationId: mutationId,
            kind: .archive(threadId: thread.id),
            gatewayRuntimeEpoch: mutationEpoch
        )
        refreshResidentThreadListStores()
        let result: GaryxMobileLifecycleCompletion<GaryxDeleteResult> =
            await performLifecycleMutation(request: request) { attempt in
                await gatewayClient.deleteThread(
                    threadId: attempt.request.threadId,
                    operationId: attempt.request.operationId,
                    expectedStoreIncarnation: attempt.request.expectedStoreIncarnation
                )
            }
        guard runtimeGeneration == gatewayRequestToken else { return }
        switch result {
        case .applied:
            if selectedThread?.id == thread.id {
                self.selectedThread = nil
                draftThreadTitle = ""
                discardComposerPayload(forThread: thread.id)
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
            _ = threadMutationHubStore.value.committed(
                mutationId: mutationId,
                gatewayRuntimeEpoch: mutationEpoch,
                authority: GaryxThreadMutationAuthority(
                    membership: .remove(threadId: thread.id)
                )
            )
            removeArchivedThreadLocally(thread.id)
            messagesByThread[thread.id] = nil
            messageSignaturesByThread[thread.id] = nil
            activeAssistantMessageIdsByThread[thread.id] = nil
            threadResidencyTracker.remove(thread.id)
            clearTranscriptCache(for: thread.id)
            await refreshThreads(source: .userAction)
        case .rejected(let code, let message):
            let reconstructionTickets: [GaryxThreadReconstructionTicket]
            if code == "wrong_incarnation" {
                reconstructionTickets = threadMutationHubStore.value.ambiguous(
                    mutationId: mutationId,
                    gatewayRuntimeEpoch: mutationEpoch
                )
            } else {
                _ = threadMutationHubStore.value.rolledBack(
                    mutationId: mutationId,
                    gatewayRuntimeEpoch: mutationEpoch,
                    message: message
                )
                reconstructionTickets = []
            }
            refreshResidentThreadListStores()
            lastError = message
            if code == "wrong_incarnation" {
                await forceReplaceThreadFeedsAfterAmbiguousLifecycle(
                    reconstructionTickets: reconstructionTickets
                )
            }
        case .operationIdConflict(let message), .exhausted(let message):
            let tickets = threadMutationHubStore.value.ambiguous(
                mutationId: mutationId,
                gatewayRuntimeEpoch: mutationEpoch
            )
            refreshResidentThreadListStores()
            lastError = message
            await forceReplaceThreadFeedsAfterAmbiguousLifecycle(
                reconstructionTickets: tickets
            )
        case .cancelled:
            _ = threadMutationHubStore.value.rolledBack(
                mutationId: mutationId,
                gatewayRuntimeEpoch: mutationEpoch
            )
            refreshResidentThreadListStores()
            return
        }
    }

    func renameSelectedThread(to proposedTitle: String? = nil) async {
        guard let selectedThread else { return }
        let threadId = selectedThread.id
        let title = (proposedTitle ?? draftThreadTitle).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, title != selectedThread.title else { return }
        let runtimeGeneration = gatewayRequestToken
        let mutationEpoch = threadMutationHubStore.value.gatewayRuntimeEpoch
        let rollbackSummary = threadRenameRollbackSummaries[threadId] ?? selectedThread
        threadRenameRollbackSummaries[threadId] = rollbackSummary
        let mutationId = nextThreadMutationId(kind: "rename", threadId: threadId)
        if let superseded = threadRenameMutationIds[threadId] {
            _ = threadMutationHubStore.value.rolledBack(
                mutationId: superseded,
                gatewayRuntimeEpoch: mutationEpoch,
                message: "Superseded by a newer title update."
            )
        }
        threadRenameMutationIds[threadId] = mutationId
        _ = threadMutationHubStore.value.began(
            mutationId: mutationId,
            kind: .rename(threadId: threadId),
            gatewayRuntimeEpoch: mutationEpoch
        )
        var optimistic = selectedThread
        optimistic.title = title
        self.selectedThread = optimistic
        draftThreadTitle = title
        cacheThreadSummaries([optimistic])
        do {
            let updated = try await client().updateThread(threadId: threadId, label: title)
            guard runtimeGeneration == gatewayRequestToken,
                  threadRenameMutationIds[threadId] == mutationId else { return }
            threadRenameMutationIds[threadId] = nil
            threadRenameRollbackSummaries[threadId] = nil
            if self.selectedThread?.id == threadId {
                self.selectedThread = updated
                draftThreadTitle = updated.title
            }
            cacheThreadSummaries([updated])
            _ = threadMutationHubStore.value.committed(
                mutationId: mutationId,
                gatewayRuntimeEpoch: mutationEpoch,
                authority: GaryxThreadMutationAuthority(summary: updated)
            )
        } catch {
            guard runtimeGeneration == gatewayRequestToken,
                  threadRenameMutationIds[threadId] == mutationId else { return }
            threadRenameMutationIds[threadId] = nil
            threadRenameRollbackSummaries[threadId] = nil
            if self.selectedThread?.id == threadId {
                self.selectedThread = rollbackSummary
                draftThreadTitle = rollbackSummary.title
            }
            cacheThreadSummaries([rollbackSummary])
            _ = threadMutationHubStore.value.rolledBack(
                mutationId: mutationId,
                gatewayRuntimeEpoch: mutationEpoch,
                message: displayMessage(for: error)
            )
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
        let runtimeGeneration = gatewayRequestToken
        let mutationEpoch = threadMutationHubStore.value.gatewayRuntimeEpoch
        let mutationId = nextThreadMutationId(kind: "runtime", threadId: threadId)
        let rollbackSnapshot = threadRuntimeRollbackSnapshots[threadId]
            ?? GaryxThreadRuntimeRollbackSnapshot(
                selectedRuntime: selectedThread.threadRuntime,
                listRuntime: threadSummaryCache.summary(for: threadId)?.threadRuntime
            )
        threadRuntimeRollbackSnapshots[threadId] = rollbackSnapshot
        if let superseded = threadRuntimeMutationIds[threadId] {
            _ = threadMutationHubStore.value.rolledBack(
                mutationId: superseded,
                gatewayRuntimeEpoch: mutationEpoch,
                message: "Superseded by newer runtime settings."
            )
        }
        threadRuntimeMutationIds[threadId] = mutationId
        _ = threadMutationHubStore.value.began(
            mutationId: mutationId,
            kind: .runtime(threadId: threadId),
            gatewayRuntimeEpoch: mutationEpoch
        )
        applyOptimisticThreadRuntimeSettings(
            threadId: threadId,
            model: model,
            reasoningEffort: reasoningEffort,
            serviceTier: serviceTier,
            mutationId: mutationId
        )
        do {
            let updated = try await client().updateThread(
                threadId: threadId,
                model: model,
                modelReasoningEffort: reasoningEffort,
                modelServiceTier: serviceTier
            )
            guard runtimeGeneration == gatewayRequestToken,
                  threadRuntimeMutationIds[threadId] == mutationId else { return }
            threadRuntimeMutationIds[threadId] = nil
            threadRuntimeRollbackSnapshots[threadId] = nil
            if self.selectedThread?.id == threadId {
                var next = updated
                next.threadRuntime = updated.threadRuntime ?? self.selectedThread?.threadRuntime
                self.selectedThread = next
                draftThreadTitle = next.title
            }
            var next = updated
            next.threadRuntime = updated.threadRuntime
                ?? threadSummaryCache.summary(for: threadId)?.threadRuntime
            cacheThreadSummaries([next])
            _ = threadMutationHubStore.value.committed(
                mutationId: mutationId,
                gatewayRuntimeEpoch: mutationEpoch,
                authority: GaryxThreadMutationAuthority(summary: next)
            )
            await loadSelectedThreadHistory()
        } catch {
            guard runtimeGeneration == gatewayRequestToken,
                  threadRuntimeMutationIds[threadId] == mutationId else { return }
            threadRuntimeMutationIds[threadId] = nil
            threadRuntimeRollbackSnapshots[threadId] = nil
            restoreThreadRuntimeSettings(
                threadId: threadId,
                selectedRuntime: rollbackSnapshot.selectedRuntime,
                listRuntime: rollbackSnapshot.listRuntime
            )
            _ = threadMutationHubStore.value.rolledBack(
                mutationId: mutationId,
                gatewayRuntimeEpoch: mutationEpoch,
                message: displayMessage(for: error)
            )
            lastError = displayMessage(for: error)
        }
    }

    private func applyOptimisticThreadRuntimeSettings(
        threadId: String,
        model: String?,
        reasoningEffort: String?,
        serviceTier: String? = nil,
        mutationId: GaryxThreadMutationID
    ) {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return }
        guard let base = selectedThread?.id == normalizedThreadId
            ? selectedThread
            : threadSummaryCache.summary(for: normalizedThreadId) else {
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
        applyThreadRuntimeSummary(
            runtime,
            threadId: normalizedThreadId,
            mutationId: mutationId
        )
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
        if var cached = threadSummaryCache.summary(for: normalizedThreadId) {
            cached.threadRuntime = listRuntime
            cacheThreadSummaries([cached])
        }
    }
}
