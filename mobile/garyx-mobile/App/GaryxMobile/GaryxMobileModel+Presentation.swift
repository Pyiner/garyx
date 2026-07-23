import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func refreshShellChromeSnapshot() {
        shellChromeStore.apply(
            GaryxShellChromeSnapshot(
                sidebarVisible: sidebarVisible
            )
        )
    }

    func refreshNavigationDrawerSnapshot() {
        navigationDrawerStore.apply(navigationDrawerSnapshot)
    }

    func refreshHomeObservationSnapshot() {
        refreshHomeObservationConnectionSnapshot()
        refreshHomeObservationPaginationSnapshot()
        homeObservationStore.setShowsSettings(showsSettings)
        homeObservationStore.setDebugShowsGatewaySwitcher(debugShowsGatewaySwitcher)
        homeObservationStore.setLastError(lastError)
    }

    func refreshHomeObservationConnectionSnapshot() {
        homeObservationStore.applyConnection(
            isGatewayConfigured: hasGatewaySettings,
            connectionState: connectionState,
            willTransitionRootSurface: { [unowned self] transition in
                applyGlobalRevealRootSurfaceTransition(transition)
            }
        )
    }

    func refreshHomeObservationPaginationSnapshot() {
        guard let pager = recentThreadFeeds.selectedPager else {
            homeObservationStore.applyPagination(
                isLoadingMoreThreads: false,
                hasMoreThreadSummaries: false,
                loadMoreFooterState: .hidden
            )
            return
        }
        homeObservationStore.applyPagination(
            isLoadingMoreThreads: pager.isLoadingMore,
            hasMoreThreadSummaries: pager.hasMoreThreadSummaries,
            loadMoreFooterState: pager.footerState
        )
    }

    func clearLastErrorIfCurrent(_ message: String) {
        if lastError == message {
            lastError = nil
        }
    }

    func predecodeAgentAvatarImages() {
        GaryxDataURLImageCache.predecodeAgentAvatars(
            from: agents.map { Optional($0.avatarDataUrl) }
        )
        writeThroughAgentAvatarImages()
    }

    func writeThroughAgentAvatarImages() {
        let upserts = GaryxAvatarWriteThroughPlan.candidates(
            scope: currentGatewayScopeId,
            agents: agents
        )
        guard !upserts.isEmpty else { return }
        let store = avatarStore
        Task.detached(priority: .utility) {
            await store.upsert(
                upserts,
                validator: GaryxAvatarCGImageValidator(),
                now: Date()
            )
        }
    }

    func predecodeChannelIconImages() {
        GaryxDataURLImageCache.predecodeChannelIcons(
            from: channelPlugins.map(\.iconDataUrl) + mobileBotGroups.map(\.iconDataUrl)
        )
    }

    var navigationDrawerSnapshot: GaryxNavigationDrawerSnapshot {
        GaryxNavigationDrawerSnapshot(
            activePanel: activePanel,
            gatewayIdentity: gatewaySwitcherIdentity,
            gatewayRows: gatewaySwitcherRows,
            botGroups: mobileBotGroups,
            workspaceRows: navigationDrawerWorkspaceRows
        )
    }

    var navigationDrawerWorkspaceRows: [GaryxNavigationDrawerWorkspaceRow] {
        // Server order and names verbatim (pinned → activity → name → path).
        workspaceCatalog.workspaces.map { workspace in
            GaryxNavigationDrawerWorkspaceRow(
                path: workspace.path,
                name: workspace.name,
                pinned: workspace.pinned
            )
        }
    }

    func emitHomeProjectionSnapshot() {
        if HomeProjectionLiveSourceConfiguration.usesActorSnapshots {
            homeProjectionGateway.capture(homeProjectionCapture)
            syncBackgroundCommittedRunReconcileLoopForHomeVisibility()
            return
        }

        applyLegacyHomeThreadListSnapshot()
    }

    func applyLegacyHomeThreadListSnapshot() {
        let input = homeThreadListInput
        if homeThreadListStore.apply(input) {
            #if DEBUG
            GaryxHomeScrollPerformanceProbe.shared.markHomeListStoreApply()
            #endif
        }
        captureHomeProjectionShadow(input: input)
        syncBackgroundCommittedRunReconcileLoopForHomeVisibility()
    }

    func captureHomeProjectionShadow(input: GaryxHomeThreadListInput) {
        guard HomeProjectionShadowConfiguration.isEnabled else { return }
        homeProjectionGateway.capture(
            HomeProjectionCapture(
                legacyInput: input,
                runTrackerBusyThreadIds: runTracker.busyThreadIds,
                committedRunStateBusyByThreadId: runStateByThread.mapValues { $0.busy }
            )
        )
    }

    var homeProjectionCapture: HomeProjectionCapture {
        HomeProjectionCapture(
            threads: homeThreadSummaries,
            recentThreadIds: visibleRecentThreadIds,
            agents: agents,
            automations: automations,
            pinnedThreadIds: pinnedThreadIds,
            favoritedThreadIds: threadFavoritesState.presentedThreadIds,
            selectedThreadId: selectedThread?.id,
            isLoadingThreads: isLoadingThreads,
            isHomeVisible: isHomeVisible,
            selectedRecentFilter: recentThreadFeeds.selectedFilter,
            recentFeedPresentation: selectedRecentFeedPresentation,
            runTrackerBusyThreadIds: runTracker.busyThreadIds,
            committedRunStateBusyByThreadId: runStateByThread.mapValues { $0.busy }
        )
    }

    func applyHomeProjectionResult(_ result: HomeProjectionBoundaryResult) {
        guard HomeProjectionLiveSourceConfiguration.usesActorSnapshots else { return }
        if homeThreadListStore.apply(actorSnapshot: result.snapshot, difference: result.difference) {
            #if DEBUG
            GaryxHomeScrollPerformanceProbe.shared.markHomeListStoreApply()
            #endif
        }
    }

    var homeThreadListInput: GaryxHomeThreadListInput {
        GaryxHomeThreadListInput(
            sectionsInput: GaryxHomeThreadSectionsInput(
                threads: homeThreadSummaries,
                agents: agents,
                automations: automations,
                pinnedThreadIds: pinnedThreadIds,
                favoritedThreadIds: threadFavoritesState.presentedThreadIds,
                recentThreadIds: visibleRecentThreadIds,
                selectedThreadId: selectedThread?.id
            ),
            runningThreadIds: homeThreadRunningThreadIds,
            isLoadingThreads: isLoadingThreads,
            isHomeVisible: isHomeVisible,
            selectedRecentFilter: recentThreadFeeds.selectedFilter,
            recentFeedPresentation: selectedRecentFeedPresentation
        )
    }

    var homeThreadRunningThreadIds: Set<String> {
        Set(homeThreadSummaries.compactMap { thread in
            let threadId = thread.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !threadId.isEmpty, isThreadSummaryRunning(thread) else { return nil }
            return threadId
        })
    }

    var hasGatewaySettings: Bool {
        !gatewayURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var configuredBotAccountSettings: [GaryxConfiguredBotAccountSettings] {
        let accounts = GaryxConfiguredBotAccountsDocument.accounts(from: gatewaySettingsDocument)
        if !accounts.isEmpty || !gatewaySettingsDocument.isEmpty {
            return accounts
        }
        return configuredBots.map { bot in
            GaryxConfiguredBotAccountSettings(
                channel: bot.channel,
                accountId: bot.accountId,
                displayName: bot.displayName,
                enabled: bot.enabled,
                agentId: bot.agentId,
                workspaceDir: bot.workspaceDir,
                workspaceMode: bot.workspaceMode,
                config: [:]
            )
        }
    }

    var canConnectGateway: Bool {
        parsedGatewayURL(from: gatewayURL) != nil
    }

    var currentGatewayProfile: GaryxGatewayProfile? {
        let currentURL = normalizedGatewayURL(gatewayURL).lowercased()
        return gatewayProfiles.first { $0.gatewayUrl.lowercased() == currentURL }
    }

    var gatewaySwitcherIdentity: GaryxGatewaySwitcherIdentity {
        GaryxGatewaySwitcherPresentation.identity(
            gatewayURL: gatewayURL,
            profileLabel: currentGatewayProfile?.label,
            connectionState: connectionState
        )
    }

    var gatewaySwitcherRows: [GaryxGatewaySwitcherRow] {
        GaryxGatewaySwitcherPresentation.rows(
            profiles: gatewayProfiles,
            currentGatewayURL: gatewayURL
        )
    }

    func switchGateway(from row: GaryxGatewaySwitcherRow) {
        if row.isCurrent {
            if !isGatewayConnectionReady {
                Task { await connectAndRefresh() }
            }
            return
        }
        guard let profile = gatewayProfiles.first(where: { $0.id == row.profileId }) else { return }
        Task { await activateGatewayProfile(profile) }
    }

    var isGatewayConnectionReady: Bool {
        if case .ready = connectionState {
            return true
        }
        return false
    }

    var canSend: Bool {
        composerPayloadCoordinator.canSend
            && canSendComposerPayload(text: activeComposerDraft, attachments: activeComposerPayloadItems)
    }

    var isSelectedThreadAwaitingInitialHistory: Bool {
        let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines)
        let selectedThreadMessages = threadId.map { cachedMessages(for: $0) } ?? []
        return GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
            threadId: threadId,
            historyLoaded: threadId.map { threadHistoryLoadedIds.contains($0) } ?? false,
            liveRenderSnapshot: threadId.flatMap { renderSnapshotsByThread[$0] },
            cachedTranscript: threadId.flatMap { transcriptMirror.snapshot(for: $0) },
            resolvedMessageIds: Set(selectedThreadMessages.map(\.id)),
            resolvedHistoryIndexes: Set(selectedThreadMessages.compactMap(\.historyIndex)),
            hasRemoteFinalMessages: threadId.map { threadId in
                cachedMessages(for: threadId).contains { $0.localState == .remoteFinal }
            } ?? false
        )
    }

    /// True while the selected thread is fetching its initial transcript,
    /// either during the in-flight request or before the first page has loaded.
    /// This drives header chrome only; the transcript region derives its
    /// single treatment from live Core inputs.
    var isSelectedThreadLoadingInitialHistory: Bool {
        isLoadingSelectedThreadHistory || isSelectedThreadAwaitingInitialHistory
    }

    var hasComposerPayload: Bool {
        !activeComposerDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !activeComposerPayloadItems.isEmpty
    }

    func canSendComposerPayload(text: String, attachments: [GaryxMobileComposerAttachment]) -> Bool {
        let hasPayload = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
        return hasPayload
            && !isNewThreadAgentBindingUnavailable
            && (
            (!isSelectedThreadSending && !isSelectedThreadRemoteBusy)
                || canQueueSelectedThreadInput
        )
    }

    var canQueueSelectedThreadInput: Bool {
        guard let selectedThread else { return false }
        return isThreadBusy(selectedThread.id)
    }

    var isSelectedThreadSending: Bool {
        guard let selectedThread else {
            return false
        }
        return isThreadBusy(selectedThread.id)
    }

    var isSelectedThreadRemoteBusy: Bool {
        guard let selectedThread else { return false }
        return isThreadBusy(selectedThread.id)
    }

    var showsTailThinkingIndicator: Bool {
        guard let threadId = selectedThread?.id else { return false }
        return renderSnapshot(for: threadId)?.tailActivity == .thinking
    }

    /// Quota / rate-limit context for the selected thread's most recent run,
    /// when it terminated because the provider's usage quota was exhausted. The
    /// conversation view renders a countdown banner at the transcript tail.
    var selectedThreadRateLimit: GaryxRenderRateLimit? {
        guard let threadId = selectedThread?.id else { return nil }
        return renderSnapshot(for: threadId)?.rateLimit
    }

    func isThreadBusy(_ threadId: String) -> Bool {
        runTracker.isThreadBusy(threadId)
            || runStateByThread[threadId]?.busy == true
    }

    func canDeleteThread(_ thread: GaryxThreadSummary) -> Bool {
        guard !isThreadBusy(thread.id) else { return false }
        if automations.contains(where: { $0.targetThreadId == thread.id }) {
            return false
        }
        let liveBotKeys = Set(
            configuredBots
                .filter(\.enabled)
                .map { "\($0.channel):\($0.accountId)" }
        )
        if channelEndpoints.contains(where: { endpoint in
            endpoint.threadId == thread.id && liveBotKeys.contains("\(endpoint.channel):\(endpoint.accountId)")
        }) {
            return false
        }
        return true
    }

    func canArchiveThread(_ thread: GaryxThreadSummary) -> Bool {
        canArchiveThreadId(thread.id)
    }

    func canArchiveThreadId(_ threadId: String) -> Bool {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty, !isThreadBusy(normalizedId) else { return false }
        if automations.contains(
            where: { ($0.targetThreadId ?? "").trimmingCharacters(in: .whitespacesAndNewlines) == normalizedId }
        ) {
            return false
        }
        return true
    }

    #if DEBUG
    func startHomeScrollPressureProbeIfRequested() {
        let environment = ProcessInfo.processInfo.environment
        let arguments = CommandLine.arguments
        guard environment["GARYX_MOBILE_HOME_SCROLL_PROBE"] == "1"
            || arguments.contains("--garyx-home-scroll-probe")
        else { return }
        debugSnapshotActive = true
        loadHomeScrollPressureFixture()
        guard environment["GARYX_MOBILE_HOME_SCROLL_PROBE_MANUAL"] != "1" else {
            return
        }
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: 500_000_000)
            guard let self else { return }
            let probe = GaryxHomeScrollPerformanceProbe.shared
            probe.beginWindow(label: "home_scroll_60hz_render_snapshot")
            let threadId = "thread-0"
            for tick in 0..<60 {
                guard !Task.isCancelled else { break }
                renderSnapshotsByThread[threadId] = GaryxRenderSnapshot(
                    basedOnSeq: tick,
                    rows: [],
                    tailActivity: .thinking
                )
                try? await Task.sleep(nanoseconds: 16_666_667)
            }
            _ = probe.endWindow()
        }
    }

    private func loadHomeScrollPressureFixture() {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        let now = Date()
        let avatarDataURLs = [
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mNUcLj0nwEPYGIgAIaHAgBE3AJBVcnK6gAAAABJRU5ErkJggg==",
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mOM8VjwnwEPYGIgAIaHAgBXtgJTMAef0wAAAABJRU5ErkJggg==",
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mOUaYn5z4AHMDEQAMNDAQAOCgILqEOeygAAAABJRU5ErkJggg==",
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mNUcLj0nwEPYGIgAIaHAgBE3AJBVcnK6gAAAABJRU5ErkJggg==",
        ]
        agents = (0..<4).map { index in
            GaryxAgentSummary(
                id: "agent-\(index)",
                displayName: "Synthetic Agent \(index)",
                providerType: "codex",
                model: "gpt-5-codex",
                defaultWorkspaceDir: "/Users/test/workspaces/project-\(index)",
                avatarDataUrl: avatarDataURLs[index],
                builtIn: false,
                standalone: true,
                createdAt: formatter.string(from: now),
                updatedAt: formatter.string(from: now)
            )
        }
        let fixtureThreads = (0..<50).map { index in
            GaryxThreadSummary(
                id: "thread-\(index)",
                title: "Synthetic thread \(index)",
                createdAt: formatter.string(from: now.addingTimeInterval(Double(-index) * 3_600)),
                updatedAt: formatter.string(from: now.addingTimeInterval(Double(-index) * 180)),
                lastMessagePreview: "Synthetic preview \(index)",
                workspacePath: "/Users/test/workspaces/project-\(index % 6)",
                messageCount: 10 + index,
                agentId: "agent-\(index % 4)",
                providerType: "codex",
                recentRunId: "run-\(index)",
                activeRunId: index == 0 ? "run-\(index)" : nil,
                runState: index == 0 ? "running" : "idle",
                worktreePath: nil
            )
        }
        pinnedThreadIds = (0..<6).map { "thread-\($0)" }
        for threadId in (0..<50).reversed().map({ "thread-\($0)" }) {
            recentThreadFeeds.upsertChat(threadId: threadId)
        }
        seedThreadSummariesForTesting(fixtureThreads)
        connectionState = .ready(version: "debug-home-scroll-probe")
        emitHomeProjectionSnapshot()
    }
    #endif

    func sidebarThreadSummary(for threadId: String) -> GaryxThreadSummary? {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return nil }
        if selectedThread?.id == normalizedId {
            return selectedThread
        }
        return threadSummaryCache.summary(for: normalizedId)
    }

    var agentTargets: [GaryxMobileAgentTarget] {
        GaryxMobileAgentTargetMapper.makeTargets(agents: agents)
    }

    var isLoadingRemoteState: Bool {
        remoteStateLoadPhase.isLoading
    }

    var hasResolvedRemoteState: Bool {
        remoteStateLoadPhase.hasResolved
    }

    var isRemoteStatePending: Bool {
        hasGatewaySettings && !remoteStateLoadPhase.hasResolved
    }

    var workspaceCatalog: GaryxWorkspaceCatalog {
        workspaceCatalogState.value
    }

    /// The gateway machine's home directory for `~` abbreviation. Never the
    /// device-local HOME.
    var gatewayHomePath: String? {
        workspaceCatalog.gatewayHome
    }

    var userWorkspacePaths: [String] {
        workspaceCatalog.paths
    }

    var isLoadingWorkspaces: Bool {
        workspaceCatalogState.phase.isLoading
    }

    var workspaceRefreshFailureMessage: String? {
        workspaceCatalogState.lastFailureMessage
    }

    var isLoadingAgentTargets: Bool {
        agentTargetsLoadPhase.isLoading
    }

    var shouldShowAgentTargetsEmptyState: Bool {
        agentTargets.isEmpty && agentTargetsLoadPhase.hasResolved
    }

    var agentTargetsEmptyTitle: String {
        if agentTargetsLoadPhase.failureMessage != nil {
            return "Unable to load agents."
        }
        return "No agents available."
    }

    var agentTargetsEmptyText: String {
        agentTargetsLoadPhase.failureMessage ?? ""
    }

    var agentTargetsPlaceholderText: String {
        if isLoadingAgentTargets || !agentTargetsLoadPhase.hasResolved {
            return "Loading agents..."
        }
        return agentTargetsEmptyTitle
    }

    var selectedAgentTarget: GaryxMobileAgentTarget? {
        GaryxMobileAgentTargetMapper.selectedTarget(
            id: newThreadAgentTargetId(),
            targets: agentTargets
        )
    }

    var selectedAgentLabel: String {
        GaryxMobileAgentTargetMapper.selectedAgentLabel(
            selectedAgentTargetId: newThreadAgentTargetId(),
            target: selectedAgentTarget
        )
    }

    var selectedThreadAgentTarget: GaryxMobileAgentTarget? {
        GaryxMobileAgentTargetMapper.selectedThreadTarget(
            thread: selectedThread,
            selectedAgentTargetId: newThreadAgentTargetId(),
            targets: agentTargets
        )
    }

    var selectedThreadAgentLabel: String {
        GaryxMobileAgentTargetMapper.selectedThreadAgentLabel(
            thread: selectedThread,
            target: selectedThreadAgentTarget,
            fallbackSelectedAgentLabel: selectedAgentLabel
        )
    }

    /// Availability is an admission concern only for a draft that would create
    /// a fresh binding. Existing threads keep their bound agent even when it is
    /// disabled or the catalog has no enabled entries.
    var isNewThreadAgentBindingUnavailable: Bool {
        guard selectedThread == nil, agentTargetsLoadPhase.hasResolved else { return false }
        let botAgentId = currentPendingBotDraft()?.agentId
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let draftOverride = botAgentId.isEmpty ? currentPendingNewThreadAgentTargetId() : botAgentId
        return !GaryxNewThreadAgentSelection.isAvailable(
            draftOverrideAgentId: draftOverride,
            effectiveDefaultAgentId: effectiveDefaultAgentId,
            enabledAgentIds: Set(agentTargets.map(\.id))
        )
    }

    var mobileBotGroups: [GaryxMobileBotGroup] {
        GaryxMobileBotGroupBuilder.groups(
            channelEndpoints: channelEndpoints,
            configuredBots: configuredBots,
            botConsoles: botConsoles,
            channelPlugins: channelPlugins
        )
    }

    var selectedThreadBotGroup: GaryxMobileBotGroup? {
        GaryxMobileBotGroupBuilder.selectedGroup(
            threadId: selectedThread?.id,
            groups: mobileBotGroups
        )
    }

    var enabledAutomationCount: Int {
        automations.filter(\.enabled).count
    }

    var workspacePathSuggestions: [String] {
        GaryxMobileWorkspacePresentation.workspacePathSuggestions(
            threadWorkspacePaths: residentRecentThreadSummaries.map(\.workspacePath),
            threadWorktreePaths: residentRecentThreadSummaries.map(\.worktreePath),
            automationWorkspacePaths: automations.map(\.workspacePath),
            savedWorkspacePaths: userWorkspacePaths,
            additionalPaths: [newThreadWorkspaceSelection.workspacePath ?? "", selectedWorkspacePath]
        )
    }

    var pinnedThreads: [GaryxThreadSummary] {
        threadSummaryCache.summaries(for: pinnedThreadIds)
    }

    var allRecentThreads: [GaryxThreadSummary] {
        threadSummaryCache.summaries(for: allRecentThreadIds)
    }

    var visibleRecentThreads: [GaryxThreadSummary] {
        threadSummaryCache.summaries(for: visibleRecentThreadIds)
    }
}
