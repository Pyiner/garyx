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
        cacheThreadSummaries([resolvedThread])
        await selectThread(
            resolvedThread,
            invalidatesPendingThreadOpen: true,
            source: source
        )
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

        if let thread = cachedThreadSummary(for: threadId),
           await restoreLastOpenedThread(thread, requestedThreadId: threadId) {
            return
        }

        await refreshThreads(source: .userAction)
        guard canContinueLastOpenedThreadRestore(threadId: threadId) else { return }
        if let thread = cachedThreadSummary(for: threadId),
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
        showPendingThreadLink(threadId, requestId: requestId, source: .replace)
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

        if let thread = cachedThreadSummary(for: threadId) {
            await selectThread(
                thread,
                invalidatesPendingThreadOpen: false,
                source: source
            )
            return
        }

        showPendingThreadLink(threadId, requestId: requestId, source: source)
        guard isCurrentPendingThreadOpen(requestId) else { return }

        await refreshThreads(source: .userAction)
        guard isCurrentPendingThreadOpen(requestId) else { return }
        if let thread = cachedThreadSummary(for: threadId) {
            await selectThread(
                thread,
                invalidatesPendingThreadOpen: false,
                source: source
            )
            return
        }
        do {
            let thread = try await client().getThread(threadId: threadId)
            guard isCurrentPendingThreadOpen(requestId) else { return }
            cacheThreadSummaries([thread])
            await selectThread(
                thread,
                invalidatesPendingThreadOpen: false,
                source: source
            )
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
        guard let restorableThreadId = GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
            persistedLastOpenedThreadId: persistedLastOpenedThreadId,
            persistedLastSessionWasOnThread: persistedLastSessionWasOnThread,
            selectedThreadId: selectedThread?.id,
            hasPendingMobileRoute: pendingMobileRoute != nil,
            hasPendingThreadIntent: threadOpenState.hasPendingIntent,
            navigationState: navigationState,
            sidebarVisible: sidebarVisible
        ) else {
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
                threadRuntime: thread.threadRuntime
            )
        }
        let resolvedThread = summaryWithCommittedRunState(restoredThread)
        cacheThreadSummaries([resolvedThread])
        await selectThread(
            resolvedThread,
            invalidatesPendingThreadOpen: false,
            source: .replace
        )
        return true
    }

    private func showPendingThreadLink(
        _ threadId: String,
        requestId: UUID,
        source: GaryxMobilePanelOpenSource
    ) {
        guard threadOpenState.markShown(threadId: threadId, requestId: requestId) else { return }
        let thread = cachedThreadSummary(for: threadId)
            ?? (selectedThread?.id == threadId ? selectedThread : nil)
            ?? Self.placeholderThreadSummary(id: threadId)
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
        if frozenNewThreadAgentTargetGeneration == selectedThreadDraftGeneration {
            return currentPendingNewThreadAgentTargetId()
        }
        return GaryxNewThreadAgentSelection.agentId(
            draftOverrideAgentId: currentPendingNewThreadAgentTargetId(),
            effectiveDefaultAgentId: effectiveDefaultAgentId
        ) ?? ""
    }

    func setNewThreadAgentTarget(_ id: String) {
        let previousKey = selectedThread == nil ? newThreadComposerPayloadKey : nil
        setPendingNewThreadAgentTarget(id)
        // A model/thinking override only makes sense for the agent it was picked for.
        clearNewThreadModelOverride()
        // The composer's target changed; bind to that target's own draft buffer so
        // one new-thread target's text is never shown or sent under another.
        if selectedThread == nil {
            let nextKey = newThreadComposerPayloadKey
            if case .some(.draft(let oldDraftID)) = previousKey,
               case .draft(let newDraftID) = nextKey,
               productionRouteStore.replaceVisibleDraftKey(
                   oldDraftID: oldDraftID,
                   newDraftID: newDraftID
               ) {
                if !productionRouteStore.isAttached {
                    applyCanonicalRouteProjection(productionRouteStore.path)
                    activateComposerPayload(for: nextKey)
                }
            } else if !productionRouteStore.isAttached {
                activateComposerPayload(for: nextKey)
            }
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

    func setPendingNewThreadAgentTarget(
        _ id: String?,
        freezesSelection: Bool = false
    ) {
        let targetId = id?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        selectedAgentTargetId = targetId.isEmpty ? nil : targetId
        pendingNewThreadAgentTargetGeneration = selectedThreadDraftGeneration
        frozenNewThreadAgentTargetGeneration = freezesSelection
            ? selectedThreadDraftGeneration
            : nil
    }

    func clearPendingNewThreadAgentTarget() {
        selectedAgentTargetId = nil
        pendingNewThreadAgentTargetGeneration = nil
        frozenNewThreadAgentTargetGeneration = nil
    }

    func currentPendingNewThreadAgentTargetId() -> String {
        guard pendingNewThreadAgentTargetGeneration == selectedThreadDraftGeneration else {
            return ""
        }
        return selectedAgentTargetId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    /// Explicit workspace pick (picker row, workspace-row "New Thread",
    /// agent one-off target). An empty path is not a choice: it re-seeds the
    /// default resolution instead.
    func selectDraftWorkspace(_ path: String) {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            seedDraftWorkspaceDefault()
            return
        }
        setDraftWorkspaceSelection(.path(trimmed))
    }

    /// The explicit "No workspace" choice; never overridden by resolution.
    func selectDraftNoWorkspace() {
        setDraftWorkspaceSelection(.none)
    }

    func setDraftWorkspaceSelection(_ selection: GaryxDraftWorkspaceSelection) {
        newThreadWorkspaceSelection = selection
        if selection.workspacePath == nil {
            newThreadWorkspaceMode = "local"
        }
        saveGatewayScopedUserState()
        if let path = selection.workspacePath, workspaceGitStatuses[path] == nil {
            Task { await refreshWorkspaceGitStatus(for: path) }
        }
    }

    /// Entry points that carry no workspace re-arm the once-only default
    /// resolution against the current catalog.
    func seedDraftWorkspaceDefault() {
        setDraftWorkspaceSelection(
            GaryxDraftWorkspaceSelection.unresolved.resolved(
                against: workspaceCatalog,
                catalogLoaded: workspaceCatalogState.phase.hasResolved
            )
        )
    }

    /// Runs after every catalog replace: fills the once-only default for
    /// unresolved drafts and re-resolves only when the selected workspace
    /// disappeared from the catalog. Resolved selections never drift.
    func resolveDraftWorkspaceSelectionIfNeeded() {
        let resolved = newThreadWorkspaceSelection.resolved(
            against: workspaceCatalog,
            catalogLoaded: workspaceCatalogState.phase.hasResolved
        )
        guard resolved != newThreadWorkspaceSelection else { return }
        setDraftWorkspaceSelection(resolved)
    }

    func refreshWorkspaces() async {
        guard hasGatewaySettings else { return }
        guard workspaceRefreshRequestId == nil else { return }
        let runtimeGeneration = gatewayRequestToken
        let requestId = UUID()
        workspaceRefreshRequestId = requestId
        beginWorkspaceCatalogRefresh()
        do {
            let workspacesPage = try await client().listWorkspaces()
            guard isCurrentWorkspaceRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            workspaceRefreshRequestId = nil
            applyWorkspacesPage(workspacesPage, persist: true)
            ensureSelectedWorkspace()
        } catch {
            guard isCurrentWorkspaceRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            workspaceRefreshRequestId = nil
            let message = displayMessage(for: error)
            failWorkspaceCatalogRefresh(message)
        }
    }

    func isCurrentWorkspaceRefresh(_ requestId: UUID, runtimeGeneration: GaryxGatewayRequestToken) -> Bool {
        runtimeGeneration == gatewayRequestToken && workspaceRefreshRequestId == requestId
    }

    @discardableResult
    func addUserWorkspacePath(_ path: String) async -> String? {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let runtimeGeneration = gatewayRequestToken
        do {
            let workspacesPage = try await client().addWorkspace(
                path: trimmed,
                name: trimmed.garyxLastPathComponent
            )
            guard runtimeGeneration == gatewayRequestToken else { return nil }
            applyWorkspacesPage(workspacesPage, persist: true)
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return nil }
            lastError = error.localizedDescription
            return nil
        }
        if workspaceGitStatuses[trimmed] == nil {
            Task { await refreshWorkspaceGitStatus(for: trimmed) }
        }
        return trimmed
    }

    /// Pin/rename/remove are point mutations; the gateway responds with the
    /// full re-sorted list, which lands verbatim. Responses issued under a
    /// stale gateway token are discarded.
    @discardableResult
    func setWorkspacePinned(path: String, pinned: Bool) async -> Bool {
        await applyWorkspaceMutation { try await $0.pinWorkspace(path: path, pinned: pinned) }
    }

    @discardableResult
    func renameUserWorkspace(path: String, name: String) async -> Bool {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedName.isEmpty else { return false }
        return await applyWorkspaceMutation { try await $0.renameWorkspace(path: path, name: trimmedName) }
    }

    /// Removes the list entry only (server tombstone); never touches files.
    @discardableResult
    func removeUserWorkspace(path: String) async -> Bool {
        await applyWorkspaceMutation { try await $0.removeWorkspace(path: path) }
    }

    private func applyWorkspaceMutation(
        _ mutation: (GaryxGatewayClient) async throws -> GaryxWorkspacesPage
    ) async -> Bool {
        let runtimeGeneration = gatewayRequestToken
        do {
            let page = try await mutation(client())
            guard runtimeGeneration == gatewayRequestToken else { return false }
            applyWorkspacesPage(page, persist: true)
            return true
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func listWorkspaceDirectories(path: String?) async throws -> GaryxWorkspaceDirectoryListing {
        try await client().listWorkspaceDirectories(path: path)
    }

    func setNewThreadWorkspaceMode(_ mode: String) {
        guard selectedThread == nil, !isSending, activeRunThreadId == nil else { return }
        let normalized = Self.normalizedWorkspaceMode(mode)
        guard normalized != "worktree" || newThreadWorkspaceCanUseWorktree else { return }
        newThreadWorkspaceMode = normalized
        if newThreadWorkspaceSelection.workspacePath == nil {
            newThreadWorkspaceMode = "local"
        }
        saveGatewayScopedUserState()
    }

    var newThreadWorkspaceLabel: String {
        switch newThreadWorkspaceSelection {
        case .unresolved:
            return "Workspace"
        case .none:
            return "No workspace"
        case .path(let workspace):
            let name = (workspace as NSString).lastPathComponent
            return name.isEmpty ? workspace : name
        }
    }

    var newThreadWorkspaceCanUseWorktree: Bool {
        guard let workspace = newThreadWorkspaceSelection.workspacePath else { return false }
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
            if !status.canUseWorktree, newThreadWorkspaceSelection.workspacePath == trimmed {
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
    ) async -> GaryxAvatarGenerationOutcome {
        let trimmedId = identifier.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedId.isEmpty || !trimmedName.isEmpty else {
            return .failure(
                GaryxAvatarGenerationFailure(
                    category: .unknown,
                    message: "Agent name is required."
                )
            )
        }
        let prompt = GaryxAvatarPromptBuilder.prompt(
            displayName: trimmedName,
            identifier: trimmedId,
            stylePrompt: stylePrompt
        )
        let runtimeGeneration = gatewayRequestToken
        do {
            try Task.checkCancellation()
            let generated = try await client().generateAvatar(prompt: prompt)
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            let avatarDataUrl = generated.avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !avatarDataUrl.isEmpty else {
                return .failure(GaryxAvatarGenerationFailure(category: .unusable))
            }
            do {
                let normalized = try GaryxMobileAvatarImageNormalizer.normalizedDataUrl(
                    fromRawValue: avatarDataUrl
                )
                return .success(dataUrl: normalized)
            } catch {
                return .failure(GaryxAvatarGenerationFailure(category: .unusable))
            }
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            return GaryxAvatarGenerationOutcome.from(error: error)
        }
    }

    func loadAuthoritativeAgent(agentId: String) async -> GaryxCustomAgentLoadResult {
        let id = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty else {
            return .failed(message: "Agent ID is required.")
        }
        let runtimeGeneration = gatewayRequestToken
        do {
            let agent = try await client().getAgent(agentId: id)
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            return .loaded(agent)
        } catch let error as GaryxGatewayError {
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            if case .httpStatus(404, _, _) = error {
                return .deleted
            }
            return .failed(message: displayMessage(for: error))
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            return .failed(message: displayMessage(for: error))
        }
    }

    func createAgent(_ request: GaryxCustomAgentRequest) async -> GaryxCustomAgentMutationResult {
        let runtimeGeneration = gatewayRequestToken
        do {
            let agent = try await client().createAgent(request)
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            await storeAvatarIfPresent(
                id: agent.id,
                dataUrl: agent.avatarDataUrl.isEmpty ? request.avatarDataUrl ?? "" : agent.avatarDataUrl,
                sourceUpdatedAt: agent.updatedAt
            )
            replaceAgent(agent)
            await refreshAgentTargets()
            return .saved(agent)
        } catch let error as GaryxGatewayError {
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            return .failed(
                GaryxCustomAgentDraftRules.mutationFailure(for: error, mode: .create)
            )
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            return .failed(.other(message: displayMessage(for: error)))
        }
    }

    func updateAgent(
        agentId: String,
        request: GaryxCustomAgentRequest
    ) async -> GaryxCustomAgentMutationResult {
        let immutableAgentId = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !immutableAgentId.isEmpty else {
            return .failed(.other(message: "Agent ID is required."))
        }
        let mode = GaryxCustomAgentDraftMode.edit(
            agentId: immutableAgentId,
            expectedUpdatedAt: request.expectedUpdatedAt ?? ""
        )
        let runtimeGeneration = gatewayRequestToken
        do {
            let updated = try await client().updateAgent(
                agentId: immutableAgentId,
                request: request
            )
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            let requestedAvatar = request.avatarDataUrl?.trimmingCharacters(in: .whitespacesAndNewlines)
            let didClearAvatar = requestedAvatar != nil && requestedAvatar?.isEmpty == true
            let didStoreAvatar: Bool
            if didClearAvatar {
                await removeAvatar(id: updated.id)
                didStoreAvatar = false
            } else {
                didStoreAvatar = await storeAvatarIfPresent(
                    id: updated.id,
                    dataUrl: updated.avatarDataUrl.isEmpty ? requestedAvatar ?? "" : updated.avatarDataUrl,
                    sourceUpdatedAt: updated.updatedAt
                )
            }
            if updated.id != immutableAgentId, didClearAvatar || didStoreAvatar {
                await removeAvatar(id: immutableAgentId)
            }
            replaceAgent(updated, replacing: immutableAgentId)
            return .saved(updated)
        } catch let error as GaryxGatewayError {
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            return .failed(
                GaryxCustomAgentDraftRules.mutationFailure(for: error, mode: mode)
            )
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return .superseded }
            return .failed(.other(message: displayMessage(for: error)))
        }
    }


    func deleteAgent(_ agent: GaryxAgentSummary) async {
        guard !agent.builtIn else { return }
        let runtimeGeneration = gatewayRequestToken
        do {
            _ = try await client().deleteAgent(agentId: agent.id)
            guard runtimeGeneration == gatewayRequestToken else { return }
            await removeAvatar(id: agent.id)
            agents.removeAll { $0.id == agent.id }
            persistCatalogCacheSnapshot()
            await refreshAgentTargets()
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func setAgentEnabled(_ agent: GaryxAgentSummary, enabled: Bool) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            let updated = try await client().setAgentEnabled(agentId: agent.id, enabled: enabled)
            guard runtimeGeneration == gatewayRequestToken else { return }
            replaceAgent(updated)
            await refreshAgentTargets()
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func setDefaultAgent(_ agent: GaryxAgentSummary) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            _ = try await client().setDefaultAgent(agentId: agent.id)
            guard runtimeGeneration == gatewayRequestToken else { return }
            // The response is one agent row; refresh the catalog so both raw and
            // effective defaults move together from the gateway truth source.
            await refreshAgentTargets()
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func loadProviderModels(
        providerType: String,
        runtimeGeneration: GaryxGatewayRequestToken? = nil,
        remoteStateRefreshRequestId: UUID? = nil
    ) async {
        #if DEBUG
        // The compact thread title mounts this loader even in screenshot
        // routes; deterministic fixtures must not contact a live gateway.
        guard !debugSnapshotActive else { return }
        #endif
        let provider = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !provider.isEmpty else { return }
        let observedGeneration = runtimeGeneration ?? gatewayRequestToken
        do {
            let models = try await client().providerModels(providerType: provider)
            guard observedGeneration == gatewayRequestToken,
                  isCurrentRemoteStateScopedRequest(remoteStateRefreshRequestId) else {
                return
            }
            providerModelsByType[provider] = models
        } catch {
            guard observedGeneration == gatewayRequestToken,
                  isCurrentRemoteStateScopedRequest(remoteStateRefreshRequestId) else {
                return
            }
            lastError = displayMessage(for: error)
        }
    }

    /// Fetches the authoritative gateway settings document before opening a
    /// provider editor, so the sheet echoes the authoritative defaults instead
    /// of a possibly cache-restored projection (the mobile-ui "fetch
    /// authoritative data before saving" contract).
    func refreshAuthoritativeGatewaySettings() async -> Bool {
        let runtimeGeneration = gatewayRequestToken
        do {
            let settings = try await client().gatewaySettings()
            guard runtimeGeneration == gatewayRequestToken else { return false }
            gatewaySettingsDocument = settings
            return true
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return false }
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
        let runtimeGeneration = gatewayRequestToken
        do {
            var patch: [String: GaryxJSONValue] = [:]
            GaryxModelProviderDefaults.update(
                settings: &patch,
                provider: provider,
                model: nextModel,
                reasoningEffort: nextReasoningEffort,
                serviceTier: request.serviceTier
            )
            _ = try await client().saveGatewaySettings(patch, merge: true)
            guard runtimeGeneration == gatewayRequestToken else { return false }
            GaryxModelProviderDefaults.update(
                settings: &gatewaySettingsDocument,
                provider: provider,
                model: nextModel,
                reasoningEffort: nextReasoningEffort,
                serviceTier: request.serviceTier
            )
            providerModelsByType.removeValue(forKey: provider.providerType)
            await loadProviderModels(providerType: provider.providerType, runtimeGeneration: runtimeGeneration)
            await refreshRemoteState()
            return true
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return false }
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
        let runtimeGeneration = gatewayRequestToken
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
        let runtimeGeneration = gatewayRequestToken
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
        openWorkspaceFilesPanel(source: source)

        let runtimeGeneration = gatewayRequestToken
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

    /// Installs resolver output only after its workspace-file occurrence is
    /// committed and visible. No navigation write is permitted from here.
    func activatePreparedWorkspaceFilePreview(
        target: GaryxMobileWorkspaceFileTarget,
        preview: GaryxWorkspaceFilePreview,
        listing: GaryxWorkspaceFileListing?
    ) {
        let workspace = target.workspaceDir.trimmingCharacters(in: .whitespacesAndNewlines)
        let filePath = target.path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, !filePath.isEmpty else { return }
        selectedWorkspacePath = workspace
        draftWorkspacePath = workspace
        selectedWorkspaceDirectory = Self.workspaceDirectory(forFilePath: filePath)
        workspacePreview = preview
        workspaceListing = listing
        workspaceUploadStatus = nil
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
        let runtimeGeneration = gatewayRequestToken
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
        runtimeGeneration: GaryxGatewayRequestToken
    ) -> Bool {
        runtimeGeneration == gatewayRequestToken
            && selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines) == workspace
            && selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines) == directory
    }

    func refreshProviderModelsForVisibleAgents(
        runtimeGeneration: GaryxGatewayRequestToken? = nil,
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
        if !residentRecentThreadSummaries.isEmpty {
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
