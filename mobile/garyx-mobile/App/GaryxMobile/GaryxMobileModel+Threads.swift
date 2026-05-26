import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

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
        pinnedThreadIds = Self.normalizedPinnedThreadIds(ids)
    }

    func removePinnedThreadIdLocally(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        pinnedThreadIds.removeAll { $0 == normalizedId }
    }

    func removeArchivedThreadLocally(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        pinnedThreadIds.removeAll { $0 == normalizedId }
        recentThreadIds.removeAll { $0 == normalizedId }
        threads.removeAll { $0.id == normalizedId }
        persistRecentThreadsWidgetSnapshot()
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
            applyPinnedThreadIds(pinsPage.threadIds)
            applyRecentThreadsPage(page, preservesLoadedPages: silent)
            var nextThreads = page.threads
            let selectionIdForThisRefresh = selectedThread?.id
            let requiredThreadIds = normalizedThreadIds(pinsPage.threadIds + [selectionIdForThisRefresh])
            nextThreads += await fetchMissingThreadSummaries(
                using: gatewayClient,
                requiredThreadIds: requiredThreadIds,
                existingThreadIds: Set(nextThreads.map(\.id))
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let existingThreads = silent ? threads : []
            let refreshedThreads = Self.mergedThreadSummaries(nextThreads)
            threads = Self.mergedThreadSummaries(existingThreads + refreshedThreads)
            persistRecentThreadsWidgetSnapshot()
            refreshRemoteBusyIdsForVisibleThreads()
            hydrateCompletedRecentThreadHistories(
                previousThreads: previousThreadSummaries,
                previouslyRemoteBusyThreadIds: previouslyRemoteBusyThreadIds,
                refreshedThreads: refreshedThreads,
                runtimeGeneration: runtimeGeneration
            )
            let currentSelectedId = selectedThread?.id
            if let selectionIdForThisRefresh,
               currentSelectedId == selectionIdForThisRefresh,
               let updatedSelection = threads.first(where: { $0.id == selectionIdForThisRefresh }) {
                selectedThread = updatedSelection
                draftThreadTitle = updatedSelection.title
            } else if currentSelectedId == selectionIdForThisRefresh, selectionIdForThisRefresh != nil {
                selectedThread = nil
                draftThreadTitle = ""
                resetComposerDraft()
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func applyRecentThreadsPage(_ page: GaryxRecentThreadsPage, preservesLoadedPages: Bool) {
        let pageIds = page.threads.map(\.id)
        let returnedEnd = page.offset + page.count
        let hasLoadedBeyondHead = preservesLoadedPages
            && (nextThreadListOffset > returnedEnd || recentThreadIds.count > pageIds.count)

        if hasLoadedBeyondHead {
            let pageIdSet = Set(pageIds)
            let existingTail = recentThreadIds.filter { !pageIdSet.contains($0) }
            recentThreadIds = pageIds + existingTail
            return
        }

        updateThreadListPagination(from: page)
        recentThreadIds = pageIds
    }

    func persistRecentThreadsWidgetSnapshot() {
        var summariesById: [String: GaryxThreadSummary] = [:]
        for thread in threads where summariesById[thread.id] == nil {
            summariesById[thread.id] = thread
        }
        let orderedThreadIds = normalizedThreadIds((pinnedThreadIds + recentThreadIds).map { Optional($0) })
        let widgetThreads = orderedThreadIds.compactMap { threadId -> GaryxMobileWidgetThread? in
            guard let thread = summariesById[threadId] else { return nil }
            let workspaceName = thread.workspacePath?
                .garyxLastPathComponent
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let identity = widgetAgentIdentity(for: thread)
            return GaryxMobileWidgetThread(
                id: thread.id,
                title: thread.title,
                workspaceName: workspaceName,
                updatedAt: thread.updatedAt ?? thread.createdAt,
                activeRunId: thread.activeRunId,
                runState: thread.runState,
                agentId: identity.id,
                agentName: identity.name,
                avatarDataUrl: identity.avatarDataUrl,
                providerType: identity.providerType,
                isTeam: identity.isTeam,
                builtIn: identity.builtIn
            )
        }
        GaryxMobileWidgetStore.saveRecentThreads(widgetThreads)
        WidgetCenter.shared.reloadTimelines(ofKind: GaryxRecentThreadsWidgetConstants.kind)
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
            var seenRecentIds = Set(recentThreadIds)
            recentThreadIds += page.threads.compactMap { thread in
                seenRecentIds.insert(thread.id).inserted ? thread.id : nil
            }
            threads = Self.mergedThreadSummaries(threads + page.threads)
            persistRecentThreadsWidgetSnapshot()
            refreshRemoteBusyIdsForVisibleThreads()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func updateThreadListPagination(from page: GaryxThreadsPage) {
        let returnedEnd = page.offset + page.count
        nextThreadListOffset = returnedEnd
        hasMoreThreadSummaries = returnedEnd < page.total
    }

    func updateThreadListPagination(from page: GaryxRecentThreadsPage) {
        nextThreadListOffset = page.offset + page.count
        hasMoreThreadSummaries = page.hasMore
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
            threads = Self.mergedThreadSummaries(threads + allThreads)
            await mergeMissingSidebarRequiredThreads(
                using: gatewayClient,
                extraThreadIds: [selectedThread?.id],
                runtimeGeneration: runtimeGeneration
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            refreshRemoteBusyIdsForVisibleThreads()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func refreshRemoteBusyIdsForVisibleThreads() {
        var refreshedBusyIds = remoteBusyThreadIds
        for thread in threads {
            if !(thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) {
                if activeTasksByThread[thread.id] == nil {
                    refreshedBusyIds.insert(thread.id)
                } else {
                    refreshedBusyIds.remove(thread.id)
                }
            } else if activeTasksByThread[thread.id] == nil {
                refreshedBusyIds.remove(thread.id)
            }
        }
        remoteBusyThreadIds = refreshedBusyIds
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
              activeTasksByThread[threadId] == nil,
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

    func hydrateCompletedRecentThreadHistoryNow(threadId: String, runtimeGeneration: UUID) async {
        defer {
            completedThreadHistoryHydrationTasks[threadId] = nil
        }
        guard runtimeGeneration == gatewayRuntimeGeneration,
              hasGatewaySettings else {
            return
        }
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            applyThreadTranscriptToCache(
                transcript,
                threadId: threadId,
                preservingLoadedOlderPages: true,
                scheduleRecoveryIfSelected: false
            )
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let message = displayMessage(for: error)
            if Self.isTransientGatewayErrorMessage(message) {
                gatewaySettingsStatus = "Waiting to sync with gateway"
            }
        }
    }

    func isThreadSummaryRunning(_ thread: GaryxThreadSummary) -> Bool {
        let runState = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let activeRunId = thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return runState == "running" || !activeRunId.isEmpty
    }

    func selectThread(_ thread: GaryxThreadSummary) async {
        let previousThreadId = selectedThread?.id
        if previousThreadId != thread.id {
            advanceSelectedThreadDraftGeneration()
            resetComposerDraft()
            selectedThreadRecoveryTask?.cancel()
            selectedThreadRecoveryTask = nil
            selectedThreadRecoveryThreadId = nil
            cancelSelectedThreadReconcileLoop()
            resetSelectedThreadHistoryPagination()
        }
        selectedThread = thread
        clearPendingBotDraft()
        draftThreadTitle = thread.title
        activePanel = .chat
        setSidebarVisible(false)
        if previousThreadId != thread.id {
            messages = cachedMessages(for: thread.id)
        }
        await loadSelectedThreadHistory()
        startSelectedThreadReconcileLoop()
    }

    func openNewThreadDraft() {
        advanceSelectedThreadDraftGeneration()
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
        cancelSelectedThreadReconcileLoop()
        selectedThreadHistoryRequestId = nil
        isLoadingSelectedThreadHistory = false
        resetSelectedThreadHistoryPagination()
        clearPendingBotDraft()
        selectedThread = nil
        draftThreadTitle = ""
        resetComposerDraft()
        messages = []
        activePanel = .chat
        setSidebarVisible(false)
        lastError = nil
    }

    func createThread() async {
        clearPendingBotDraft()
        await createThread(workspaceOverride: nil)
    }

    func createThreadFromCurrentDraft() async {
        guard pendingBotId != nil else {
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
        do {
            saveGatewaySettings()
            let workspace = (workspaceOverride ?? newThreadWorkspace).trimmingCharacters(in: .whitespacesAndNewlines)
            let agentId = (agentOverride ?? selectedAgentTargetId).trimmingCharacters(in: .whitespacesAndNewlines)
            let workspaceMode = workspaceModeForNewThread(workspace: workspace)
            let thread = try await client().createThread(
                GaryxCreateThreadRequest(
                    workspaceDir: workspace.isEmpty ? nil : workspace,
                    workspaceMode: workspaceMode,
                    agentId: agentId.isEmpty ? nil : agentId,
                    metadata: ["client": "garyx-mobile"]
                )
            )
            threads.insert(thread, at: 0)
            selectedThread = thread
            clearPendingBotDraft()
            resetComposerDraft()
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
        if let status = workspaceGitStatuses[trimmedWorkspace], !status.canUseWorktree {
            return "local"
        }
        return "worktree"
    }

    func createThread(inWorkspace workspacePath: String) async {
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
        pendingBotId = Self.botSelectorId(channel: group.channel, accountId: group.accountId)
        pendingBotWorkspace = workspace.isEmpty ? nil : workspace
        pendingBotAgentId = agentId.isEmpty ? nil : agentId
        advanceSelectedThreadDraftGeneration()
        cancelSelectedThreadReconcileLoop()
        selectedThread = nil
        resetSelectedThreadHistoryPagination()
        draftThreadTitle = ""
        resetComposerDraft()
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
                resetComposerDraft()
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
            discardPendingAssistantDelta(for: thread.id)
            messagesByThread[thread.id] = nil
            messageSignaturesByThread[thread.id] = nil
            activeAssistantMessageIdsByThread[thread.id] = nil
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

    func loadSelectedThreadHistory() async {
        guard let selectedThread else {
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
        defer {
            if selectedThreadHistoryRequestId == requestId {
                isLoadingSelectedThreadHistory = false
            }
        }
        let shouldFastLoadVisibleMessages = cachedMessages(for: threadId).isEmpty
        do {
            if shouldFastLoadVisibleMessages {
                let visibleTranscript = try await client().threadHistory(
                    threadId: threadId,
                    limit: Self.threadHistoryPageLimit,
                    userQueryLimit: Self.threadHistoryUserQueryLimit,
                    includeToolMessages: false
                )
                guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
                applySelectedThreadTranscript(visibleTranscript, threadId: threadId)
                isLoadingSelectedThreadHistory = false

                do {
                    let fullTranscript = try await client().threadHistory(
                        threadId: threadId,
                        limit: Self.threadHistoryPageLimit,
                        userQueryLimit: Self.threadHistoryUserQueryLimit
                    )
                    guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
                    applySelectedThreadTranscript(fullTranscript, threadId: threadId)
                } catch {
                    guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
                    let message = displayMessage(for: error)
                    if Self.isTransientGatewayErrorMessage(message) {
                        gatewaySettingsStatus = "Waiting to sync with gateway"
                    }
                }
                return
            }

            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
            applySelectedThreadTranscript(transcript, threadId: threadId)
        } catch {
            guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
            if cachedMessages(for: threadId).isEmpty {
                messages = []
            }
            lastError = displayMessage(for: error)
        }
    }

    func applySelectedThreadTranscript(_ transcript: GaryxThreadTranscript, threadId: String) {
        applyThreadTranscriptToCache(
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
    ) {
        selectedThreadActivitySignatures[threadId] = GaryxThreadActivitySignature.make(from: transcript)
        updateThreadRuntimeState(threadId: threadId, transcript: transcript)
        if selectedThread?.id == threadId {
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: preservingLoadedOlderPages
            )
        }
        let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
        setMessages(
            mergedMessages(
                remoteMessages,
                withLocal: cachedMessages(for: threadId),
                preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
            ),
            for: threadId,
            reconcileActiveAssistant: true
        )
        if scheduleRecoveryIfSelected {
            scheduleSelectedThreadRecoveryIfNeeded(threadId: threadId)
        }
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

    func updateThreadRuntimeState(threadId: String, transcript: GaryxThreadTranscript) {
        let hasActiveRun = transcript.threadRuntime?.activeRun != nil
        let hasActivePendingInput = transcript.pendingUserInputs.contains { input in
            input.active && (input.status ?? "awaiting_ack").lowercased() != "abandoned"
        }
        if hasActiveRun || hasActivePendingInput {
            if activeTasksByThread[threadId] == nil {
                remoteBusyThreadIds.insert(threadId)
            } else {
                remoteBusyThreadIds.remove(threadId)
            }
        } else if activeTasksByThread[threadId] == nil {
            remoteBusyThreadIds.remove(threadId)
            markThreadSummaryRuntimeInactive(threadId)
        }
    }

    func markThreadSummaryRuntimeInactive(_ threadId: String) {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return }

        func inactiveSummary(_ thread: GaryxThreadSummary) -> GaryxThreadSummary {
            var updated = thread
            updated.activeRunId = nil
            let recentRunId = updated.recentRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            updated.runState = recentRunId.isEmpty ? "idle" : "completed"
            return updated
        }

        var changed = false
        threads = threads.map { thread in
            guard thread.id == normalizedThreadId,
                  !(thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) else {
                return thread
            }
            changed = true
            return inactiveSummary(thread)
        }
        if selectedThread?.id == normalizedThreadId,
           let selectedThread,
           !(selectedThread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) {
            self.selectedThread = inactiveSummary(selectedThread)
            changed = true
        }
        if changed {
            refreshRemoteBusyIdsForVisibleThreads()
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
            .compactMap { Self.historyIndex(fromMessageId: $0.id) }
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
              activeTasksByThread[threadId] == nil,
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
            && activeTasksByThread[threadId] == nil
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
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            selectedThreadActivitySignatures[threadId] = GaryxThreadActivitySignature.make(from: transcript)
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: true
            )
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
            if !remoteBusyThreadIds.contains(threadId) {
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
        if activeTasksByThread[threadId] != nil {
            return
        }
        let observedHistoryRequestId = selectedThreadHistoryRequestId
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            let signature = GaryxThreadActivitySignature.make(from: transcript)
            if selectedThreadActivitySignatures[threadId] == signature {
                updateThreadRuntimeState(threadId: threadId, transcript: transcript)
                return
            }
            selectedThreadActivitySignatures[threadId] = signature
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: true
            )
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
            if !remoteBusyThreadIds.contains(threadId) {
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
