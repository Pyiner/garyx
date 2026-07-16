import Foundation

private struct GaryxFetchedRecentRefresh {
    var bundle: GaryxRecentThreadRefreshBundle
    var threads: [GaryxThreadSummary]
}

private struct GaryxRecentIdentityInterrupted: Error {}

// Home recent-thread list: refresh and pagination, the widget snapshot
// projection, workspace/bot thread merging, completed-thread history
// hydration, and the background committed-run reconcile loop.
extension GaryxMobileModel {
    func selectRecentThreadFilter(_ filter: GaryxRecentThreadFilter) {
        guard recentThreadFeeds.selectedFilter != filter else { return }
        recentThreadFeeds.select(filter)
        GaryxRecentThreadFilterStorage.save(
            filter,
            defaults: defaults,
            key: GaryxMobileSettingsKeys.recentThreadFilter
        )
        Task { [weak self] in
            await self?.refreshThreads(source: .userAction)
        }
    }

    func refreshThreads(
        source: GaryxThreadListRefreshSource,
        forceReplacement: Bool = false
    ) async {
        guard hasGatewaySettings else { return }
        servicePinnedOrderRetry(source: source)
        refreshThreadFavoritesSnapshot()
        let gatewayScope = threadFavoritesState.gatewayScope
        let favoritesEpoch = threadFavoritesState.runtimeEpoch
        guard let ticket = recentThreadFeeds.requestRefresh(
            gatewayScope: gatewayScope,
            runtimeEpoch: favoritesEpoch,
            forceReplacement: forceReplacement || source == .userPullToRefresh
        ) else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        if ticket.filter == .nonTask {
            startAuxiliaryAllRecentThreadsRefresh(source: source)
        }
        let previousThreadSummaries = Self.mergedThreadSummaries(threads + [selectedThread].compactMap { $0 })
        let previouslyRemoteBusyThreadIds = remoteBusyThreadIds
        let transactionId = homeProjectionGateway.beginTransaction(label: "refreshThreads")
        defer { homeProjectionGateway.endTransaction(transactionId) }
        do {
            let gatewayClient = try client()
            let pinsRequestStamp = capturePinnedOrderRequestStamp()
            async let recentRefresh = fetchRecentRefresh(
                ticket: ticket,
                gatewayClient: gatewayClient
            )
            async let threadPinsPage = gatewayClient.listThreadPins()
            let (fetchedRefresh, pinsPage) = try await (recentRefresh, threadPinsPage)
            guard runtimeGeneration == gatewayRuntimeGeneration else {
                recentThreadFeeds.failRefresh(ticket)
                return
            }
            var fetchedThreads = pendingThreadArchives.visibleThreads(fetchedRefresh.threads)
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
                recentThreadFeeds.failRefresh(ticket)
                return
            }
            // Transaction commit point: the ticket completes only after the
            // LAST await, so the pager's refresh gate spans the entire
            // App-layer refresh — a second refresh cannot interleave between
            // this page landing and its state writes and then be overwritten
            // by this (older) page when we resume (review #TASK-1804).
            switch recentThreadFeeds.completeRefresh(
                ticket,
                bundle: fetchedRefresh.bundle
            ) {
            case .abandonedStaleEpoch:
                // The pager was reset mid-flight: the page belongs to the
                // previous gateway and is dropped silently.
                return
            case .abandonedLocalMutation:
                // Archive/delete/pin surgery raced this refresh
                // (review #TASK-1804 round 3): every pre-await snapshot is
                // stale. Drop the page and follow up with a fresh refresh,
                // which also replaces the surgery-triggered refresh this one
                // coalesced away.
                Task { [weak self] in
                    await self?.refreshThreads(source: source)
                }
                return
            case .forceReplacement:
                Task { [weak self] in
                    await self?.refreshThreads(
                        source: source,
                        forceReplacement: true
                    )
                }
                return
            case .applied:
                commitRefreshedRecentThreadsPage(
                    pinsPageThreadIds: pinsPage.threadIds,
                    fetchedThreads: fetchedThreads,
                    previousThreadSummaries: previousThreadSummaries,
                    previouslyRemoteBusyThreadIds: previouslyRemoteBusyThreadIds,
                    selectionIdForThisRefresh: selectionIdForThisRefresh,
                    runtimeGeneration: runtimeGeneration,
                    pinsRevision: pinsPage.revision,
                    pinsRequestStamp: pinsRequestStamp
                )
            }
        } catch is GaryxRecentIdentityInterrupted {
            return
        } catch {
            recentThreadFeeds.failRefresh(ticket)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if recentThreadFeeds.selectedFilter == ticket.filter {
                presentThreadListRefreshFailure(source: source, error: error)
            }
        }
    }

    private func startAuxiliaryAllRecentThreadsRefresh(source: GaryxThreadListRefreshSource) {
        guard let ticket = recentThreadFeeds.requestRefresh(
            filter: .all,
            gatewayScope: threadFavoritesState.gatewayScope,
            runtimeEpoch: threadFavoritesState.runtimeEpoch
        ) else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let taskId = UUID()
        auxiliaryAllRecentThreadsRefreshTaskId = taskId
        let task = Task { [weak self] in
            guard let self else { return }
            await self.performAuxiliaryAllRecentThreadsRefresh(
                ticket: ticket,
                source: source,
                runtimeGeneration: runtimeGeneration
            )
            if self.auxiliaryAllRecentThreadsRefreshTaskId == taskId {
                self.auxiliaryAllRecentThreadsRefreshTask = nil
                self.auxiliaryAllRecentThreadsRefreshTaskId = nil
            }
        }
        auxiliaryAllRecentThreadsRefreshTask = task
    }

    private func performAuxiliaryAllRecentThreadsRefresh(
        ticket: GaryxRecentThreadRefreshTicket,
        source: GaryxThreadListRefreshSource,
        runtimeGeneration: UUID
    ) async {
        do {
            let fetched = try await fetchRecentRefresh(
                ticket: ticket,
                gatewayClient: client()
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else {
                recentThreadFeeds.failRefresh(ticket)
                return
            }
            let pageThreads = pendingThreadArchives.visibleThreads(fetched.threads)
            // Feed ids and their shared summaries are one projection commit.
            // This is normally inactive while Chats is selected, but the user
            // can switch to All while the auxiliary request is in flight.
            let transactionId = homeProjectionGateway.beginTransaction(
                label: "auxiliary-all-recent-threads-refresh"
            )
            defer { homeProjectionGateway.endTransaction(transactionId) }
            switch recentThreadFeeds.completeRefresh(
                ticket,
                bundle: fetched.bundle
            ) {
            case .applied:
                threads = Self.mergedThreadSummaries(
                    pendingThreadArchives.visibleThreads(threads)
                        + pageThreads.map(summaryWithCommittedRunState)
                )
                persistRecentThreadsWidgetSnapshot()
            case .abandonedLocalMutation:
                // The replacement ticket belongs to the runtime that exists
                // now, not the generation captured by the abandoned request.
                startAuxiliaryAllRecentThreadsRefresh(source: source)
            case .abandonedStaleEpoch:
                return
            case .forceReplacement:
                startAuxiliaryAllRecentThreadsRefresh(source: source)
            }
        } catch is GaryxRecentIdentityInterrupted {
            return
        } catch {
            recentThreadFeeds.failRefresh(ticket)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            // Auxiliary All failures remain silent while Chats is selected.
            // If the user switched to All while the coalesced request was in
            // flight, the request now owns the visible feed and follows the
            // original trigger's normal toast/status policy.
            if recentThreadFeeds.selectedFilter == ticket.filter {
                presentThreadListRefreshFailure(source: source, error: error)
            }
        }
    }

    private func fetchRecentRefresh(
        ticket: GaryxRecentThreadRefreshTicket,
        gatewayClient: GaryxGatewayClient
    ) async throws -> GaryxFetchedRecentRefresh {
        let primary = try await fetchRecentChain(
            ticket: ticket,
            mode: ticket.mode,
            oldHeadActivitySeq: ticket.oldHeadActivitySeq,
            gatewayClient: gatewayClient
        )
        let verification = try await fetchRecentPage(
            ticket: ticket,
            cursor: nil,
            gatewayClient: gatewayClient
        )
        let primaryHead = primary.pages.first?.headActivitySeq
        var immediate: (pages: [GaryxRecentThreadFeedPage], threads: [GaryxThreadSummary])?
        var immediateVerification: GaryxRecentThreadFeedPage?
        if GaryxRecentThreadRangeFill.verificationObservedNewerHead(
            chainFirstHead: primaryHead,
            verificationPage: verification.feedPage
        ) {
            immediate = try await fetchRecentChain(
                ticket: ticket,
                mode: .rangeFill,
                oldHeadActivitySeq: primaryHead,
                gatewayClient: gatewayClient
            )
            immediateVerification = try await fetchRecentPage(
                ticket: ticket,
                cursor: nil,
                gatewayClient: gatewayClient
            ).feedPage
        }
        return GaryxFetchedRecentRefresh(
            bundle: GaryxRecentThreadRefreshBundle(
                primaryPages: primary.pages,
                verificationPage: verification.feedPage,
                immediatePages: immediate?.pages,
                immediateVerificationPage: immediateVerification
            ),
            threads: primary.threads + (immediate?.threads ?? [])
        )
    }

    private func fetchRecentChain(
        ticket: GaryxRecentThreadRefreshTicket,
        mode: GaryxRecentThreadRefreshMode,
        oldHeadActivitySeq: Int64?,
        gatewayClient: GaryxGatewayClient
    ) async throws -> (pages: [GaryxRecentThreadFeedPage], threads: [GaryxThreadSummary]) {
        var pages: [GaryxRecentThreadFeedPage] = []
        var threads: [GaryxThreadSummary] = []
        var cursor: String?
        repeat {
            let fetched = try await fetchRecentPage(
                ticket: ticket,
                cursor: cursor,
                gatewayClient: gatewayClient
            )
            pages.append(fetched.feedPage)
            threads += fetched.threads
            cursor = fetched.nextCursor
        } while GaryxRecentThreadRangeFill.needsNextPage(
            mode: mode,
            oldHeadActivitySeq: oldHeadActivitySeq,
            pages: pages
        )
        return (pages, threads)
    }

    private func fetchRecentPage(
        ticket: GaryxRecentThreadRefreshTicket,
        cursor: String?,
        gatewayClient: GaryxGatewayClient
    ) async throws -> (
        feedPage: GaryxRecentThreadFeedPage,
        threads: [GaryxThreadSummary],
        nextCursor: String?
    ) {
        let page = try await gatewayClient.listRecentThreads(
            filter: ticket.filter,
            limit: Self.threadListPageLimit,
            cursor: cursor
        )
        let ownedFeed = recentThreadFeeds.feed(for: ticket.filter)
        let decision = observeThreadStoreIdentity(
            gatewayScope: ticket.gatewayScope,
            runtimeEpoch: ticket.runtimeEpoch,
            owned: ownedFeed.pager.epoch == ticket.pagerTicket.epoch
                && ownedFeed.pager.isRefreshingHead,
            storeIncarnationId: page.storeIncarnationId
        )
        guard decision == .accept else { throw GaryxRecentIdentityInterrupted() }
        return (GaryxRecentThreadFeedPage(page), page.threads, page.nextCursor)
    }

    /// Archive/delete ambiguity keeps today's rollback/error UX, then calls
    /// this reconstruction path. A commit before either replacement snapshot
    /// disappears now; a later commit is picked up by the next M=30/foreground
    /// replacement cycle.
    func forceReplaceThreadFeedsAfterAmbiguousLifecycle() async {
        recentThreadFeeds.forceReplacement()
        // refreshThreads owns the favorites snapshot trigger as part of every
        // head replacement. Do not enqueue a duplicate trailing snapshot for
        // the same lifecycle reconstruction.
        let selected = recentThreadFeeds.selectedFilter
        await refreshThreads(source: .userAction, forceReplacement: true)
        let other: GaryxRecentThreadFilter = selected == .all ? .nonTask : .all
        await performAuxiliaryRecentReplacement(filter: other)
    }

    private func performAuxiliaryRecentReplacement(
        filter: GaryxRecentThreadFilter
    ) async {
        guard hasGatewaySettings,
              let ticket = recentThreadFeeds.requestRefresh(
                  filter: filter,
                  gatewayScope: threadFavoritesState.gatewayScope,
                  runtimeEpoch: threadFavoritesState.runtimeEpoch,
                  forceReplacement: true
              ) else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let fetched = try await fetchRecentRefresh(
                ticket: ticket,
                gatewayClient: client()
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else {
                recentThreadFeeds.failRefresh(ticket)
                return
            }
            switch recentThreadFeeds.completeRefresh(ticket, bundle: fetched.bundle) {
            case .applied:
                threads = Self.mergedThreadSummaries(
                    pendingThreadArchives.visibleThreads(threads)
                        + pendingThreadArchives.visibleThreads(fetched.threads)
                            .map(summaryWithCommittedRunState)
                )
                persistRecentThreadsWidgetSnapshot()
            case .forceReplacement:
                // Keep the pending bit; the next periodic/foreground cycle
                // retries without spinning recursively on a changing boot.
                return
            case .abandonedLocalMutation, .abandonedStaleEpoch:
                return
            }
        } catch is GaryxRecentIdentityInterrupted {
            return
        } catch {
            recentThreadFeeds.failRefresh(ticket)
        }
    }

    /// Synchronous commit of a completed head-refresh transaction. Runs
    /// entirely on the MainActor after the refresh's last await, so all
    /// inputs captured before the backfill are re-filtered here against the
    /// committed tombstones in `pendingThreadArchives`: a thread archived
    /// while the backfill was suspended must not be resurrected by pre-await
    /// snapshots (review #TASK-1804 round 2).
    func commitRefreshedRecentThreadsPage(
        pinsPageThreadIds: [String],
        fetchedThreads: [GaryxThreadSummary],
        previousThreadSummaries: [GaryxThreadSummary],
        previouslyRemoteBusyThreadIds: Set<String>,
        selectionIdForThisRefresh: String?,
        runtimeGeneration: UUID,
        pinsRevision: Int64 = 0,
        pinsRequestStamp: GaryxPinnedOrderRequestStamp? = nil
    ) {
        applyPinnedThreadIds(
            pendingThreadArchives.visibleThreadIds(pinsPageThreadIds),
            revision: pinsRevision,
            stamp: pinsRequestStamp
        )
        let visibleFetchedThreads = pendingThreadArchives.visibleThreads(fetchedThreads)
        // Loaded tail summaries always survive a head refresh; which
        // rows are visible is the selected Recent feed's concern.
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

    func persistRecentThreadsWidgetSnapshot() {
        recentThreadsWidgetPersistenceGeneration &+= 1
        let generation = recentThreadsWidgetPersistenceGeneration
        let input = GaryxRecentThreadsWidgetSnapshotInput(
            threads: threads,
            agents: agents,
            pinnedThreadIds: pinnedThreadIds,
            recentThreadIds: allRecentThreadIds,
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
              let ticket = recentThreadFeeds.requestLoadMore(
                  trigger: trigger,
                  gatewayScope: threadFavoritesState.gatewayScope,
                  runtimeEpoch: threadFavoritesState.runtimeEpoch
              ) else {
            // Rejected triggers are free: nothing is consumed, and the
            // sentinel/footer re-evaluate from pager state (TASK-1802 R4).
            return
        }
        await performLoadMoreThreads(ticket: ticket)
    }

    /// The explicit tap on the failed footer row.
    func retryLoadMoreThreads() async {
        guard hasGatewaySettings,
              let ticket = recentThreadFeeds.retryLoadMore(
                  gatewayScope: threadFavoritesState.gatewayScope,
                  runtimeEpoch: threadFavoritesState.runtimeEpoch
              ) else {
            return
        }
        await performLoadMoreThreads(ticket: ticket)
    }

    private func performLoadMoreThreads(ticket: GaryxRecentThreadLoadMoreTicket) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().listRecentThreads(
                filter: ticket.filter,
                limit: ticket.limit,
                cursor: ticket.cursor
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else {
                recentThreadFeeds.failLoadMore(ticket)
                return
            }
            let pageThreads = pendingThreadArchives.visibleThreads(page.threads)
            let ownedFeed = recentThreadFeeds.feed(for: ticket.filter)
            let identity = observeThreadStoreIdentity(
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                owned: ownedFeed.pager.epoch == ticket.pagerTicket.epoch
                    && ownedFeed.pager.isLoadingMore,
                storeIncarnationId: page.storeIncarnationId
            )
            guard identity == .accept else { return }
            switch recentThreadFeeds.completeLoadMore(
                ticket,
                page: GaryxRecentThreadFeedPage(page)
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
            case .forceReplacement:
                await forceReplaceThreadFeedsAfterAmbiguousLifecycle()
                return
            case .applied:
                threads = Self.mergedThreadSummaries(threads + pageThreads.map(summaryWithCommittedRunState))
                persistRecentThreadsWidgetSnapshot()
            }
        } catch {
            // No global toast: the footer's failed state is the feedback,
            // and the pager's failed gate blocks automatic re-fires
            // (TASK-1802 R5).
            recentThreadFeeds.failLoadMore(ticket)
        }
    }

    /// Re-issues an abandoned load-more. Goes through the normal gate; when
    /// the abandoned request was an explicit retry (gate still `.failed`),
    /// the follow-up continues that user intent via `retryLoadMore`.
    private func scheduleLoadMoreFollowUpAfterLocalMutation() {
        Task { [weak self] in
            guard let self else { return }
            guard let ticket = recentThreadFeeds.requestLoadMore(
                trigger: .footer,
                gatewayScope: threadFavoritesState.gatewayScope,
                runtimeEpoch: threadFavoritesState.runtimeEpoch
            )
                ?? recentThreadFeeds.retryLoadMore(
                    gatewayScope: threadFavoritesState.gatewayScope,
                    runtimeEpoch: threadFavoritesState.runtimeEpoch
                ) else {
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
