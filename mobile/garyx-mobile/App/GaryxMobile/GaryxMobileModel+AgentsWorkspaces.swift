import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WidgetKit

enum GaryxMobileAvatarImageNormalizer {
    enum NormalizationError: LocalizedError {
        case unreadable
        case tooLarge

        var errorDescription: String? {
            switch self {
            case .unreadable:
                "Failed to read avatar image."
            case .tooLarge:
                "Avatar image is too large."
            }
        }
    }

    static func normalizedDataUrl(fromRawValue rawValue: String) throws -> String {
        let trimmed = rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw NormalizationError.unreadable }
        let parts = trimmed.split(separator: ",", maxSplits: 1).map(String.init)
        let encoded = parts.count == 2 ? parts[1] : parts[0]
        guard let sourceData = Data(base64Encoded: encoded) else {
            throw NormalizationError.unreadable
        }
        return try normalizedDataUrl(fromImageData: sourceData)
    }

    static func normalizedDataUrl(fromImageData data: Data) throws -> String {
        guard let sourceImage = UIImage(data: data) else {
            throw NormalizationError.unreadable
        }
        let sourceSize = sourceImage.size
        guard sourceSize.width > 0, sourceSize.height > 0 else {
            throw NormalizationError.unreadable
        }

        let side = CGFloat(avatarImageSize)
        let targetRect = CGRect(x: 0, y: 0, width: side, height: side)
        let scale = max(side / sourceSize.width, side / sourceSize.height)
        let drawSize = CGSize(width: sourceSize.width * scale, height: sourceSize.height * scale)
        let drawRect = CGRect(
            x: (side - drawSize.width) / 2,
            y: (side - drawSize.height) / 2,
            width: drawSize.width,
            height: drawSize.height
        )

        let transparentFormat = UIGraphicsImageRendererFormat()
        transparentFormat.scale = 1
        transparentFormat.opaque = false
        let transparentImage = UIGraphicsImageRenderer(size: targetRect.size, format: transparentFormat).image { context in
            UIColor.clear.setFill()
            context.cgContext.fill(targetRect)
            sourceImage.draw(in: drawRect)
        }
        if let pngData = transparentImage.pngData(), pngData.count <= avatarMaxBytes {
            return dataUrl(mediaType: "image/png", data: pngData)
        }

        let opaqueFormat = UIGraphicsImageRendererFormat()
        opaqueFormat.scale = 1
        opaqueFormat.opaque = true
        let flattenedImage = UIGraphicsImageRenderer(size: targetRect.size, format: opaqueFormat).image { context in
            UIColor(red: 0.969, green: 0.973, blue: 0.980, alpha: 1).setFill()
            context.cgContext.fill(targetRect)
            transparentImage.draw(in: targetRect)
        }
        guard let jpegData = flattenedImage.jpegData(compressionQuality: avatarJPEGQuality),
              jpegData.count <= avatarMaxBytes else {
            throw NormalizationError.tooLarge
        }
        return dataUrl(mediaType: "image/jpeg", data: jpegData)
    }

    private static func dataUrl(mediaType: String, data: Data) -> String {
        "data:\(mediaType);base64,\(data.base64EncodedString())"
    }

    private static var avatarImageSize: Int { 256 }
    private static var avatarMaxBytes: Int { 450 * 1024 }
    private static var avatarJPEGQuality: CGFloat { 0.88 }
}

extension GaryxMobileModel {

    /// Single summary-based open entry (docs/agents/mobile-ui.md: row taps,
    /// widget links, tasks, automations, bot conversations, and deep links
    /// all route through the shared openThread path).
    func openThread(
        _ thread: GaryxThreadSummary,
        source: GaryxMobilePanelOpenSource = .replace
    ) async {
        let resolvedThread = summaryWithCommittedRunState(thread)
        threads = Self.mergedThreadSummaries(threads + [resolvedThread])
        if await openThreadDestination(
            resolvedThread,
            requestId: nil,
            invalidatesPendingThreadOpen: true,
            source: source
        ) {
            return
        }
        // Unknown/missing threadType (e.g. bot-conversation fallback
        // summaries): resolve by id through the shared resolving flow,
        // exactly like id-based opens.
        await openThread(id: resolvedThread.id, source: source)
    }

    func openThread(
        id: String,
        source: GaryxMobilePanelOpenSource = .replace
    ) async {
        let requestId = beginDirectThreadOpen()
        await openThread(id: id, requestId: requestId, source: source)
    }

    func restoreLastOpenedThread(id: String) async {
        let threadId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty, canContinueLastOpenedThreadRestore(threadId: threadId) else { return }

        if let thread = threads.first(where: { $0.id == threadId }),
           await restoreLastOpenedThread(thread, requestedThreadId: threadId) {
            return
        }

        await refreshThreads(source: .userAction)
        guard canContinueLastOpenedThreadRestore(threadId: threadId) else { return }
        if let thread = threads.first(where: { $0.id == threadId }),
           await restoreLastOpenedThread(thread, requestedThreadId: threadId) {
            return
        }

        do {
            let thread = try await client().getThread(threadId: threadId)
            guard canContinueLastOpenedThreadRestore(threadId: threadId) else { return }
            _ = await restoreLastOpenedThread(thread, requestedThreadId: threadId)
        } catch {
            guard canContinueLastOpenedThreadRestore(threadId: threadId) else { return }
            lastError = displayMessage(for: error)
        }
    }

