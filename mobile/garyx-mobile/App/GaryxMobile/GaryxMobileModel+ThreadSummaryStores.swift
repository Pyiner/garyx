import Foundation

// Thread-list ownership bridge. Membership stays in the S2 providers while
// every row body is resolved through `threadSummaryCache`.
extension GaryxMobileModel {
    var homeThreadSummaries: [GaryxThreadSummary] {
        threadSummaryCache.summaries(
            for: normalizedThreadIds(
                pinnedThreadIds.map(Optional.some)
                    + visibleRecentThreadIds.map(Optional.some)
                    + [selectedThread?.id]
            )
        )
    }

    var residentRecentThreadSummaries: [GaryxThreadSummary] {
        threadSummaryCache.summaries(
            for: normalizedThreadIds(
                pinnedThreadIds.map(Optional.some)
                    + allRecentThreadIds.map(Optional.some)
                    + threadFavoritesProvider.snapshot.orderedThreadIds.map(Optional.some)
                    + [selectedThread?.id]
            )
        )
    }

    func cachedThreadSummary(for threadId: String) -> GaryxThreadSummary? {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return nil }
        if selectedThread?.id == normalizedId {
            return selectedThread
        }
        return threadSummaryCache.summary(for: normalizedId)
    }

    func liveThreadRowCapabilities(for thread: GaryxThreadSummary) -> GaryxThreadRowCapabilities {
        GaryxThreadRowCapabilityDeriver.capabilities(
            for: thread,
            context: GaryxThreadRowCapabilityContext(
                automationTargetThreadIds: Set(automations.compactMap { automation in
                    (automation.targetThreadId ?? "").garyxTrimmedNilIfEmpty
                }),
                hasActiveRun: remoteBusyThreadIds.contains(thread.id)
            )
        )
    }

    func cacheThreadSummaries(
        _ summaries: [GaryxThreadSummary],
        publish: Bool = true
    ) {
        let visible = pendingThreadArchives.visibleThreads(summaries)
            .map(summaryWithCommittedRunState)
        threadSummaryCache.writeThrough(visible)
        refreshRecentThreadLeases(summaryWrites: visible)
        if publish {
            publishThreadSummaryState()
        }
    }

    func refreshRecentThreadLeases(summaryWrites: [GaryxThreadSummary] = []) {
        let visiblePinnedIds = pendingThreadArchives.visibleThreadIds(pinnedThreadIds)
        let allIds = pendingThreadArchives.visibleThreadIds(
            recentThreadFeeds.feed(for: .all)?.orderedThreadIds ?? []
        )
        let chatIds = pendingThreadArchives.visibleThreadIds(
            recentThreadFeeds.feed(for: .nonTask)?.orderedThreadIds ?? []
        )
        threadSummaryLeaseOwner.replaceFeed(
            ownerId: "recent-all",
            threadIds: visiblePinnedIds + allIds,
            summaries: summaryWrites
        )
        threadSummaryLeaseOwner.replaceFeed(
            ownerId: "recent-chats",
            threadIds: visiblePinnedIds + chatIds,
            summaries: summaryWrites
        )
    }

    func publishThreadSummaryState() {
        registerResidentRecentFeeds()
        refreshHomeObservationPaginationSnapshot()
        emitHomeProjectionSnapshot()
        refreshNavigationDrawerSnapshot()
        refreshResidentThreadListStores()
    }

    private func registerResidentRecentFeeds() {
        let all = threadFeedRegistry.activate(.recentAll)
        threadMutationHubStore.value.registerStore(
            storeId: "recent:all",
            instanceId: all.instanceId,
            orderedThreadIds: recentThreadFeeds.allFeed.orderedThreadIds
        )
        let chats = threadFeedRegistry.activate(.recentChats)
        threadMutationHubStore.value.registerStore(
            storeId: "recent:non_task",
            instanceId: chats.instanceId,
            orderedThreadIds: recentThreadFeeds.nonTaskFeed.orderedThreadIds
        )
    }

    func removeCachedThreadSummary(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        refreshRecentThreadLeases()
        threadSummaryCache.remove(normalizedId)
        publishThreadSummaryState()
    }

    func applyThreadMutationAuthorityToResidentProviders(
        _ authority: GaryxThreadMutationAuthority
    ) {
        if let summary = authority.summary {
            threadSummaryCache.writeThrough([summaryWithCommittedRunState(summary)])
        }
        switch authority.membership {
        case .unchanged:
            break
        case .remove(let threadId):
            recentThreadFeeds.removeThread(threadId)
        case .upsertAtHead(let threadId):
            recentThreadFeeds.upsertChat(threadId: threadId)
        case .replace:
            // A hub replacement is committed into each provider below; the
            // two permanent Recent feeds retain their own pager barriers.
            break
        }

        for path in Array(workspaceThreadProviders.keys) {
            guard var provider = workspaceThreadProviders[path] else { continue }
            provider.apply(authority.membership, summary: authority.summary)
            workspaceThreadProviders[path] = provider
        }
        for automationId in Array(automationThreadProviders.keys) {
            guard var provider = automationThreadProviders[automationId] else { continue }
            if case .remove = authority.membership {
                provider.apply(authority.membership)
            }
            automationThreadProviders[automationId] = provider
        }
        for groupId in Array(botThreadProviders.keys) {
            guard var provider = botThreadProviders[groupId] else { continue }
            if case .remove = authority.membership {
                provider.apply(authority.membership)
            }
            botThreadProviders[groupId] = provider
            threadSummaryLeaseOwner.replaceBotEntries(
                groupId: groupId,
                threadIds: provider.snapshot.orderedThreadIds,
                summaries: threadSummaryCache.summaries(
                    for: provider.snapshot.orderedThreadIds
                )
            )
        }
        refreshRecentThreadLeases()
        if case .remove(let threadId) = authority.membership {
            // Release resident page/bot pins before removing the truth row.
            // `remove` intentionally refuses to evict a still-owned summary.
            refreshResidentThreadListStores()
            threadSummaryCache.remove(threadId)
        }
        publishThreadSummaryState()
    }

    func refreshResidentThreadListStores() {
        let favorites = Set(threadFavoritesState.presentedThreadIds)
        let pinned = Set(pinnedThreadIds)
        let active = remoteBusyThreadIds
        let automationTargets = Set(automations.compactMap { automation -> String? in
            let id = (automation.targetThreadId ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return id.isEmpty ? nil : id
        })

        for (path, provider) in workspaceThreadProviders {
            guard let store = workspaceThreadStores[path] else { continue }
            commitThreadListStore(
                store,
                snapshot: provider.snapshot,
                favoriteThreadIds: favorites,
                pinnedThreadIds: pinned,
                automationTargetThreadIds: automationTargets,
                activeRunThreadIds: active
            )
        }
        for (automationId, provider) in automationThreadProviders {
            guard let store = automationThreadStores[automationId] else { continue }
            commitThreadListStore(
                store,
                snapshot: provider.snapshot,
                favoriteThreadIds: favorites,
                pinnedThreadIds: pinned,
                automationTargetThreadIds: automationTargets,
                activeRunThreadIds: active
            )
        }
        for (groupId, provider) in botThreadProviders {
            guard let store = botThreadStores[groupId] else { continue }
            let entries = Dictionary(uniqueKeysWithValues: provider.entries.map { ($0.threadId, $0) })
            commitThreadListStore(
                store,
                snapshot: provider.snapshot,
                favoriteThreadIds: favorites,
                pinnedThreadIds: pinned,
                automationTargetThreadIds: automationTargets,
                activeRunThreadIds: active,
                botEntries: entries
            )
        }
    }

    private func commitThreadListStore(
        _ store: GaryxThreadListStore,
        snapshot: GaryxThreadListMembershipSnapshot,
        summaryWrites: [GaryxThreadSummary] = [],
        botEntries: [String: GaryxBotConversationMembershipEntry] = [:]
    ) {
        commitThreadListStore(
            store,
            snapshot: snapshot,
            summaryWrites: summaryWrites,
            favoriteThreadIds: Set(threadFavoritesState.presentedThreadIds),
            pinnedThreadIds: Set(pinnedThreadIds),
            automationTargetThreadIds: Set(automations.compactMap { automation in
                (automation.targetThreadId ?? "").garyxTrimmedNilIfEmpty
            }),
            activeRunThreadIds: remoteBusyThreadIds,
            botEntries: botEntries
        )
    }

    private func commitThreadListStore(
        _ store: GaryxThreadListStore,
        snapshot: GaryxThreadListMembershipSnapshot,
        summaryWrites: [GaryxThreadSummary] = [],
        favoriteThreadIds: Set<String>,
        pinnedThreadIds: Set<String>,
        automationTargetThreadIds: Set<String>,
        activeRunThreadIds: Set<String>,
        botEntries: [String: GaryxBotConversationMembershipEntry] = [:]
    ) {
        let storeId = threadListStoreId(snapshot.identity)
        _ = store.commit(
            GaryxThreadListMembershipCommit(
                snapshot: snapshot,
                summaryWrites: summaryWrites
            ),
            favoriteThreadIds: favoriteThreadIds,
            pinnedStateThreadIds: pinnedThreadIds,
            selectedThreadId: selectedThread?.id,
            automationTargetThreadIds: automationTargetThreadIds,
            activeRunThreadIds: activeRunThreadIds,
            pendingMutations: threadMutationHubStore.value.residents[storeId]?.pending ?? [:],
            botEntries: botEntries
        )
        threadMutationHubStore.value.registerStore(
            storeId: storeId,
            instanceId: snapshot.identity.instanceId,
            orderedThreadIds: snapshot.orderedThreadIds
        )
    }

    private func threadListStoreId(_ identity: GaryxThreadListProviderIdentity) -> String {
        switch identity.kind {
        case .recent(let filter): return "recent:\(filter.rawValue)"
        case .workspace(let path): return "workspace:\(path)"
        case .botConversations(let groupId): return "bot:\(groupId)"
        case .automationThreads(let automationId): return "automation:\(automationId)"
        case .favorites: return "favorites"
        case .picker: return "picker:\(identity.instanceId)"
        }
    }

    func workspaceThreadListStore(path rawPath: String) -> GaryxThreadListStore {
        let path = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
        if let store = workspaceThreadStores[path] {
            _ = threadFeedRegistry.activateWorkspace(path)
            return store
        }
        let activation = threadFeedRegistry.activateWorkspace(path)
        for evicted in activation.evicted {
            guard case .workspace(let evictedPath) = evicted.key else { continue }
            workspaceThreadStores[evictedPath]?.resetGatewayScope()
            workspaceThreadStores[evictedPath] = nil
            workspaceThreadProviders[evictedPath] = nil
            threadMutationHubStore.value.evictStore(
                storeId: "workspace:\(evictedPath)",
                instanceId: evicted.instanceId
            )
        }
        let provider = GaryxThreadSummaryMembershipProvider(
            scope: .workspace(path: path),
            pageLimit: Self.threadListPageLimit,
            overlap: Self.threadListPageOverlap,
            instanceId: activation.instanceId
        )
        let store = GaryxThreadListStore(
            ownerId: "workspace:\(path)",
            cache: threadSummaryCache,
            leaseOwner: threadSummaryLeaseOwner
        )
        workspaceThreadProviders[path] = provider
        workspaceThreadStores[path] = store
        commitThreadListStore(store, snapshot: provider.snapshot)
        return store
    }

    func refreshWorkspaceThreadList(path rawPath: String) async {
        let path = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty, hasGatewaySettings else { return }
        let store = workspaceThreadListStore(path: path)
        #if DEBUG
        if debugSnapshotActive, store.snapshot.isPrimed {
            return
        }
        #endif
        let resolution = await resolveThreadSummaryCapability()
        guard !Task.isCancelled else { return }
        switch resolution.state {
        case .unsupported:
            store.setAvailability(.unsupportedGateway)
            return
        case .unknown where resolution.probeFailed:
            store.setAvailability(.failed(message: "Could not reach the gateway."))
            return
        case .unknown, .supported:
            break
        }
        guard var provider = workspaceThreadProviders[path],
              let ticket = provider.requestRefresh() else { return }
        workspaceThreadProviders[path] = provider
        store.setAvailability(.ready)
        commitThreadListStore(store, snapshot: provider.snapshot)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().listThreadSummaries(
                workspaceDir: ticket.workspacePath,
                limit: provider.pager.pageLimit
            )
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = workspaceThreadProviders[path] else { return }
            let completion = owned.completeRefresh(ticket, page: page)
            workspaceThreadProviders[path] = owned
            applyWorkspaceCompletion(completion, path: path, store: store)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = workspaceThreadProviders[path] else { return }
            owned.failRefresh(ticket)
            workspaceThreadProviders[path] = owned
            commitThreadListStore(store, snapshot: owned.snapshot)
            store.setAvailability(
                owned.snapshot.isPrimed
                    ? .ready
                    : .failed(message: displayMessage(for: error))
            )
        }
    }

    func loadMoreWorkspaceThreadList(
        path rawPath: String,
        trigger: GaryxThreadListLoadMoreTrigger,
        retryingFailure: Bool = false
    ) async {
        let path = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var provider = workspaceThreadProviders[path],
              let store = workspaceThreadStores[path] else { return }
        let ticket = retryingFailure
            ? provider.retryLoadMore()
            : provider.requestLoadMore(trigger: trigger)
        guard let ticket else { return }
        workspaceThreadProviders[path] = provider
        commitThreadListStore(store, snapshot: provider.snapshot)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().listThreadSummaries(
                workspaceDir: ticket.workspacePath,
                query: ticket.query,
                limit: provider.pager.pageLimit,
                cursor: ticket.cursor
            )
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = workspaceThreadProviders[path] else { return }
            let completion = owned.completeLoadMore(ticket, page: page)
            workspaceThreadProviders[path] = owned
            applyWorkspaceCompletion(completion, path: path, store: store)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = workspaceThreadProviders[path] else { return }
            owned.failLoadMore(ticket)
            workspaceThreadProviders[path] = owned
            commitThreadListStore(store, snapshot: owned.snapshot)
        }
    }

    func retryLoadMoreWorkspaceThreadList(path: String) async {
        await loadMoreWorkspaceThreadList(
            path: path,
            trigger: .footer,
            retryingFailure: true
        )
    }

    private func applyWorkspaceCompletion(
        _ completion: GaryxThreadListProviderCompletion,
        path: String,
        store: GaryxThreadListStore
    ) {
        switch completion {
        case .accepted(let commit):
            commitThreadListStore(
                store,
                snapshot: commit.snapshot,
                summaryWrites: commit.summaryWrites
            )
            store.setAvailability(.ready)
            publishThreadSummaryState()
        case .replacementRequired:
            Task { await refreshWorkspaceThreadList(path: path) }
        case .rejectedStaleInstance:
            break
        }
    }

    func refreshThreadPicker(
        _ owner: GaryxThreadPickerMembershipOwner
    ) async {
        guard hasGatewaySettings else {
            owner.setAvailability(.failed(message: "Gateway is not configured."))
            return
        }
        let resolution = await resolveThreadSummaryCapability()
        guard !Task.isCancelled else { return }
        switch resolution.state {
        case .unsupported:
            owner.presentUnsupportedFallback(allRecentThreads)
            return
        case .unknown where resolution.probeFailed:
            owner.setAvailability(.failed(message: "Could not reach the gateway."))
            return
        case .unknown:
            return
        case .supported:
            break
        }
        guard let ticket = owner.requestRefresh() else { return }
        owner.setAvailability(.ready)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().listThreadSummaries(
                query: ticket.query,
                limit: owner.provider.pager.pageLimit
            )
            guard !Task.isCancelled,
                  runtimeGeneration == gatewayRuntimeGeneration else { return }
            switch owner.completeRefresh(ticket, page: page) {
            case .accepted:
                publishThreadSummaryState()
            case .replacementRequired:
                await refreshThreadPicker(owner)
            case .rejectedStaleInstance:
                break
            }
        } catch {
            guard !Task.isCancelled,
                  runtimeGeneration == gatewayRuntimeGeneration else { return }
            owner.failRefresh(ticket)
            owner.setAvailability(.failed(message: displayMessage(for: error)))
        }
    }

    func loadMoreThreadPicker(
        _ owner: GaryxThreadPickerMembershipOwner,
        trigger: GaryxThreadListLoadMoreTrigger,
        retryingFailure: Bool = false
    ) async {
        guard owner.availability == .ready else { return }
        let ticket = retryingFailure
            ? owner.retryLoadMore()
            : owner.requestLoadMore(trigger: trigger)
        guard let ticket else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().listThreadSummaries(
                query: ticket.query,
                limit: owner.provider.pager.pageLimit,
                cursor: ticket.cursor
            )
            guard !Task.isCancelled,
                  runtimeGeneration == gatewayRuntimeGeneration else { return }
            switch owner.completeLoadMore(ticket, page: page) {
            case .accepted:
                publishThreadSummaryState()
            case .replacementRequired:
                await refreshThreadPicker(owner)
            case .rejectedStaleInstance:
                break
            }
        } catch {
            guard !Task.isCancelled,
                  runtimeGeneration == gatewayRuntimeGeneration else { return }
            owner.failLoadMore(ticket)
        }
    }

    func retryLoadMoreThreadPicker(_ owner: GaryxThreadPickerMembershipOwner) async {
        await loadMoreThreadPicker(owner, trigger: .footer, retryingFailure: true)
    }

    func hydrateThreadPickerSelectedTarget(
        _ owner: GaryxThreadPickerMembershipOwner,
        threadId rawThreadId: String
    ) async -> GaryxThreadSummary? {
        let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else {
            owner.swapSelectedTarget(nil)
            return nil
        }
        if let cached = cachedThreadSummary(for: threadId) {
            owner.swapSelectedTarget(cached)
            return cached
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let summary = try await client().getThread(threadId: threadId)
            guard !Task.isCancelled,
                  runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            owner.swapSelectedTarget(summary)
            publishThreadSummaryState()
            return summary
        } catch {
            guard !Task.isCancelled else { return nil }
            owner.swapSelectedTarget(nil)
            return nil
        }
    }

    func automationThreadListStore(automationId: String) -> GaryxThreadListStore {
        if let store = automationThreadStores[automationId] { return store }
        let provider = GaryxAutomationThreadMembershipProvider(automationId: automationId)
        let store = GaryxThreadListStore(
            ownerId: "automation:\(automationId)",
            cache: threadSummaryCache,
            leaseOwner: threadSummaryLeaseOwner
        )
        automationThreadProviders[automationId] = provider
        automationThreadStores[automationId] = store
        commitThreadListStore(store, snapshot: provider.snapshot)
        return store
    }

    func refreshAutomationThreadList(automationId: String) async {
        let store = automationThreadListStore(automationId: automationId)
        guard var provider = automationThreadProviders[automationId],
              let ticket = provider.requestRefresh() else { return }
        automationThreadProviders[automationId] = provider
        commitThreadListStore(store, snapshot: provider.snapshot)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().automationThreads(
                id: automationId,
                limit: provider.pager.pageLimit,
                offset: 0
            )
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = automationThreadProviders[automationId] else { return }
            let completion = owned.completeRefresh(ticket, page: page)
            automationThreadProviders[automationId] = owned
            applyAutomationCompletion(completion, automationId: automationId, store: store)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = automationThreadProviders[automationId] else { return }
            owned.failRefresh(ticket)
            automationThreadProviders[automationId] = owned
            commitThreadListStore(store, snapshot: owned.snapshot)
            store.setAvailability(
                owned.snapshot.isPrimed
                    ? .ready
                    : .failed(message: displayMessage(for: error))
            )
        }
    }

    func loadMoreAutomationThreadList(
        automationId: String,
        trigger: GaryxThreadListLoadMoreTrigger,
        retryingFailure: Bool = false
    ) async {
        guard var provider = automationThreadProviders[automationId],
              let store = automationThreadStores[automationId] else { return }
        let ticket = retryingFailure
            ? provider.retryLoadMore()
            : provider.requestLoadMore(trigger: trigger)
        guard let ticket else { return }
        automationThreadProviders[automationId] = provider
        commitThreadListStore(store, snapshot: provider.snapshot)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().automationThreads(
                id: automationId,
                limit: ticket.pagerTicket.limit,
                offset: ticket.pagerTicket.offset
            )
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = automationThreadProviders[automationId] else { return }
            let completion = owned.completeLoadMore(ticket, page: page)
            automationThreadProviders[automationId] = owned
            applyAutomationCompletion(completion, automationId: automationId, store: store)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration,
                  var owned = automationThreadProviders[automationId] else { return }
            owned.failLoadMore(ticket)
            automationThreadProviders[automationId] = owned
            commitThreadListStore(store, snapshot: owned.snapshot)
        }
    }

    func retryLoadMoreAutomationThreadList(automationId: String) async {
        await loadMoreAutomationThreadList(
            automationId: automationId,
            trigger: .footer,
            retryingFailure: true
        )
    }

    private func applyAutomationCompletion(
        _ completion: GaryxThreadListProviderCompletion,
        automationId: String,
        store: GaryxThreadListStore
    ) {
        switch completion {
        case .accepted(let commit):
            commitThreadListStore(
                store,
                snapshot: commit.snapshot,
                summaryWrites: commit.summaryWrites
            )
            store.setAvailability(.ready)
            publishThreadSummaryState()
        case .replacementRequired:
            Task { await refreshAutomationThreadList(automationId: automationId) }
        case .rejectedStaleInstance:
            break
        }
    }

    func botThreadListStore(group: GaryxMobileBotGroup) -> GaryxThreadListStore {
        if let store = botThreadStores[group.id] { return store }
        let provider = GaryxBotConversationMembershipProvider(groupId: group.id)
        let store = GaryxThreadListStore(
            ownerId: "bot:\(group.id)",
            cache: threadSummaryCache,
            leaseOwner: threadSummaryLeaseOwner
        )
        botThreadProviders[group.id] = provider
        botThreadStores[group.id] = store
        commitThreadListStore(store, snapshot: provider.snapshot)
        return store
    }

    func refreshBotThreadList(group: GaryxMobileBotGroup) {
        let store = botThreadListStore(group: group)
        store.setAvailability(.ready)
        let entries = group.sidebarChildConversationEntries().compactMap { entry -> GaryxBotConversationMembershipEntry? in
            guard let threadId = entry.threadId?.garyxTrimmedNilIfEmpty else { return nil }
            return GaryxBotConversationMembershipEntry(
                threadId: threadId,
                endpointKey: entry.endpoint.endpointKey,
                openable: entry.openable,
                canArchiveEndpoint: true
            )
        }
        let availableIds = Set(entries.compactMap { entry in
            threadSummaryCache.summary(for: entry.threadId) == nil ? nil : entry.threadId
        })
        guard var provider = botThreadProviders[group.id] else { return }
        let update = provider.replaceEntries(entries, availableSummaryIds: availableIds)
        botThreadProviders[group.id] = provider
        botThreadHydrationTasks[group.id]?.values.forEach { $0.cancel() }
        botThreadHydrationTasks[group.id] = [:]
        let cachedRows = threadSummaryCache.summaries(for: update.commit.snapshot.orderedThreadIds)
        threadSummaryLeaseOwner.replaceBotEntries(
            groupId: group.id,
            threadIds: update.commit.snapshot.orderedThreadIds,
            summaries: cachedRows
        )
        commitThreadListStore(
            store,
            snapshot: update.commit.snapshot,
            botEntries: Dictionary(uniqueKeysWithValues: entries.map { ($0.threadId, $0) })
        )
        for ticket in update.hydrationTickets {
            let task = Task { [weak self] in
                guard let self else { return }
                do {
                    let summary = try await client().getThread(threadId: ticket.threadId)
                    guard !Task.isCancelled,
                          let owned = botThreadProviders[group.id] else { return }
                    let completion = owned.completeHydration(ticket, summary: summary)
                    guard case .accepted(let commit) = completion else { return }
                    threadSummaryLeaseOwner.replaceBotEntries(
                        groupId: group.id,
                        threadIds: commit.snapshot.orderedThreadIds,
                        summaries: commit.summaryWrites
                    )
                    commitThreadListStore(
                        store,
                        snapshot: commit.snapshot,
                        summaryWrites: commit.summaryWrites,
                        botEntries: Dictionary(
                            uniqueKeysWithValues: entries.map { ($0.threadId, $0) }
                        )
                    )
                    publishThreadSummaryState()
                } catch {
                    guard !Task.isCancelled else { return }
                    store.setAvailability(.failed(message: displayMessage(for: error)))
                }
                botThreadHydrationTasks[group.id]?[ticket.threadId] = nil
            }
            botThreadHydrationTasks[group.id]?[ticket.threadId] = task
        }
    }

    func probeThreadSummaryCapability() async -> GaryxThreadSummaryCapabilityProbeResult {
        guard hasGatewaySettings else { return .failed }
        do {
            let gatewayClient = try client()
            return await gatewayClient.probeThreadSummariesCapability()
        } catch {
            return .failed
        }
    }

    func resolveThreadSummaryCapability() async -> GaryxThreadSummaryCapabilityResolution {
        let resolution = await threadSummaryCapabilityStateMachine.resolve()
        if resolution.becameSupported {
            runThreadFavoritesEffects(
                threadFavoritesProvider.transitionToSupported(
                    capabilityGeneration: resolution.capabilityGeneration
                ).effects
            )
        }
        return resolution
    }

    func resetThreadSummaryOwnership() {
        cancelThreadFavoritesSnapshotTransport()
        for tasks in botThreadHydrationTasks.values {
            for task in tasks.values { task.cancel() }
        }
        botThreadHydrationTasks = [:]
        for store in workspaceThreadStores.values { store.resetGatewayScope() }
        for store in automationThreadStores.values { store.resetGatewayScope() }
        for store in botThreadStores.values { store.resetGatewayScope() }
        workspaceThreadProviders = [:]
        workspaceThreadStores = [:]
        automationThreadProviders = [:]
        automationThreadStores = [:]
        botThreadProviders = [:]
        botThreadStores = [:]
        threadFeedRegistry.resetGatewayScope()
        threadSummaryLeaseOwner.resetGatewayScope()
        threadSummaryRuntimeEpoch &+= 1
        homeThreadListStore.resetTransitions(gatewayRuntimeEpoch: threadSummaryRuntimeEpoch)
        Task {
            await threadSummaryCapabilityStateMachine.reset(runtimeEpoch: threadSummaryRuntimeEpoch)
        }
    }

    #if DEBUG
    /// Explicit catalog/debug fixture seed. Production code never replaces a
    /// whole-model summary collection.
    func seedThreadSummariesForTesting(
        _ summaries: [GaryxThreadSummary],
        recentThreadIds: [String]? = nil
    ) {
        threadSummaryCache.writeThrough(summaries)
        if let recentThreadIds {
            recentThreadFeeds.resetFeedData()
            for threadId in recentThreadIds.reversed() {
                recentThreadFeeds.upsertChat(threadId: threadId)
            }
        }
        refreshRecentThreadLeases(summaryWrites: summaries)
        publishThreadSummaryState()
    }

    /// UI-test fixture seed for the scoped workspace membership owner. This
    /// follows the same cache + store commit boundary as a decoded first page
    /// without teaching production code a second transport path.
    func seedWorkspaceThreadListForTesting(
        path rawPath: String,
        summaries: [GaryxThreadSummary]
    ) {
        let path = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { return }
        let rows = summaries.filter {
            $0.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) == path
        }
        let store = workspaceThreadListStore(path: path)
        guard var provider = workspaceThreadProviders[path],
              let ticket = provider.requestRefresh() else { return }
        let completion = provider.completeRefresh(
            ticket,
            page: GaryxThreadSummariesPage(
                storeIncarnationId: "debug-workspace-incarnation",
                serverBootId: "debug-workspace-boot",
                threads: rows,
                hasMore: false,
                nextCursor: nil
            )
        )
        workspaceThreadProviders[path] = provider
        guard case .accepted(let commit) = completion else { return }
        commitThreadListStore(store, snapshot: commit.snapshot, summaryWrites: commit.summaryWrites)
        store.setAvailability(.ready)
    }
    #endif
}
