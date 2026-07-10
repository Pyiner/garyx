import Foundation

// Home recent-thread list: refresh and pagination, the widget snapshot
// projection, workspace/bot thread merging, completed-thread history
// hydration, and the background committed-run reconcile loop.
extension GaryxMobileModel {
    func refreshThreads(source: GaryxThreadListRefreshSource) async {
        guard hasGatewaySettings else { return }
        // Concurrent refresh entry points (pull-to-refresh, the 10s loop,
        // the reconcile loop, action refreshes) coalesce into the ticket
        // holder; refreshes never truncate loaded pages (TASK-1802 R2/R9).
        guard let ticket = threadListPager.requestRefresh() else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let previousThreadSummaries = Self.mergedThreadSummaries(threads + [selectedThread].compactMap { $0 })
        let previouslyRemoteBusyThreadIds = remoteBusyThreadIds
        let showsSkeleton = GaryxThreadListRefreshPolicy.showsSkeleton(
            listIsEmpty: pendingThreadArchives.visibleThreadIds(recentThreadIds).isEmpty
        )
        if showsSkeleton, !isLoadingThreads {
            isLoadingThreads = true
        }
        let transactionId = homeProjectionGateway.beginTransaction(label: "refreshThreads")
        defer { homeProjectionGateway.endTransaction(transactionId) }
        defer {
            if isLoadingThreads, runtimeGeneration == gatewayRuntimeGeneration {
                isLoadingThreads = false
            }
        }
        do {
            let gatewayClient = try client()
            async let threadsPage = gatewayClient.listRecentThreads(limit: Self.threadListPageLimit)
            async let threadPinsPage = gatewayClient.listThreadPins()
            let (page, pinsPage) = try await (threadsPage, threadPinsPage)
            guard runtimeGeneration == gatewayRuntimeGeneration else {
                threadListPager.failRefresh(ticket)
                return
            }
            var fetchedThreads = pendingThreadArchives.visibleThreads(page.threads)
            let selectionIdForThisRefresh = selectedThread?.id
            let requiredThreadIds = normalizedThreadIds(
                pendingThreadArchives.visibleThreadIds(pinsPage.threadIds) + [selectionIdForThisRefresh]
            )
            fetchedThreads += await fetchMissingThreadSummaries(
                using: gatewayClient,
                requiredThreadIds: requiredThreadIds,
                existingThreadIds: Set(fetchedThreads.map(\.id))
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else {
                // Release the refresh gate: this transaction is abandoned
                // (no-op if a pager reset already bumped the epoch).
                threadListPager.failRefresh(ticket)
                return
            }
            // Transaction commit point: the ticket completes only after the
            // LAST await, so the pager's refresh gate spans the entire
            // App-layer refresh — a second refresh cannot interleave between
            // this page landing and its state writes and then be overwritten
            // by this (older) page when we resume (review #TASK-1804).
            switch threadListPager.completeRefresh(
                ticket,
                pageOffset: page.offset,
                pageCount: page.count,
                hasMore: page.hasMore
            ) {
            case .abandonedStaleEpoch:
                // The pager was reset mid-flight: the page belongs to the
                // previous gateway and is dropped silently.
                return
            case .abandonedLocalMutation:
                // Archive/delete/pin surgery raced this refresh
                // (review #TASK-1804 round 3): every pre-await snapshot is
                // stale — the surgery's tombstone may already be resolved,
                // so commit-point re-filtering cannot save them. Drop the
                // page and follow up with a fresh refresh, which also
                // replaces the surgery-triggered refresh this one coalesced
                // away.
                Task { [weak self] in
                    await self?.refreshThreads(source: source)
                }
                return
            case .apply(let application):
                commitRefreshedRecentThreadsPage(
                    page: page,
                    pinsPageThreadIds: pinsPage.threadIds,
                    application: application,
                    fetchedThreads: fetchedThreads,
                    previousThreadSummaries: previousThreadSummaries,
                    previouslyRemoteBusyThreadIds: previouslyRemoteBusyThreadIds,
                    selectionIdForThisRefresh: selectionIdForThisRefresh,
                    runtimeGeneration: runtimeGeneration
                )
            }
        } catch {
            threadListPager.failRefresh(ticket)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            presentThreadListRefreshFailure(source: source, error: error)
        }
    }

    /// Synchronous commit of a completed head-refresh transaction. Runs
    /// entirely on the MainActor after the refresh's last await, so all
    /// inputs captured before the backfill are re-filtered here against the
    /// commit-point `pendingThreadArchives`: a thread archived while the
    /// backfill was suspended must not be resurrected by pre-await
    /// snapshots (review #TASK-1804 round 2).
    func commitRefreshedRecentThreadsPage(
        page: GaryxRecentThreadsPage,
        pinsPageThreadIds: [String],
        application: GaryxThreadListRefreshApplication,
        fetchedThreads: [GaryxThreadSummary],
        previousThreadSummaries: [GaryxThreadSummary],
        previouslyRemoteBusyThreadIds: Set<String>,
        selectionIdForThisRefresh: String?,
        runtimeGeneration: UUID
    ) {
        applyPinnedThreadIds(pendingThreadArchives.visibleThreadIds(pinsPageThreadIds))
        applyRecentThreadsHeadPage(page, application: application)
        let visibleFetchedThreads = pendingThreadArchives.visibleThreads(fetchedThreads)
        // Loaded tail summaries always survive a head refresh; which
        // rows are visible is recentThreadIds' concern.
        let existingThreads = pendingThreadArchives.visibleThreads(threads)
        let previousRuntimeByThreadId = Dictionary(
            uniqueKeysWithValues: previousThreadSummaries.compactMap { thread -> (String, GaryxThreadRuntimeSummary)? in
                guard let runtime = thread.threadRuntime else { return nil }
                return (thread.id, runtime)
            }
        )
        let refreshedGatewayThreads = Self.mergedThreadSummaries(visibleFetchedThreads).map { thread in
            var next = thread
            if next.threadRuntime == nil {
                next.threadRuntime = previousRuntimeByThreadId[next.id]
            }
            return next
        }
        let refreshedThreads = refreshedGatewayThreads.map(summaryWithCommittedRunState)
        let mergedThreads = Self.mergedThreadSummaries(existingThreads + refreshedThreads)
        if threads != mergedThreads {
            threads = mergedThreads
        }
        persistRecentThreadsWidgetSnapshot()
        hydrateCompletedRecentThreadHistories(
            previousThreads: previousThreadSummaries,
            previouslyRemoteBusyThreadIds: previouslyRemoteBusyThreadIds,
            refreshedThreads: refreshedGatewayThreads,
            runtimeGeneration: runtimeGeneration
        )
        let currentSelectedId = selectedThread?.id
        if let selectionIdForThisRefresh,
           currentSelectedId == selectionIdForThisRefresh,
           let updatedSelection = threads.first(where: { $0.id == selectionIdForThisRefresh }) {
            var nextSelection = updatedSelection
            if nextSelection.threadRuntime == nil {
                nextSelection.threadRuntime = selectedThread?.threadRuntime
            }
            nextSelection = summaryWithCommittedRunState(nextSelection)
            if selectedThread != nextSelection {
                selectedThread = nextSelection
            }
            if draftThreadTitle != nextSelection.title {
                draftThreadTitle = nextSelection.title
            }
        }
    }

    private func presentThreadListRefreshFailure(source: GaryxThreadListRefreshSource, error: Error) {
        switch GaryxThreadListRefreshPolicy.failurePresentation(source: source) {
        case .toast:
            lastError = displayMessage(for: error)
        case .transientStatus:
            gatewaySettingsStatus = "Waiting to sync with gateway"
        }
    }

    func refreshHomeThreadsAfterLocalRunStart() {
        refreshHomeThreadsAfterLocalRunStateChange()
        scheduleHomeThreadsRunStateRefresh(after: 350_000_000)
    }

    func refreshHomeThreadsAfterLocalRunStateChange() {
        scheduleHomeThreadsRunStateRefresh()
    }

    private func scheduleHomeThreadsRunStateRefresh(after delayNanos: UInt64? = nil) {
        guard hasGatewaySettings else { return }
        Task { [weak self] in
            if let delayNanos {
                try? await Task.sleep(nanoseconds: delayNanos)
                guard !Task.isCancelled else { return }
            }
            await self?.refreshHomeThreadsRunStateIfConnected()
        }
    }

    private func refreshHomeThreadsRunStateIfConnected() async {
        guard hasGatewaySettings, case .ready = connectionState else { return }
        await refreshThreads(source: .backgroundLoop)
    }

    func applyRecentThreadsHeadPage(
        _ page: GaryxRecentThreadsPage,
        application: GaryxThreadListRefreshApplication
    ) {
        let pageIds = pendingThreadArchives.visibleThreads(page.threads).map(\.id)
        let merged: [String]
        switch application {
        case .replaceHead:
            merged = pageIds
        case .mergeBeyondHead:
            merged = GaryxThreadListPageMerge.mergeHead(
                pageIds: pageIds,
                existingIds: pendingThreadArchives.visibleThreadIds(recentThreadIds)
            )
        }
        if recentThreadIds != merged {
            recentThreadIds = merged
        }
    }

    func persistRecentThreadsWidgetSnapshot() {
        recentThreadsWidgetPersistenceGeneration &+= 1
        let generation = recentThreadsWidgetPersistenceGeneration
        let input = GaryxRecentThreadsWidgetSnapshotInput(
            threads: threads,
            agents: agents,
            pinnedThreadIds: pinnedThreadIds,
            recentThreadIds: recentThreadIds,
            gatewayScopeId: currentGatewayScopeId
        )
        let queue = recentThreadsWidgetPersistenceQueue
        let store = avatarStore
        Task.detached(priority: .utility) {
            await queue.persist(
                input: input,
                generation: generation,
                avatarStore: store,
                validator: GaryxAvatarCGImageValidator()
            )
        }
    }

    func widgetAgentIdentity(for thread: GaryxThreadSummary) -> WidgetAgentIdentity {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread,
            agents: agents
        )
        return WidgetAgentIdentity(
            id: identity.id,
            name: identity.name,
            avatarDataUrl: identity.avatarDataUrl,
            providerType: identity.providerType,
            builtIn: identity.builtIn
        )
    }

