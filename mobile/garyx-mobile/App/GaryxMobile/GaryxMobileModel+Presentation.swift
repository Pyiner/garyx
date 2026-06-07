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

    var canSend: Bool {
        canSendComposerPayload(text: draft, attachments: composerAttachments)
    }

    var isSelectedThreadAwaitingInitialHistory: Bool {
        guard let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty,
              cachedMessages(for: threadId).isEmpty else {
            return false
        }
        return !threadHistoryLoadedIds.contains(threadId)
    }

    var hasComposerPayload: Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !composerAttachments.isEmpty
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
        return (isSending && activeRunThreadId == selectedThread.id)
            || remoteBusyThreadIds.contains(selectedThread.id)
            || threads.contains { thread in
                thread.id == selectedThread.id
                    && !(thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true)
            }
    }

    var isSelectedThreadRemoteBusy: Bool {
        guard let selectedThread else { return false }
        return remoteBusyThreadIds.contains(selectedThread.id)
    }

    var showsTailThinkingIndicator: Bool {
        GaryxMobileThreadActivityModel.showsTailThinkingIndicator(
            messages: messages,
            runActive: isSelectedThreadSending
        )
    }

    func isThreadBusy(_ threadId: String) -> Bool {
        activeRunThreadId == threadId
            || remoteBusyThreadIds.contains(threadId)
            || threads.contains { thread in
                thread.id == threadId
                    && !(thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true)
            }
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
            autoResearchWorkspaceDirs: autoResearchRuns.map(\.workspaceDir),
            savedWorkspacePaths: userWorkspacePaths,
            additionalPaths: [newThreadWorkspace, selectedWorkspacePath]
        )
    }

    var runningResearchCount: Int {
        autoResearchRuns.filter { run in
            !garyxAutoResearchIsTerminal(run.state)
        }.count
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
