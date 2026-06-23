import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

actor GaryxRecentThreadsWidgetPersistenceQueue {
    private let planner = GaryxRecentThreadsWidgetPersistencePlanner()
    private var latestGeneration: UInt64 = 0

    func persist(input: GaryxRecentThreadsWidgetSnapshotInput, generation: UInt64) {
        latestGeneration = max(latestGeneration, generation)
        guard generation == latestGeneration else { return }
        let widgetThreads = GaryxRecentThreadsWidgetSnapshotProjector.widgetThreads(from: input)
        guard generation == latestGeneration else { return }
        switch planner.nextWrite(for: widgetThreads) {
        case .skipUnchanged:
            return
        case .write(let threads):
            GaryxMobileWidgetStore.saveRecentThreads(threads)
            WidgetCenter.shared.reloadTimelines(ofKind: GaryxRecentThreadsWidgetConstants.kind)
        }
    }
}

extension GaryxMobileModel {
    func isThreadPinned(_ threadId: String) -> Bool {
        pinnedThreadIds.contains(threadId.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    func togglePinnedThread(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        let pinned = !isThreadPinned(normalizedId)
        Task { await setThreadPinned(normalizedId, pinned: pinned) }
    }

    func unpinThread(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        Task { await setThreadPinned(normalizedId, pinned: false) }
    }

    func setThreadPinned(_ threadId: String, pinned: Bool) async {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        let previousIds = pinnedThreadIds
        pinnedThreadIds = Self.pinnedThreadIdsWith(
            pinnedThreadIds,
            threadId: normalizedId,
            pinned: pinned
        )
        do {
            let page = try await client().setThreadPinned(threadId: normalizedId, pinned: pinned)
            applyPinnedThreadIds(page.threadIds)
            persistRecentThreadsWidgetSnapshot()
        } catch {
            pinnedThreadIds = previousIds
            persistRecentThreadsWidgetSnapshot()
            lastError = displayMessage(for: error)
        }
    }

    func applyPinnedThreadIds(_ ids: [String]) {
        let normalized = Self.normalizedPinnedThreadIds(ids)
        // The silent sidebar refresh loop calls this every few seconds; skip
        // the publish when nothing changed so observers do not re-render.
        if pinnedThreadIds != normalized {
            pinnedThreadIds = normalized
        }
    }

    func removePinnedThreadIdLocally(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        pinnedThreadIds.removeAll { $0 == normalizedId }
    }

    func removeArchivedThreadLocally(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        let transactionId = homeProjectionGateway.beginTransaction(label: "archive-local-remove")
        defer { homeProjectionGateway.endTransaction(transactionId) }
        pinnedThreadIds.removeAll { $0 == normalizedId }
        recentThreadIds.removeAll { $0 == normalizedId }
        threads.removeAll { $0.id == normalizedId }
        clearPersistedLastOpenedThreadId(ifMatches: normalizedId)
        persistRecentThreadsWidgetSnapshot()
    }

    // MARK: - Last opened thread restore

    /// Remembers the most recently opened thread per gateway scope so a fresh
    /// app launch can land back in it instead of the new-thread draft.
    func persistLastOpenedThreadId(_ threadId: String) {
        #if DEBUG
        if debugSnapshotActive { return }
        #endif
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        defaults.set(normalizedId, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId))
    }

    func persistOpenedThreadDestination(_ destination: GaryxWorkflowRunDestination) {
        let previousThreadId = persistedLastOpenedThreadId
        guard let nextThreadId = GaryxLastOpenedThreadRestorationPolicy.persistedThreadId(
            afterOpening: destination,
            previousThreadId: previousThreadId
        ) else {
            return
        }
        persistLastOpenedThreadId(nextThreadId)
    }

    func clearPersistedLastOpenedThreadId(ifMatches threadId: String) {
        let key = scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId)
        guard defaults.string(forKey: key) == threadId else { return }
        defaults.removeObject(forKey: key)
    }