    @discardableResult
    func applyThreadTitleUpdate(threadId: String, title: String) -> Bool {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty, !nextTitle.isEmpty else { return false }

        var changed = false
        threads = threads.map { thread in
            guard thread.id == normalizedThreadId, thread.title != nextTitle else {
                return thread
            }
            var updated = thread
            updated.title = nextTitle
            changed = true
            return updated
        }

        if selectedThread?.id == normalizedThreadId,
           selectedThread?.title != nextTitle {
            selectedThread?.title = nextTitle
            draftThreadTitle = nextTitle
            changed = true
        }

        if changed {
            persistRecentThreadsWidgetSnapshot()
        }
        return changed
    }

    func loadMoreThreads(trigger: GaryxThreadListLoadMoreTrigger) async {
        guard hasGatewaySettings,
              let ticket = threadListPager.requestLoadMore(trigger: trigger) else {
            // Rejected triggers are free: nothing is consumed, and the
            // sentinel/footer re-evaluate from pager state (TASK-1802 R4).
            return
        }
        await performLoadMoreThreads(ticket: ticket)
    }

    /// The explicit tap on the failed footer row.
    func retryLoadMoreThreads() async {
        guard hasGatewaySettings,
              let ticket = threadListPager.retryLoadMore() else {
            return
        }
        await performLoadMoreThreads(ticket: ticket)
    }