    func queuePendingThreadLink(_ id: String) {
        guard let requestId = threadOpenState.queue(threadId: id, source: .url),
              let threadId = threadOpenState.pendingThreadId else {
            return
        }
        showResolvingWorkflowThread(threadId: threadId, requestId: requestId, source: .replace)
    }

    func openPendingThreadLinkIfNeeded() async {
        guard let threadId = threadOpenState.pendingThreadId else {
            return
        }
        guard case .ready = connectionState else { return }
        let requestId = threadOpenState.requestId
        await openThread(id: threadId, requestId: requestId, source: .replace)
        if isCurrentPendingThreadOpen(requestId), threadHistoryLoadedIds.contains(threadId) {
            completePendingThreadLink(threadId, requestId: requestId)
        }
    }

    private func openThread(
        id: String,
        requestId: UUID,
        source: GaryxMobilePanelOpenSource
    ) async {
        let threadId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return }

        if let thread = threads.first(where: { $0.id == threadId }),
           await openThreadDestination(
                thread,
                requestId: requestId,
                invalidatesPendingThreadOpen: false,
                source: source
           ) {
            return
        }

        showResolvingWorkflowThread(threadId: threadId, requestId: requestId, source: source)
        guard isCurrentPendingThreadOpen(requestId) else { return }

        await refreshThreads(source: .userAction)
        guard isCurrentPendingThreadOpen(requestId) else { return }
        if let thread = threads.first(where: { $0.id == threadId }),
           await openThreadDestination(
                thread,
                requestId: requestId,
                invalidatesPendingThreadOpen: false,
                source: source
           ) {
            return
        }
        do {
            let thread = try await client().getThread(threadId: threadId)
            guard isCurrentPendingThreadOpen(requestId) else { return }
            if await openThreadDestination(
                thread,
                requestId: requestId,
                invalidatesPendingThreadOpen: false,
                source: source
            ) {
                return
            }
            lastError = "Garyx could not resolve thread type for \(threadId)."
        } catch {
            guard isCurrentPendingThreadOpen(requestId) else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func canContinueLastOpenedThreadRestore(threadId: String) -> Bool {
        GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
            persistedLastOpenedThreadId: persistedLastOpenedThreadId,
            persistedLastSessionWasOnThread: persistedLastSessionWasOnThread,
            selectedThreadId: selectedThread?.id,
            hasPendingMobileRoute: pendingMobileRoute != nil,
            hasPendingThreadIntent: threadOpenState.hasPendingIntent,
            navigationState: navigationState,
            sidebarVisible: sidebarVisible
        ) == threadId
    }

    @discardableResult
    private func restoreLastOpenedThread(
        _ thread: GaryxThreadSummary,
        requestedThreadId: String
    ) async -> Bool {
        let destination = GaryxWorkflowRunDestination.destination(for: thread, fallbackThreadId: requestedThreadId)
        guard let restorableThreadId = GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
            persistedLastOpenedThreadId: persistedLastOpenedThreadId,
            persistedLastSessionWasOnThread: persistedLastSessionWasOnThread,
            selectedThreadId: selectedThread?.id,
            hasPendingMobileRoute: pendingMobileRoute != nil,
            hasPendingThreadIntent: threadOpenState.hasPendingIntent,
            navigationState: navigationState,
            sidebarVisible: sidebarVisible,
            resolvedDestination: destination
        ) else {
            markLastOpenedThreadRestoreNonRestorable(destination, requestedThreadId: requestedThreadId)
            return false
        }

