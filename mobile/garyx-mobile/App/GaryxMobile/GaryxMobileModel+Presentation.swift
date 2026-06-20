import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
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

    var isGatewayConnectionReady: Bool {
        if case .ready = connectionState {
            return true
        }
        return false
    }

    var canSend: Bool {
        canSendComposerPayload(text: activeComposerDraft, attachments: composerAttachments)
    }

    var isSelectedThreadAwaitingInitialHistory: Bool {
        let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines)
        return GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
            threadId: threadId,
            historyLoaded: threadId.map { threadHistoryLoadedIds.contains($0) } ?? false,
            liveRenderSnapshot: threadId.flatMap { renderSnapshotsByThread[$0] },
            cachedTranscript: threadId.flatMap { cachedTranscriptSnapshots[$0] },
            hasRemoteFinalMessages: threadId.map { threadId in
                cachedMessages(for: threadId).contains { $0.localState == .remoteFinal }
            } ?? false
        )
    }

    /// True while the selected thread is fetching its initial transcript, either
    /// during the in-flight request or before the first page has loaded. Drives
    /// the empty-state loading view and the toolbar loading indicator together.
    var isSelectedThreadLoadingInitialHistory: Bool {
        isLoadingSelectedThreadHistory || isSelectedThreadAwaitingInitialHistory
    }

    var hasComposerPayload: Bool {
        !activeComposerDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !composerAttachments.isEmpty
    }

    func canSendComposerPayload(text: String, attachments: [GaryxMobileComposerAttachment]) -> Bool {
        let hasPayload = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
        return hasPayload && (
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

    func sidebarThreadSummary(for threadId: String) -> GaryxThreadSummary? {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return nil }
        if selectedThread?.id == normalizedId {
            return selectedThread
        }
        if let thread = threads.first(where: { $0.id == normalizedId }) {
            return thread
        }
        if let thread = recentThreads.first(where: { $0.id == normalizedId }) {
            return thread
        }
        if let thread = pinnedThreads.first(where: { $0.id == normalizedId }) {
            return thread
        }
        return nil
    }

    var agentTargets: [GaryxMobileAgentTarget] {
        GaryxMobileAgentTargetMapper.makeTargets(agents: agents, teams: teams)
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

    var userWorkspacePaths: [String] {
        workspaceCatalogState.value
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
            id: selectedAgentTargetId,
            targets: agentTargets
        )
    }

    var selectedAgentLabel: String {
        GaryxMobileAgentTargetMapper.selectedAgentLabel(
            selectedAgentTargetId: selectedAgentTargetId,
            target: selectedAgentTarget
        )
    }

    var selectedThreadAgentTarget: GaryxMobileAgentTarget? {
        GaryxMobileAgentTargetMapper.selectedThreadTarget(
            thread: selectedThread,
            selectedAgentTargetId: selectedAgentTargetId,
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

    var activeTaskCount: Int {
        tasks.filter { $0.status != .done }.count
    }

    var selectedThreadTasks: [GaryxTaskSummary] {
        guard let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return []
        }
        return GaryxMobileTasksPanelState.sourceThreadTasks(tasks, sourceThreadId: threadId)
    }

    var selectedThreadTasksMenuTitle: String {
        GaryxMobileTasksPanelState.viewTasksMenuTitle(count: selectedThreadTasks.count)
    }

    var visibleTasks: [GaryxTaskSummary] {
        tasksPanelState.visibleTasks(from: tasks)
    }

    var visibleTaskCount: Int {
        visibleTasks.count
    }

    var visibleActiveTaskCount: Int {
        visibleTasks.filter { $0.status != .done }.count
    }

    var tasksPanelSubtitle: String {
        if tasksPanelState.isSourceThreadFilterActive {
            return "\(visibleActiveTaskCount) active / \(visibleTaskCount) from thread"
        }
        return "\(activeTaskCount) active / \(tasks.count) total"
    }

    var enabledAutomationCount: Int {
        automations.filter(\.enabled).count
    }

    var workspacePathSuggestions: [String] {
        GaryxMobileWorkspacePresentation.workspacePathSuggestions(
            threadWorkspacePaths: threads.map(\.workspacePath),
            threadWorktreePaths: threads.map(\.worktreePath),
            automationWorkspacePaths: automations.map(\.workspacePath),
            savedWorkspacePaths: userWorkspacePaths,
            additionalPaths: [newThreadWorkspace, selectedWorkspacePath]
        )
    }

    var pinnedThreads: [GaryxThreadSummary] {
        var byId: [String: GaryxThreadSummary] = [:]
        for thread in threads {
            byId[thread.id] = thread
        }
        return pinnedThreadIds.compactMap { byId[$0] }
    }

    var recentThreads: [GaryxThreadSummary] {
        var byId: [String: GaryxThreadSummary] = [:]
        for thread in threads {
            byId[thread.id] = thread
        }
        return recentThreadIds.compactMap { byId[$0] }
    }
}