    private func performLoadMoreThreads(ticket: GaryxThreadListLoadMoreTicket) async {
        do {
            let page = try await client().listRecentThreads(limit: ticket.limit, offset: ticket.offset)
            switch threadListPager.completeLoadMore(
                ticket,
                pageOffset: page.offset,
                pageCount: page.count,
                hasMore: page.hasMore
            ) {
            case .abandonedStaleEpoch:
                // Pager reset mid-flight: the page belongs to the previous
                // gateway and is dropped silently.
                return
            case .abandonedLocalMutation:
                // Archive/delete/pin surgery raced this page (review
                // #TASK-1804 round 4): dedup-append against the
                // post-surgery list would resurrect the removed row as a
                // "new" id. The cursor did not advance; re-request the same
                // window with fresh filters.
                scheduleLoadMoreFollowUpAfterLocalMutation()
                return
            case .apply:
                let pageThreads = pendingThreadArchives.visibleThreads(page.threads)
                let appended = GaryxThreadListPageMerge.appendPage(
                    pageIds: pageThreads.map(\.id),
                    existingIds: recentThreadIds
                )
                if recentThreadIds != appended {
                    recentThreadIds = appended
                }
                threads = Self.mergedThreadSummaries(threads + pageThreads.map(summaryWithCommittedRunState))
                persistRecentThreadsWidgetSnapshot()
            }
        } catch {
            // No global toast: the footer's failed state is the feedback,
            // and the pager's failed gate blocks automatic re-fires
            // (TASK-1802 R5).
            threadListPager.failLoadMore(ticket)
        }
    }