        var restoredThread = thread
        if restoredThread.id != restorableThreadId {
            restoredThread = GaryxThreadSummary(
                id: restorableThreadId,
                title: thread.title,
                createdAt: thread.createdAt,
                updatedAt: thread.updatedAt,
                lastMessagePreview: thread.lastMessagePreview,
                workspacePath: thread.workspacePath,
                messageCount: thread.messageCount,
                agentId: thread.agentId,
                providerType: thread.providerType,
                recentRunId: thread.recentRunId,
                activeRunId: thread.activeRunId,
                runState: thread.runState,
                worktreePath: thread.worktreePath,
                automationId: thread.automationId,
                automationThreadMode: thread.automationThreadMode,
                threadType: thread.threadType,
                workflowRunId: thread.workflowRunId,
                excludeFromRecent: thread.excludeFromRecent,
                threadRuntime: thread.threadRuntime
            )
        }
        let resolvedThread = summaryWithCommittedRunState(restoredThread)
        threads = Self.mergedThreadSummaries(threads + [resolvedThread])
        clearWorkflowRunSurface()
        await selectThread(
            resolvedThread,
            invalidatesPendingThreadOpen: false,
            source: .replace
        )
        return true
    }

    private func markLastOpenedThreadRestoreNonRestorable(
        _ destination: GaryxWorkflowRunDestination,
        requestedThreadId: String
    ) {
        clearPersistedLastOpenedThreadId(ifMatches: requestedThreadId)
        switch destination {
        case .workflowRun(let workflowRunId):
            clearPersistedLastOpenedThreadId(ifMatches: workflowRunId)
        case .chat, .unresolved:
            break
        }
        persistLastSessionRestorable(false)
    }

    private func openThreadDestination(
        _ thread: GaryxThreadSummary,
        requestId: UUID?,
        invalidatesPendingThreadOpen: Bool,
        source: GaryxMobilePanelOpenSource
    ) async -> Bool {
        if let requestId {
            guard isCurrentPendingThreadOpen(requestId) else { return false }
        }
        switch GaryxWorkflowRunDestination.destination(for: thread) {
        case .chat:
            clearWorkflowRunSurface()
            await selectThread(
                thread,
                invalidatesPendingThreadOpen: invalidatesPendingThreadOpen,
                source: source
            )
            return true
        case .workflowRun(let workflowRunId):
            await openWorkflowRun(
                workflowRunId: workflowRunId,
                thread: thread,
                invalidatesPendingThreadOpen: invalidatesPendingThreadOpen,
                source: source
            )
            return true
        case .unresolved:
            return false
        }
    }

    private func showPendingThreadLink(
        _ threadId: String,
        requestId: UUID,
        source: GaryxMobilePanelOpenSource
    ) {
        guard threadOpenState.markShown(threadId: threadId, requestId: requestId) else { return }
        let thread = threads.first(where: { $0.id == threadId })
            ?? (selectedThread?.id == threadId ? selectedThread : nil)
            ?? Self.placeholderThreadSummary(id: threadId)
        clearWorkflowRunSurface()
        showSelectedThread(thread, invalidatesPendingThreadOpen: false, source: source)
        lastError = nil
    }

    func completePendingThreadLink(_ threadId: String, requestId: UUID? = nil) {
        threadOpenState.complete(threadId: threadId, requestId: requestId)
    }

    func beginDirectThreadOpen() -> UUID {
        threadOpenState.beginDirectOpen()
    }

    func invalidatePendingThreadOpen() {
        threadOpenState.invalidate()
        pendingMobileRoute = nil
    }

    func isCurrentPendingThreadOpen(_ requestId: UUID) -> Bool {
        threadOpenState.isCurrent(requestId)
    }

    static func placeholderThreadSummary(id: String) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: "Loading thread",
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }

    func setSelectedAgentTarget(_ id: String) {
        selectedAgentTargetId = id
        defaults.set(id, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.selectedAgentTargetId))
    }

    func openAgentChatDraft(_ id: String) {
        let targetId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !targetId.isEmpty else { return }
        openNewThreadDraft(agentTargetOverride: targetId)
    }

    func newThreadAgentTargetId(agentOverride: String? = nil) -> String {
        if let agentOverride {
            let targetId = agentOverride.trimmingCharacters(in: .whitespacesAndNewlines)
            if !targetId.isEmpty {
                return targetId
            }
        }
        let pendingTargetId = currentPendingNewThreadAgentTargetId()
        if !pendingTargetId.isEmpty {
            return pendingTargetId
        }
        return selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func setNewThreadAgentTarget(_ id: String) {
        if pendingNewThreadAgentTargetGeneration == selectedThreadDraftGeneration {
            setPendingNewThreadAgentTarget(id)
        } else {
            setSelectedAgentTarget(id)
        }
        // A model/thinking override only makes sense for the agent it was picked for.
        clearNewThreadModelOverride()
        // The composer's target changed; bind to that target's own draft buffer so
        // one new-thread target's text is never shown or sent under another. Covers
        // both the pending and the selected-default branches above.
        if selectedThread == nil {
            switchComposerDraft(to: newThreadComposerDraftKey)
        }
    }

    var newThreadAgentTarget: GaryxMobileAgentTarget? {
        let targetId = newThreadAgentTargetId()
        guard !targetId.isEmpty else { return nil }
        return agentTargets.first { $0.id == targetId }
    }

    var newThreadProviderModels: GaryxProviderModels? {
        guard let target = newThreadAgentTarget else { return nil }
        let providerType = target.providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !providerType.isEmpty else { return nil }
        return providerModelsByType[providerType]
    }

    /// The model that filters thinking levels for the new-thread draft: the
    /// override when chosen, else the agent's configured model.
    var newThreadEffortFilterModel: String? {
        GaryxThreadModelOverridePresentation.effortFilterModel(
            override: newThreadModelOverride,
            agentConfiguredModel: newThreadAgentTarget?.model,
            providerModels: newThreadProviderModels
        )
    }

    func setNewThreadModelOverride(_ model: String) {
        newThreadModelOverride = model.trimmingCharacters(in: .whitespacesAndNewlines)
        newThreadReasoningEffortOverride = GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
            providerModels: newThreadProviderModels,
            model: newThreadEffortFilterModel,
            reasoningEffort: newThreadReasoningEffortOverride
        ) ?? ""
        newThreadServiceTierOverride = GaryxThreadModelOverridePresentation.sanitizedServiceTier(
            providerModels: newThreadProviderModels,
            model: newThreadEffortFilterModel,
            serviceTier: newThreadServiceTierOverride
        ) ?? ""
    }

    func setNewThreadReasoningEffortOverride(_ effort: String) {
        newThreadReasoningEffortOverride = effort.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func setNewThreadServiceTierOverride(_ tier: String) {
        newThreadServiceTierOverride = tier.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func clearNewThreadModelOverride() {
        newThreadModelOverride = ""
        newThreadReasoningEffortOverride = ""
        newThreadServiceTierOverride = ""
    }

    func ensureNewThreadProviderModelsLoaded() async {
        guard let target = newThreadAgentTarget else { return }
        let providerType = target.providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !providerType.isEmpty, providerModelsByType[providerType] == nil else { return }
        await loadProviderModels(providerType: providerType)
    }

    func setPendingNewThreadAgentTarget(_ id: String?) {
        let targetId = id?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !targetId.isEmpty else {
            clearPendingNewThreadAgentTarget()
            return
        }
        pendingNewThreadAgentTargetId = targetId
        pendingNewThreadAgentTargetGeneration = selectedThreadDraftGeneration
    }

    func clearPendingNewThreadAgentTarget() {
        pendingNewThreadAgentTargetId = nil
        pendingNewThreadAgentTargetGeneration = nil
    }

    func currentPendingNewThreadAgentTargetId() -> String {
        guard pendingNewThreadAgentTargetGeneration == selectedThreadDraftGeneration else {
            return ""
        }
        return pendingNewThreadAgentTargetId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    func setNewThreadWorkspace(_ path: String) {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        newThreadWorkspace = trimmed
        if trimmed.isEmpty {
            newThreadWorkspaceMode = "local"
        }
        saveGatewayScopedUserState()
        if !trimmed.isEmpty, workspaceGitStatuses[trimmed] == nil {
            Task { await refreshWorkspaceGitStatus(for: trimmed) }
        }
    }

    func refreshWorkspaces() async {
        guard hasGatewaySettings else { return }
        guard workspaceRefreshRequestId == nil else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let requestId = UUID()
        workspaceRefreshRequestId = requestId
        beginWorkspaceCatalogRefresh()
        do {
            let workspaces = try await client().listWorkspaces()
            guard isCurrentWorkspaceRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            workspaceRefreshRequestId = nil
            applyWorkspaceSummaries(workspaces, persist: true)
            ensureSelectedWorkspace()
        } catch {
            guard isCurrentWorkspaceRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            workspaceRefreshRequestId = nil
            let message = displayMessage(for: error)
            failWorkspaceCatalogRefresh(message)
        }
    }

    func isCurrentWorkspaceRefresh(_ requestId: UUID, runtimeGeneration: UUID) -> Bool {
        runtimeGeneration == gatewayRuntimeGeneration && workspaceRefreshRequestId == requestId
    }

    @discardableResult
    func addUserWorkspacePath(_ path: String) async -> String? {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let workspaces = try await client().addWorkspace(path: trimmed, name: trimmed.garyxLastPathComponent)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            applyWorkspaceSummaries(workspaces, persist: true)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            lastError = error.localizedDescription
            return nil
        }
        if workspaceGitStatuses[trimmed] == nil {
            Task { await refreshWorkspaceGitStatus(for: trimmed) }
        }
        return trimmed
    }

    func listWorkspaceDirectories(path: String?) async throws -> GaryxWorkspaceDirectoryListing {
        try await client().listWorkspaceDirectories(path: path)
    }

    func setNewThreadWorkspaceMode(_ mode: String) {
        guard selectedThread == nil, !isSending, activeRunThreadId == nil else { return }
        let normalized = Self.normalizedWorkspaceMode(mode)
        guard normalized != "worktree" || newThreadWorkspaceCanUseWorktree else { return }
        newThreadWorkspaceMode = normalized
        if newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            newThreadWorkspaceMode = "local"
        }
        saveGatewayScopedUserState()
    }

    var newThreadWorkspaceLabel: String {
        let workspace = newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = (workspace as NSString).lastPathComponent
        return workspace.isEmpty ? "No workspace" : (name.isEmpty ? workspace : name)
    }

    var newThreadWorkspaceCanUseWorktree: Bool {
        let workspace = newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty else { return false }
        return workspaceGitStatuses[workspace]?.canUseWorktree == true
    }

    var newThreadUsesWorktree: Bool {
        Self.normalizedWorkspaceMode(newThreadWorkspaceMode) == "worktree" && newThreadWorkspaceCanUseWorktree
    }

    func refreshWorkspaceGitStatus(for path: String) async {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            let status = try await client().workspaceGitStatus(workspaceDir: trimmed)
            workspaceGitStatuses[trimmed] = status
            if !status.canUseWorktree, newThreadWorkspace == trimmed {
                setNewThreadWorkspaceMode("local")
            }
        } catch {
            // Workspace status is an affordance for the mode selector; keep chat usable if it fails.
        }
    }

    func generateAvatar(
        identifier: String,
        displayName: String,
        stylePrompt: String
    ) async -> String? {
        let trimmedId = identifier.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedId.isEmpty || !trimmedName.isEmpty else {
            lastError = "Agent name is required"
            return nil
        }
        let prompt = GaryxAvatarPromptBuilder.prompt(
            displayName: trimmedName,
            identifier: trimmedId,
            stylePrompt: stylePrompt
        )
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let generated = try await client().generateAvatar(prompt: prompt)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            let avatarDataUrl = generated.avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !avatarDataUrl.isEmpty else {
                lastError = "Image generation did not return an avatar."
                return nil
            }
            do {
                return try GaryxMobileAvatarImageNormalizer.normalizedDataUrl(fromRawValue: avatarDataUrl)
            } catch {
                lastError = error.localizedDescription
                return nil
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            lastError = displayMessage(for: error)
            return nil
        }
    }

    func createAgent(
        agentId: String,
        displayName: String,
        providerType: String,
        modelName: String,
        modelReasoningEffort: String = "",
        workspace: String,
        avatarDataUrl: String,
        systemPrompt: String,
        env: [String: String] = [:]
    ) async -> Bool {
        let agentId = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let provider = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        let model = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        let reasoningEffort = modelReasoningEffort.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let avatarDataUrl = avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = systemPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !agentId.isEmpty, !displayName.isEmpty, !provider.isEmpty else { return false }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let agent = try await client().createAgent(
                GaryxCustomAgentRequest(
                    agentId: agentId,
                    displayName: displayName,
                    providerType: provider,
                    model: model,
                    modelReasoningEffort: reasoningEffort,
                    providerEnv: env.isEmpty ? nil : env,
                    defaultWorkspaceDir: workspace.isEmpty ? nil : workspace,
                    avatarDataUrl: avatarDataUrl.isEmpty ? nil : avatarDataUrl,
                    systemPrompt: prompt
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            await storeAvatarIfPresent(id: agent.id, dataUrl: agent.avatarDataUrl.isEmpty ? avatarDataUrl : agent.avatarDataUrl, sourceUpdatedAt: agent.updatedAt)
            replaceAgent(agent)
            setSelectedAgentTarget(agent.id)
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    /// Fetch an agent's authoritative env for seeding the editor. When the
    /// catalog was restored from a cache snapshot (which strips provider_env),
    /// re-fetch the live agent; otherwise the in-memory agent is authoritative.
    func authoritativeProviderEnv(for agent: GaryxAgentSummary) async -> [String: String] {
        guard catalogSnapshotRestored else { return agent.providerEnv }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let latestAgents = try await client().listAgents()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return agent.providerEnv }
            return latestAgents.first(where: { $0.id == agent.id })?.providerEnv ?? agent.providerEnv
        } catch {
            return agent.providerEnv
        }
    }

    func updateAgent(
        _ agent: GaryxAgentSummary,
        agentId: String,
        displayName: String,
        providerType: String,
        modelName: String,
        modelReasoningEffort: String? = nil,
        workspace: String,
        avatarDataUrl: String,
        clearsAvatar: Bool = false,
        systemPrompt: String,
        envIntent: GaryxAgentEnvIntent = .unchanged
    ) async -> GaryxAgentSummary? {
        let nextAgentId = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDisplayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextProviderType = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextModelName = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextAvatarDataUrl = avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextAgentId.isEmpty, !nextDisplayName.isEmpty, !nextProviderType.isEmpty else { return nil }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            var baseAgent = agent
            // Restored rows are display projections; a missing updatedAt also
            // means the row cannot vouch for the stored state. Both cases must
            // re-fetch before building a conditional update.
            if catalogSnapshotRestored || agent.updatedAt == nil {
                let latestAgents = try await client().listAgents()
                guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
                guard let latestAgent = latestAgents.first(where: { $0.id == agent.id }) else {
                    lastError = "Agent details are still loading. Try again after refresh."
                    return nil
                }
                baseAgent = latestAgent
            }
            let nextSystemPrompt = systemPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
            // nil keeps the stored thinking level; an explicit value (or "") replaces it.
            let nextReasoningEffort = modelReasoningEffort?.trimmingCharacters(in: .whitespacesAndNewlines)
                ?? baseAgent.modelReasoningEffort
            let requestAvatarDataUrl: String?
            if nextAvatarDataUrl.isEmpty {
                requestAvatarDataUrl = clearsAvatar ? "" : nil
            } else {
                requestAvatarDataUrl = nextAvatarDataUrl
            }
            // Env is expressed as an explicit intent: `.unchanged` omits
            // provider_env (gateway preserves the stored value), `.clear` sends
            // an empty map, `.replace` sends the full desired map. This never
            // re-sends a possibly-stale baseAgent snapshot for an untouched edit.
            let providerEnvRequestValue: [String: String]?
            switch envIntent {
            case .unchanged:
                providerEnvRequestValue = nil
            case .clear:
                providerEnvRequestValue = [:]
            case .replace(let map):
                providerEnvRequestValue = map
            }
            let updated = try await client().updateAgent(
                agentId: agent.id,
                request: GaryxCustomAgentRequest(
                    agentId: nextAgentId,
                    displayName: nextDisplayName,
                    providerType: nextProviderType,
                    model: nextModelName,
                    modelReasoningEffort: nextReasoningEffort,
                    modelServiceTier: baseAgent.modelServiceTier,
                    providerEnv: providerEnvRequestValue,
                    authSource: baseAgent.authSource.isEmpty ? nil : baseAgent.authSource,
                    baseUrl: baseAgent.baseUrl.isEmpty ? nil : baseAgent.baseUrl,
                    codexHome: baseAgent.codexHome.isEmpty ? nil : baseAgent.codexHome,
                    maxToolIterations: baseAgent.maxToolIterations,
                    requestTimeoutSeconds: baseAgent.requestTimeoutSeconds,
                    defaultWorkspaceDir: nextWorkspace.isEmpty ? nil : nextWorkspace,
                    avatarDataUrl: requestAvatarDataUrl,
                    systemPrompt: nextSystemPrompt,
                    expectedUpdatedAt: baseAgent.updatedAt
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            let didClearAvatar = clearsAvatar && nextAvatarDataUrl.isEmpty
            let didStoreAvatar: Bool
            if didClearAvatar {
                await removeAvatar(id: updated.id)
                didStoreAvatar = false
            } else {
                didStoreAvatar = await storeAvatarIfPresent(id: updated.id, dataUrl: updated.avatarDataUrl.isEmpty ? nextAvatarDataUrl : updated.avatarDataUrl, sourceUpdatedAt: updated.updatedAt)
            }
            if updated.id != agent.id, didClearAvatar || didStoreAvatar {
                await removeAvatar(id: agent.id)
            }
            replaceAgent(updated, replacing: agent.id)
            setSelectedAgentTarget(updated.id)
            return updated
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            lastError = displayMessage(for: error)
            return nil
        }
    }


    func deleteAgent(_ agent: GaryxAgentSummary) async {
        guard !agent.builtIn else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().deleteAgent(agentId: agent.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await removeAvatar(id: agent.id)
            agents.removeAll { $0.id == agent.id }
            ensureSelectedAgentTarget()
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func loadProviderModels(
        providerType: String,
        runtimeGeneration: UUID? = nil,
        remoteStateRefreshRequestId: UUID? = nil
    ) async {
        let provider = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !provider.isEmpty else { return }
        let observedGeneration = runtimeGeneration ?? gatewayRuntimeGeneration
        do {
            let models = try await client().providerModels(providerType: provider)
            guard observedGeneration == gatewayRuntimeGeneration,
                  isCurrentRemoteStateScopedRequest(remoteStateRefreshRequestId) else {
                return
            }
            providerModelsByType[provider] = models
        } catch {
            guard observedGeneration == gatewayRuntimeGeneration,
                  isCurrentRemoteStateScopedRequest(remoteStateRefreshRequestId) else {
                return
            }
            lastError = displayMessage(for: error)
        }
    }

    /// Fetches the authoritative gateway settings document before opening a
    /// provider editor, so the sheet echoes the real current API key / base URL
    /// / auth source instead of a possibly cache-restored projection (the
    /// mobile-ui "fetch authoritative data before saving" contract).
    func refreshAuthoritativeGatewaySettings() async -> Bool {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let settings = try await client().gatewaySettings()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            gatewaySettingsDocument = settings
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func updateModelProviderDefaults(
        provider: GaryxModelProviderDefault,
        request: GaryxProviderSettingsPresentation.SaveRequest
    ) async -> Bool {
        let nextModel = request.modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextReasoningEffort = request.reasoningEffort.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            var patch: [String: GaryxJSONValue] = [:]
            GaryxModelProviderDefaults.update(
                settings: &patch,
                provider: provider,
                model: nextModel,
                reasoningEffort: nextReasoningEffort,
                serviceTier: request.serviceTier,
                authSource: request.authSource,
                baseUrl: request.baseUrl,
                apiKey: request.apiKey
            )
            _ = try await client().saveGatewaySettings(patch, merge: true)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            GaryxModelProviderDefaults.update(
                settings: &gatewaySettingsDocument,
                provider: provider,
                model: nextModel,
                reasoningEffort: nextReasoningEffort,
                serviceTier: request.serviceTier,
                authSource: request.authSource,
                baseUrl: request.baseUrl,
                apiKey: request.apiKey
            )
            providerModelsByType.removeValue(forKey: provider.providerType)
            await loadProviderModels(providerType: provider.providerType, runtimeGeneration: runtimeGeneration)
            await refreshRemoteState()
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }


    func selectWorkspace(_ path: String) async {
        selectedWorkspacePath = path
        draftWorkspacePath = path
        selectedWorkspaceDirectory = ""
        workspaceListing = nil
        workspacePreview = nil
        workspaceUploadStatus = nil
        await refreshSelectedWorkspace()
    }

    func prepareWorkspaceBrowser() async {
        ensureSelectedWorkspace()
        guard !selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        await refreshSelectedWorkspace()
    }

    func selectDraftWorkspace() async {
        let path = draftWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { return }
        await selectWorkspace(path)
    }

    func refreshSelectedWorkspace() async {
        let path = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { return }
        let directory = selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let gateway = try client()
            async let listingResult = gateway.listWorkspaceFiles(
                workspaceDir: path,
                directoryPath: directory.isEmpty ? nil : directory
            )
            async let gitStatusResult = gateway.workspaceGitStatus(workspaceDir: path)
            let listing = try await listingResult
            guard isCurrentWorkspaceRequest(
                workspace: path,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspaceListing = listing
            if let status = try? await gitStatusResult {
                guard isCurrentWorkspaceRequest(
                    workspace: path,
                    directory: directory,
                    runtimeGeneration: runtimeGeneration
                ) else { return }
                workspaceGitStatuses[path] = status
            }
        } catch {
            guard isCurrentWorkspaceRequest(
                workspace: path,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            lastError = displayMessage(for: error)
        }
    }

    func openWorkspaceEntry(_ entry: GaryxWorkspaceFileEntry) async {
        guard !selectedWorkspacePath.isEmpty else { return }
        if entry.entryType == "directory" {
            selectedWorkspaceDirectory = entry.path
            workspaceListing = nil
            workspacePreview = nil
            await refreshSelectedWorkspace()
            return
        }
        let workspace = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let directory = selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let preview = try await client().previewWorkspaceFile(
                workspaceDir: workspace,
                path: entry.path
            )
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspacePreview = preview
        } catch {
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            lastError = displayMessage(for: error)
        }
    }

    func openLocalFilePreview(_ target: String) async {
        guard let resolved = GaryxMobileFileLink.previewTarget(
            fromLink: target,
            workspacePaths: workspacePathSuggestions
        ) else {
            lastError = "Garyx could not resolve this local file for preview."
            return
        }
        await openWorkspaceFilePreview(resolved)
    }

    func localFilePreview(_ target: String, reportsError: Bool = true) async -> GaryxWorkspaceFilePreview? {
        // Relative targets (bare `docs/a.md` paths or relative markdown links
        // in transcripts) resolve against the selected thread's workspace;
        // absolute targets ignore this.
        let threadWorkspace = selectedThread?.workspacePath?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard let resolved = GaryxMobileFileLink.previewTarget(
            fromLink: target,
            workspacePaths: workspacePathSuggestions,
            currentWorkspaceDir: threadWorkspace.isEmpty ? nil : threadWorkspace
        ) else {
            if reportsError {
                lastError = "Garyx could not resolve this local file for preview."
            }
            return nil
        }
        return await workspaceFilePreview(resolved, reportsError: reportsError)
    }

    func openWorkspacePreviewLink(
        _ target: String,
        from preview: GaryxWorkspaceFilePreview
    ) async {
        let workspacePaths = workspacePathSuggestions + [preview.workspaceDir]
        guard let resolved = GaryxMobileFileLink.previewTarget(
            fromLink: target,
            workspacePaths: workspacePaths,
            currentWorkspaceDir: preview.workspaceDir,
            currentFilePath: preview.path
        ) else {
            lastError = "Garyx could not resolve this local file for preview."
            return
        }
        await openWorkspaceFilePreview(resolved)
    }

    func workspaceFilePreviewLink(
        _ target: String,
        from preview: GaryxWorkspaceFilePreview,
        reportsError: Bool = true
    ) async -> GaryxWorkspaceFilePreview? {
        let workspacePaths = workspacePathSuggestions + [preview.workspaceDir]
        guard let resolved = GaryxMobileFileLink.previewTarget(
            fromLink: target,
            workspacePaths: workspacePaths,
            currentWorkspaceDir: preview.workspaceDir,
            currentFilePath: preview.path
        ) else {
            if reportsError {
                lastError = "Garyx could not resolve this local file for preview."
            }
            return nil
        }
        return await workspaceFilePreview(resolved, reportsError: reportsError)
    }

    func openWorkspaceFilePreview(
        _ target: GaryxMobileWorkspaceFileTarget,
        source: GaryxMobilePanelOpenSource = .current
    ) async {
        let workspace = target.workspaceDir.trimmingCharacters(in: .whitespacesAndNewlines)
        let filePath = target.path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, !filePath.isEmpty else { return }

        let directory = Self.workspaceDirectory(forFilePath: filePath)
        selectedWorkspacePath = workspace
        draftWorkspacePath = workspace
        selectedWorkspaceDirectory = directory
        workspaceListing = nil
        workspacePreview = nil
        workspaceUploadStatus = nil
        workspaceBotsDrilldown = nil
        openWorkspaceFilesPanel(source: source)

        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let gateway = try client()
            async let listingResult: GaryxWorkspaceFileListing? = try? gateway.listWorkspaceFiles(
                workspaceDir: workspace,
                directoryPath: directory.isEmpty ? nil : directory
            )
            let preview = try await gateway.previewWorkspaceFile(
                workspaceDir: workspace,
                path: filePath
            )
            let listing = await listingResult
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspacePreview = preview
            if let listing {
                workspaceListing = listing
            }
        } catch {
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func workspaceFilePreview(
        _ target: GaryxMobileWorkspaceFileTarget,
        reportsError: Bool = true
    ) async -> GaryxWorkspaceFilePreview? {
        let workspace = target.workspaceDir.trimmingCharacters(in: .whitespacesAndNewlines)
        let filePath = target.path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, !filePath.isEmpty else { return nil }

        do {
            return try await client().previewWorkspaceFile(
                workspaceDir: workspace,
                path: filePath
            )
        } catch {
            if reportsError {
                lastError = displayMessage(for: error)
            }
            return nil
        }
    }

    static func workspaceDirectory(forFilePath filePath: String) -> String {
        let parent = (filePath.trimmingCharacters(in: .whitespacesAndNewlines) as NSString).deletingLastPathComponent
        return parent == "." ? "" : parent
    }

    func goUpWorkspaceDirectory() async {
        guard !selectedWorkspaceDirectory.isEmpty else { return }
        let parent = (selectedWorkspaceDirectory as NSString).deletingLastPathComponent
        selectedWorkspaceDirectory = parent == "." ? "" : parent
        workspaceListing = nil
        workspacePreview = nil
        await refreshSelectedWorkspace()
    }

    func uploadFilesToSelectedWorkspace(from urls: [URL]) async {
        let workspace = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, !urls.isEmpty else { return }
        let directory = selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeGeneration = gatewayRuntimeGeneration
        isUploadingWorkspaceFiles = true
        workspaceUploadStatus = nil
        defer { isUploadingWorkspaceFiles = false }
        do {
            var files: [GaryxUploadFileBlob] = []
            for url in urls {
                let didStartAccess = url.startAccessingSecurityScopedResource()
                defer {
                    if didStartAccess {
                        url.stopAccessingSecurityScopedResource()
                    }
                }
                let values = try url.resourceValues(forKeys: [.isDirectoryKey, .nameKey, .contentTypeKey])
                if values.isDirectory == true {
                    continue
                }
                let data = try Data(contentsOf: url)
                let name = (values.name ?? url.lastPathComponent).trimmingCharacters(in: .whitespacesAndNewlines)
                guard !name.isEmpty else { continue }
                let mediaType = values.contentType?.preferredMIMEType
                    ?? UTType(filenameExtension: (name as NSString).pathExtension)?.preferredMIMEType
                files.append(
                    GaryxUploadFileBlob(
                        name: name,
                        mediaType: mediaType,
                        dataBase64: data.base64EncodedString()
                    )
                )
            }
            guard !files.isEmpty else {
                guard isCurrentWorkspaceRequest(
                    workspace: workspace,
                    directory: directory,
                    runtimeGeneration: runtimeGeneration
                ) else { return }
                workspaceUploadStatus = "No files selected"
                return
            }
            let result = try await client().uploadWorkspaceFiles(
                GaryxUploadWorkspaceFilesRequest(
                    workspaceDir: workspace,
                    path: directory.isEmpty ? nil : directory,
                    files: files
                )
            )
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspaceUploadStatus = files.count == 1 ? "Uploaded \(files[0].name)" : "Uploaded \(files.count) files"
            await refreshSelectedWorkspace()
            if let firstPath = result.uploadedPaths.first?.trimmingCharacters(in: .whitespacesAndNewlines),
               !firstPath.isEmpty {
                let preview = try? await client().previewWorkspaceFile(workspaceDir: workspace, path: firstPath)
                guard isCurrentWorkspaceRequest(
                    workspace: workspace,
                    directory: directory,
                    runtimeGeneration: runtimeGeneration
                ) else { return }
                workspacePreview = preview
            }
        } catch {
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspaceUploadStatus = nil
            lastError = displayMessage(for: error)
        }
    }

    func isCurrentWorkspaceRequest(
        workspace: String,
        directory: String,
        runtimeGeneration: UUID
    ) -> Bool {
        runtimeGeneration == gatewayRuntimeGeneration
            && selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines) == workspace
            && selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines) == directory
    }

    func refreshProviderModelsForVisibleAgents(
        runtimeGeneration: UUID? = nil,
        remoteStateRefreshRequestId: UUID? = nil
    ) async {
        let providerTypes = Set(agents.map(\.providerType).filter { !$0.isEmpty })
        for providerType in providerTypes where providerModelsByType[providerType] == nil {
            guard isCurrentRemoteStateScopedRequest(remoteStateRefreshRequestId) else { return }
            await loadProviderModels(
                providerType: providerType,
                runtimeGeneration: runtimeGeneration,
                remoteStateRefreshRequestId: remoteStateRefreshRequestId
            )
        }
    }

    func replaceAgent(_ agent: GaryxAgentSummary, replacing oldId: String? = nil) {
        var next = agents
        if let oldId = oldId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !oldId.isEmpty,
           oldId != agent.id {
            next.removeAll { $0.id == oldId }
        }
        if let index = next.firstIndex(where: { $0.id == agent.id }) {
            next[index] = agent
        } else {
            next.insert(agent, at: 0)
        }
        if next != agents {
            agents = next
        }
        if !threads.isEmpty {
            persistRecentThreadsWidgetSnapshot()
        }
        persistCatalogCacheSnapshot()
    }

    @discardableResult
    func storeAvatarIfPresent(
        id: String,
        dataUrl: String,
        sourceUpdatedAt: String? = nil
    ) async -> Bool {
        let identity = GaryxAvatarIdentity(scope: currentGatewayScopeId, id: id)
        guard identity.isUsable else { return false }
        let dataUrl = dataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !dataUrl.isEmpty else { return false }
        await avatarStore.upsert(
            [GaryxAvatarUpsert(identity: identity, dataUrl: dataUrl, sourceUpdatedAt: sourceUpdatedAt)],
            validator: GaryxAvatarCGImageValidator(),
            now: Date()
        )
        return !(await avatarStore.avatarFingerprints(for: [identity], now: Date())).isEmpty
    }

    func removeAvatar(id: String) async {
        let identity = GaryxAvatarIdentity(scope: currentGatewayScopeId, id: id)
        guard identity.isUsable else { return }
        await avatarStore.remove(identity)
        avatarImageProvider.invalidate(identity: identity)
    }
}
