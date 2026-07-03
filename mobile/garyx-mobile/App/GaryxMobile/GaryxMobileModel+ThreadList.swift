import Foundation

// Home recent-thread list: refresh and pagination, the widget snapshot
// projection, workspace/bot thread merging, completed-thread history
// hydration, and the background committed-run reconcile loop.
extension GaryxMobileModel {
    func refreshThreads(silent: Bool = false) async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let previousThreadSummaries = Self.mergedThreadSummaries(threads + [selectedThread].compactMap { $0 })
        let previouslyRemoteBusyThreadIds = remoteBusyThreadIds
        if !silent {
            isLoadingThreads = true
        }
        let transactionId = homeProjectionGateway.beginTransaction(label: "refreshThreads")
        defer { homeProjectionGateway.endTransaction(transactionId) }
        defer {
            if !silent, runtimeGeneration == gatewayRuntimeGeneration {
                isLoadingThreads = false
            }
        }
        do {
            let gatewayClient = try client()
            async let threadsPage = gatewayClient.listRecentThreads(limit: Self.threadListPageLimit)
            async let threadPinsPage = gatewayClient.listThreadPins()
            let (page, pinsPage) = try await (threadsPage, threadPinsPage)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let visiblePinnedThreadIds = pendingThreadArchives.visibleThreadIds(pinsPage.threadIds)
            applyPinnedThreadIds(visiblePinnedThreadIds)
            applyRecentThreadsPage(page, preservesLoadedPages: silent)
            var nextThreads = pendingThreadArchives.visibleThreads(page.threads)
            let selectionIdForThisRefresh = selectedThread?.id
            let requiredThreadIds = normalizedThreadIds(visiblePinnedThreadIds + [selectionIdForThisRefresh])
            nextThreads += await fetchMissingThreadSummaries(
                using: gatewayClient,
                requiredThreadIds: requiredThreadIds,
                existingThreadIds: Set(nextThreads.map(\.id))
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let existingThreads = silent ? pendingThreadArchives.visibleThreads(threads) : []
            let previousRuntimeByThreadId = Dictionary(
                uniqueKeysWithValues: previousThreadSummaries.compactMap { thread -> (String, GaryxThreadRuntimeSummary)? in
                    guard let runtime = thread.threadRuntime else { return nil }
                    return (thread.id, runtime)
                }
            )
            let refreshedGatewayThreads = Self.mergedThreadSummaries(nextThreads).map { thread in
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
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
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
        await refreshThreads(silent: true)
    }

    func applyRecentThreadsPage(_ page: GaryxRecentThreadsPage, preservesLoadedPages: Bool) {
        let pageIds = pendingThreadArchives.visibleThreads(page.threads).map(\.id)
        let returnedEnd = page.offset + page.count
        let hasLoadedBeyondHead = preservesLoadedPages
            && (nextThreadListOffset > returnedEnd || recentThreadIds.count > pageIds.count)

        if hasLoadedBeyondHead {
            let pageIdSet = Set(pageIds)
            let existingTail = pendingThreadArchives.visibleThreadIds(
                recentThreadIds.filter { !pageIdSet.contains($0) }
            )
            let merged = pageIds + existingTail
            if recentThreadIds != merged {
                recentThreadIds = merged
            }
            return
        }

        updateThreadListPagination(from: page)
        if recentThreadIds != pageIds {
            recentThreadIds = pageIds
        }
    }

    func persistRecentThreadsWidgetSnapshot() {
        recentThreadsWidgetPersistenceGeneration &+= 1
        let generation = recentThreadsWidgetPersistenceGeneration
        let input = GaryxRecentThreadsWidgetSnapshotInput(
            threads: threads,
            agents: agents,
            teams: teams,
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
            agents: agents,
            teams: teams
        )
        return WidgetAgentIdentity(
            id: identity.id,
            name: identity.name,
            avatarDataUrl: identity.avatarDataUrl,
            providerType: identity.providerType,
            isTeam: identity.isTeam,
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

    func loadMoreThreads() async {
        guard hasGatewaySettings,
              hasMoreThreadSummaries,
              !isLoadingThreads,
              !isLoadingMoreThreads else {
            return
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        let offset = nextThreadListOffset
        guard offset > 0 else { return }
        isLoadingMoreThreads = true
        defer {
            if runtimeGeneration == gatewayRuntimeGeneration {
                isLoadingMoreThreads = false
            }
        }
        do {
            let page = try await client().listRecentThreads(limit: Self.threadListPageLimit, offset: offset)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            updateThreadListPagination(from: page)
            let pageThreads = pendingThreadArchives.visibleThreads(page.threads)
            var seenRecentIds = Set(recentThreadIds)
            recentThreadIds += pageThreads.compactMap { thread in
                seenRecentIds.insert(thread.id).inserted ? thread.id : nil
            }
            threads = Self.mergedThreadSummaries(threads + pageThreads.map(summaryWithCommittedRunState))
            persistRecentThreadsWidgetSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func updateThreadListPagination(from page: GaryxThreadsPage) {
        let returnedEnd = page.offset + page.count
        nextThreadListOffset = returnedEnd
        let hasMore = returnedEnd < page.total
        if hasMoreThreadSummaries != hasMore {
            hasMoreThreadSummaries = hasMore
        }
    }

    func updateThreadListPagination(from page: GaryxRecentThreadsPage) {
        nextThreadListOffset = page.offset + page.count
        if hasMoreThreadSummaries != page.hasMore {
            hasMoreThreadSummaries = page.hasMore
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
            await refreshThreads(silent: true)
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
            await refreshThreads(silent: true)
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