    /// Re-issues an abandoned load-more. Goes through the normal gate; when
    /// the abandoned request was an explicit retry (gate still `.failed`),
    /// the follow-up continues that user intent via `retryLoadMore`.
    private func scheduleLoadMoreFollowUpAfterLocalMutation() {
        Task { [weak self] in
            guard let self else { return }
            guard let ticket = threadListPager.requestLoadMore(trigger: .footer)
                ?? threadListPager.retryLoadMore() else {
                return
            }
            await performLoadMoreThreads(ticket: ticket)
        }
    }

    func refreshWorkspaceAndBotThreads() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let gatewayClient = try client()
            var offset = 0
            var allThreads: [GaryxThreadSummary] = []
            while true {
                let page = try await gatewayClient.listThreads(limit: 1000, offset: offset)
                allThreads += page.threads
                let nextOffset = page.offset + page.count
                if nextOffset >= page.total || page.count == 0 {
                    break
                }
                offset = nextOffset
            }
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let visibleThreads = pendingThreadArchives.visibleThreads(threads)
            let visibleAllThreads = pendingThreadArchives.visibleThreads(allThreads)
            threads = Self.mergedThreadSummaries(
                visibleThreads + visibleAllThreads.map(summaryWithCommittedRunState)
            )
            await mergeMissingSidebarRequiredThreads(
                using: gatewayClient,
                extraThreadIds: [selectedThread?.id],
                runtimeGeneration: runtimeGeneration
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func hydrateCompletedRecentThreadHistories(
        previousThreads: [GaryxThreadSummary],
        previouslyRemoteBusyThreadIds: Set<String>,
        refreshedThreads: [GaryxThreadSummary],
        runtimeGeneration: UUID
    ) {
        guard hasGatewaySettings else { return }
        let previousThreadsById = Dictionary(uniqueKeysWithValues: previousThreads.map { ($0.id, $0) })
        for thread in refreshedThreads {
            let threadId = thread.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !threadId.isEmpty,
                  shouldHydrateCompletedRecentThread(
                    previousThread: previousThreadsById[threadId],
                    previousRemoteBusyThreadIds: previouslyRemoteBusyThreadIds,
                    refreshedThread: thread
                  ) else {
                continue
            }
            hydrateCompletedRecentThreadHistory(
                threadId: threadId,
                runtimeGeneration: runtimeGeneration
            )
        }
    }

    func shouldHydrateCompletedRecentThread(
        previousThread: GaryxThreadSummary?,
        previousRemoteBusyThreadIds: Set<String>,
        refreshedThread: GaryxThreadSummary
    ) -> Bool {
        GaryxCompletedThreadHydrationPolicy.shouldHydrate(
            previousThread: previousThread,
            previousRemoteBusyThreadIds: previousRemoteBusyThreadIds,
            refreshedThread: refreshedThread,
            selectedThreadId: selectedThread?.id
        )
    }

    func hydrateCompletedRecentThreadHistory(threadId: String, runtimeGeneration: UUID) {
        guard completedThreadHistoryHydrationTasks[threadId] == nil else { return }
        completedThreadHistoryHydrationTasks[threadId] = Task { [weak self] in
            guard let self else { return }
            await hydrateCompletedRecentThreadHistoryNow(
                threadId: threadId,
                runtimeGeneration: runtimeGeneration
            )
        }
    }

    @discardableResult
    func hydrateCompletedRecentThreadHistoryNow(threadId: String, runtimeGeneration: UUID) async -> Bool {
        defer {
            completedThreadHistoryHydrationTasks[threadId] = nil
        }
        guard runtimeGeneration == gatewayRuntimeGeneration,
              hasGatewaySettings else {
            return true
        }
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return true }
            let prepared = await Task.detached(priority: .utility) {
                GaryxPreparedThreadTranscriptUpdate.make(from: transcript)
            }.value
            return applyPreparedThreadTranscriptToCache(
                prepared,
                transcript: transcript,
                threadId: threadId,
                preservingLoadedOlderPages: true,
                scheduleRecoveryIfSelected: false
            )
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return true }
            let message = displayMessage(for: error)
            if Self.isTransientGatewayErrorMessage(message) {
                gatewaySettingsStatus = "Waiting to sync with gateway"
            }
            return true
        }
    }

    func startBackgroundCommittedRunReconcileLoop() {
        guard hasGatewaySettings,
              isHomeVisible,
              case .ready = connectionState else {
            cancelBackgroundCommittedRunReconcileLoop()
            return
        }
        guard backgroundCommittedRunReconcileTask == nil else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        backgroundCommittedRunReconcileTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Self.backgroundCommittedRunReconcileIntervalNanos)
                if Task.isCancelled { break }
                await reconcileBackgroundCommittedRunStates(runtimeGeneration: runtimeGeneration)
            }
        }
    }

    func cancelBackgroundCommittedRunReconcileLoop() {
        backgroundCommittedRunReconcileTask?.cancel()
        backgroundCommittedRunReconcileTask = nil
    }

    func reconcileBackgroundCommittedRunStates(runtimeGeneration: UUID) async {
        guard runtimeGeneration == gatewayRuntimeGeneration,
              hasGatewaySettings,
              case .ready = connectionState else {
            cancelBackgroundCommittedRunReconcileLoop()
            return
        }
        let decision = backgroundCommittedRunReconcilePlanner.nextDecision(
            candidateThreadIds: backgroundCommittedRunCandidateThreadIds()
        )
        if decision.refreshesThreads {
            await refreshThreads(source: .backgroundLoop)
        }
        guard decision.hydratesCandidateThreads else { return }

        var observedCompletion = false
        for threadId in decision.candidateThreadIds {
            if Task.isCancelled { break }
            if completedThreadHistoryHydrationTasks[threadId] != nil {
                continue
            }
            let remainedBusy = await hydrateCompletedRecentThreadHistoryNow(
                threadId: threadId,
                runtimeGeneration: runtimeGeneration
            )
            observedCompletion = observedCompletion || !remainedBusy
        }
        guard runtimeGeneration == gatewayRuntimeGeneration else { return }
        if observedCompletion {
            await refreshThreads(source: .backgroundLoop)
        }
    }

    func syncBackgroundCommittedRunReconcileLoopForHomeVisibility() {
        if isHomeVisible {
            startBackgroundCommittedRunReconcileLoop()
        } else {
            cancelBackgroundCommittedRunReconcileLoop()
        }
    }

    func backgroundCommittedRunCandidateThreadIds() -> [String] {
        GaryxBackgroundCommittedRunReconcilePlanner.candidateThreadIds(
            locallyTrackedThreadIds: runTracker.locallyTrackedThreadIds,
            runStateByThread: runStateByThread,
            threads: threads,
            selectedThreadId: selectedThread?.id
        )
    }
}