    func restorePersistedLastOpenedThreadId(_ threadId: String?) {
        let key = scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId)
        let normalizedId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if normalizedId.isEmpty {
            defaults.removeObject(forKey: key)
        } else {
            defaults.set(normalizedId, forKey: key)
        }
    }

    /// True when the app last went to background while showing a
    /// conversation; launches restore the thread only in that case.
    func persistLastSessionLocation() {
        #if DEBUG
        if debugSnapshotActive { return }
        #endif
        let onThread = GaryxLastOpenedThreadRestorationPolicy.isCurrentSessionRestorable(
            navigationState: navigationState,
            selectedThreadId: selectedThread?.id,
            activeWorkflowRunId: workflowRunPanelState.activeWorkflowRunId
        )
        defaults.set(onThread, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastSessionOnThread))
    }

    func persistLastSessionRestorable(_ restorable: Bool) {
        #if DEBUG
        if debugSnapshotActive { return }
        #endif
        defaults.set(restorable, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastSessionOnThread))
    }

    var persistedLastSessionWasOnThread: Bool {
        defaults.bool(forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastSessionOnThread))
    }

    var persistedLastOpenedThreadId: String? {
        let value = defaults.string(forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    /// One-shot launch restore: when nothing else (deep link, widget link,
    /// pending route) claimed navigation, reopen the last opened thread
    /// through the shared open path.
    func restoreLastOpenedThreadIfNeeded() async {
        guard !hasAttemptedLastOpenedThreadRestore else { return }
        hasAttemptedLastOpenedThreadRestore = true
        #if DEBUG
        guard !debugSnapshotActive else { return }
        #endif
        guard let threadId = GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
            persistedLastOpenedThreadId: persistedLastOpenedThreadId,
            persistedLastSessionWasOnThread: persistedLastSessionWasOnThread,
            selectedThreadId: selectedThread?.id,
            hasPendingMobileRoute: pendingMobileRoute != nil,
            hasPendingThreadIntent: threadOpenState.hasPendingIntent,
            navigationState: navigationState,
            sidebarVisible: sidebarVisible
        ) else {
            return
        }
        await restoreLastOpenedThread(id: threadId)
    }

    static func pinnedThreadIdsWith(
        _ ids: [String],
        threadId: String,
        pinned: Bool
    ) -> [String] {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return normalizedPinnedThreadIds(ids) }
        let remaining = normalizedPinnedThreadIds(ids).filter { $0 != normalizedId }
        return pinned ? [normalizedId] + remaining : remaining
    }

    static func normalizedPinnedThreadIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []
        for rawId in ids {
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            normalized.append(id)
        }
        return normalized
    }

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
            recentThreadIds: recentThreadIds
        )
        let queue = recentThreadsWidgetPersistenceQueue
        Task.detached(priority: .utility) {
            await queue.persist(input: input, generation: generation)
        }
    }

    func widgetAgentIdentity(for thread: GaryxThreadSummary) -> WidgetAgentIdentity {
        let teamId = thread.teamId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !teamId.isEmpty {
            if let team = teams.first(where: { $0.id == teamId }) {
                return WidgetAgentIdentity(
                    id: team.id,
                    name: team.displayName,
                    avatarDataUrl: team.avatarDataUrl.isEmpty ? nil : team.avatarDataUrl,
                    providerType: nil,
                    isTeam: true,
                    builtIn: false
                )
            }
            return WidgetAgentIdentity(
                id: teamId,
                name: thread.teamName,
                avatarDataUrl: nil,
                providerType: nil,
                isTeam: true,
                builtIn: false
            )
        }

        let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !agentId.isEmpty {
            if let agent = agents.first(where: { $0.id == agentId }) {
                return WidgetAgentIdentity(
                    id: agent.id,
                    name: agent.displayName,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    providerType: agent.providerType,
                    isTeam: false,
                    builtIn: agent.builtIn
                )
            }
            return WidgetAgentIdentity(
                id: agentId,
                name: nil,
                avatarDataUrl: nil,
                providerType: thread.providerType,
                isTeam: false,
                builtIn: false
            )
        }

        return WidgetAgentIdentity(
            id: nil,
            name: nil,
            avatarDataUrl: nil,
            providerType: thread.providerType,
            isTeam: false,
            builtIn: false
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
        let threadId = refreshedThread.id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty,
              selectedThread?.id != threadId,
              !isThreadSummaryRunning(refreshedThread) else {
            return false
        }
        return previousThread.map(isThreadSummaryRunning) == true
            || previousRemoteBusyThreadIds.contains(threadId)
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
        let selectedThreadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        var ids = runTracker.locallyTrackedThreadIds
        ids.formUnion(runStateByThread.compactMap { threadId, state in
            state.busy ? threadId : nil
        })
        ids.formUnion(threads.compactMap { thread in
            if let committedState = runStateByThread[thread.id] {
                return committedState.busy ? thread.id : nil
            }
            return isThreadSummaryRunning(thread) ? thread.id : nil
        })
        return ids
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && $0 != selectedThreadId }
            .sorted()
    }

    func isThreadSummaryRunning(_ thread: GaryxThreadSummary) -> Bool {
        let runState = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return runState == "running"
    }

    func selectThread(
        _ thread: GaryxThreadSummary,
        invalidatesPendingThreadOpen: Bool = true,
        source: GaryxMobilePanelOpenSource = .replace
    ) async {
        let reopeningSelectedThreadFromHome = isHomeVisible && selectedThread?.id == thread.id
        showSelectedThread(
            thread,
            invalidatesPendingThreadOpen: invalidatesPendingThreadOpen,
            source: source,
            startsSelectedThreadStream: !reopeningSelectedThreadFromHome
        )
        // Bound the open to the newest ~threadHistoryUserQueryLimit user turns: always
        // refresh from the gateway, which returns the forward delta when the cached
        // cursor is within that window, or the newest window + `reset` when the cursor
        // is older (the client overwrites its cache). With no cache it seeds the newest
        // window. The stream then resumes near the tail (live only); older history
        // pages in on scroll-up. The stream supersedes the reconcile poll and falls
        // back to it (and the after_index HTTP path) on failure.
        await loadSelectedThreadHistory()
        if reopeningSelectedThreadFromHome {
            ensureSelectedThreadStreamForVisibleConversation()
        }
    }

    func showSelectedThread(
        _ thread: GaryxThreadSummary,
        invalidatesPendingThreadOpen: Bool = true,
        source: GaryxMobilePanelOpenSource = .replace,
        startsSelectedThreadStream: Bool = true
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
        let shouldSuppressStreamPolicy = !startsSelectedThreadStream
        if shouldSuppressStreamPolicy {
            suppressesSelectedThreadStreamPolicy = true
        }
        selectedThread = thread
        if shouldSuppressStreamPolicy {
            suppressesSelectedThreadStreamPolicy = false
        }
        if !thread.excludeFromRecent {
            persistOpenedThreadDestination(.chat(threadId: thread.id))
        }
        clearPendingNewThreadAgentTarget()
        clearPendingBotDraft()
        draftThreadTitle = thread.title
        openConversation(
            source: source,
            invalidatesPendingThreadOpen: false,
            startsSelectedThreadStream: startsSelectedThreadStream
        )
        if previousThreadId != thread.id {
            let inMemory = cachedMessages(for: thread.id)
            if inMemory.isEmpty {
                // Cold start / first open this session: show the persisted committed
                // window immediately instead of a blank screen, then refresh below.
                let restored = restoredCachedMessages(for: thread.id)
                if restored.isEmpty {
                    messages = []
                } else {
                    setMessages(restored, for: thread.id)
                }
            } else {
                messages = inMemory
            }
        }
    }

    func openNewThreadDraft(agentTargetOverride: String? = nil) {
        invalidatePendingThreadOpen()
        advanceSelectedThreadDraftGeneration()
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
        let trimmedWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedWorkspace.isEmpty else { return "local" }
        guard Self.normalizedWorkspaceMode(newThreadWorkspaceMode) == "worktree" else { return "local" }
        guard workspaceGitStatuses[trimmedWorkspace]?.canUseWorktree == true else { return "local" }
        return "worktree"
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
            await updateTranscriptCache(
                threadId: threadId,
                fetched: transcript,
                direction: .older,
                committedOnly: true
            )
            updateSelectedThreadHistoryPagination(threadId: threadId, transcript: transcript)
            prependOlderMessages(
                mobileMessages(from: transcript.messages, live: false),
                for: threadId
            )
        } catch {
            guard self.selectedThread?.id == threadId else { return }
            lastError = displayMessage(for: error)
        }
    }

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
        runStateByThread = next
        emitHomeProjectionSnapshot()
    }

    func emitCommittedRunStateProjectionDelta(threadId: String, state: GaryxTranscriptRunState) {
        homeProjectionGateway.captureCommittedRunStateDelta(threadId: threadId, isRunning: state.busy)
        if !HomeProjectionLiveSourceConfiguration.usesActorSnapshots {
            emitHomeProjectionSnapshot()
        }
    }

    func summaryWithCommittedRunState(_ thread: GaryxThreadSummary) -> GaryxThreadSummary {
        guard let state = runStateByThread[thread.id] else {
            var updated = thread
            if var runtime = updated.threadRuntime {
                runtime.activeRun = nil
                updated.threadRuntime = runtime
            }
            updated.runState = GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: updated.runState,
                recentRunId: updated.recentRunId,
                committedState: nil
            )
            return updated
        }
        return summary(thread, applying: state)
    }

    func summary(_ thread: GaryxThreadSummary, applying state: GaryxTranscriptRunState) -> GaryxThreadSummary {
        var updated = thread
        if var runtime = updated.threadRuntime {
            runtime.activeRun = nil
            updated.threadRuntime = runtime
        }
        updated.activeRunId = state.busy ? state.activeRunId : nil
        updated.runState = GaryxThreadSummaryRunStateResolver.resolvedRunState(
            apiRunState: updated.runState,
            recentRunId: updated.recentRunId,
            committedState: state
        )
        if let title = state.title?.trimmingCharacters(in: .whitespacesAndNewlines),
           !title.isEmpty {
            updated.title = title
        }
        return updated
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

    func applyThreadRuntimeSummary(_ runtime: GaryxThreadRuntimeSummary, threadId: String) {
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

        if let index = threads.firstIndex(where: { $0.id == normalizedThreadId }) {
            let nextThread = mergedRuntimeSummary(threads[index])
            if threads[index] != nextThread {
                var nextThreads = threads
                nextThreads[index] = nextThread
                threads = nextThreads
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

    func updateSelectedThreadHistoryPagination(
        threadId: String,
        transcript: GaryxThreadTranscript,
        preservingLoadedOlderPages: Bool = false
    ) {
        guard selectedThread?.id == threadId else { return }
        if preservingLoadedOlderPages,
           let oldestLoadedIndex = oldestLoadedHistoryIndex(for: threadId),
           let latestPageStartIndex = preserveRemoteBeforeIndex(from: transcript),
           oldestLoadedIndex < latestPageStartIndex {
            if oldestLoadedIndex > 0 {
                selectedThreadHasMoreHistoryBefore = true
                selectedThreadNextHistoryBeforeIndex = oldestLoadedIndex
            } else {
                selectedThreadHasMoreHistoryBefore = false
                selectedThreadNextHistoryBeforeIndex = nil
            }
            return
        }
        selectedThreadHasMoreHistoryBefore = transcript.pageInfo?.hasMoreBefore ?? false
        selectedThreadNextHistoryBeforeIndex = transcript.pageInfo?.nextBeforeIndex
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
        selectedThread?.id == threadId
            && remoteBusyThreadIds.contains(threadId)
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
                await refreshThreads()
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
                await refreshThreads()
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
        guard case let .object(snapshot)? = payload["payload"] else {
            return nil
        }
        let data = try JSONEncoder().encode(GaryxJSONValue.object(snapshot))
        return try JSONDecoder().decode(GaryxThreadTranscript.self, from: data)
    }
}
