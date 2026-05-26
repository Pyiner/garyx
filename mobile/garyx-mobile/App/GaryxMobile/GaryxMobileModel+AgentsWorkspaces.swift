import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func openThread(id: String) async {
        pendingThreadLinkId = nil
        let requestId = beginPendingThreadOpen()
        await openThread(id: id, requestId: requestId)
    }

    func queuePendingThreadLink(_ id: String) {
        let threadId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return }
        pendingThreadLinkId = threadId
        let requestId = beginPendingThreadOpen()
        showPendingThreadLink(threadId, requestId: requestId)
    }

    func openPendingThreadLinkIfNeeded() async {
        guard let threadId = pendingThreadLinkId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            pendingThreadLinkId = nil
            return
        }
        guard case .ready = connectionState else { return }
        let requestId = pendingThreadOpenRequestId
        await openThread(id: threadId, requestId: requestId)
        if isCurrentPendingThreadOpen(requestId), selectedThread?.id == threadId {
            pendingThreadLinkId = nil
        }
    }

    private func openThread(id: String, requestId: UUID) async {
        let threadId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return }

        if let thread = threads.first(where: { $0.id == threadId }) {
            guard isCurrentPendingThreadOpen(requestId) else { return }
            await selectThread(thread, invalidatesPendingThreadOpen: false)
            return
        }

        await selectThread(
            Self.placeholderThreadSummary(id: threadId),
            invalidatesPendingThreadOpen: false
        )
        guard isCurrentPendingThreadOpen(requestId) else { return }

        await refreshThreads()
        guard isCurrentPendingThreadOpen(requestId) else { return }
        if let thread = threads.first(where: { $0.id == threadId }) {
            applyOpenedThreadSummary(thread)
            return
        }
        do {
            let thread = try await client().getThread(threadId: threadId)
            guard isCurrentPendingThreadOpen(requestId) else { return }
            threads = Self.mergedThreadSummaries(threads + [thread])
            applyOpenedThreadSummary(thread)
        } catch {
            guard isCurrentPendingThreadOpen(requestId) else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func showPendingThreadLink(_ threadId: String, requestId: UUID) {
        guard isCurrentPendingThreadOpen(requestId) else { return }
        let thread = threads.first(where: { $0.id == threadId })
            ?? (selectedThread?.id == threadId ? selectedThread : nil)
            ?? Self.placeholderThreadSummary(id: threadId)
        let previousThreadId = selectedThread?.id
        if previousThreadId != threadId {
            advanceSelectedThreadDraftGeneration()
            resetComposerDraft()
            selectedThreadRecoveryTask?.cancel()
            selectedThreadRecoveryTask = nil
            selectedThreadRecoveryThreadId = nil
            cancelSelectedThreadReconcileLoop()
            resetSelectedThreadHistoryPagination()
            messages = cachedMessages(for: threadId)
        }
        selectedThread = thread
        clearPendingBotDraft()
        draftThreadTitle = thread.title
        setActivePanel(.chat, invalidatesPendingThreadOpen: false)
        setSidebarVisible(false)
        lastError = nil
    }

    func beginPendingThreadOpen() -> UUID {
        let requestId = UUID()
        pendingThreadOpenRequestId = requestId
        return requestId
    }

    func invalidatePendingThreadOpen() {
        pendingThreadOpenRequestId = UUID()
        pendingThreadLinkId = nil
    }

    func isCurrentPendingThreadOpen(_ requestId: UUID) -> Bool {
        pendingThreadOpenRequestId == requestId
    }

    func applyOpenedThreadSummary(_ thread: GaryxThreadSummary) {
        threads = Self.mergedThreadSummaries(threads + [thread])
        guard selectedThread?.id == thread.id else { return }
        selectedThread = thread
        draftThreadTitle = thread.title
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
            teamId: nil,
            teamName: nil,
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

    func addUserWorkspacePath(_ path: String) async {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            let workspaces = try await client().addWorkspace(path: trimmed, name: trimmed.garyxLastPathComponent)
            userWorkspacePaths = GaryxMobileWorkspacePresentation.userWorkspacePaths(
                savedWorkspacePaths: workspaces.map(\.path)
            )
        } catch {
            lastError = error.localizedDescription
            return
        }
        if workspaceGitStatuses[trimmed] == nil {
            Task { await refreshWorkspaceGitStatus(for: trimmed) }
        }
    }

    func setNewThreadWorkspaceMode(_ mode: String) {
        let normalized = Self.normalizedWorkspaceMode(mode)
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
        return workspaceGitStatuses[workspace]?.canUseWorktree ?? true
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

    func createAgentFromDraft() async -> Bool {
        let agentId = draftAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = draftAgentName.trimmingCharacters(in: .whitespacesAndNewlines)
        let provider = draftAgentProvider.trimmingCharacters(in: .whitespacesAndNewlines)
        let model = draftAgentModel.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = draftAgentWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = draftAgentPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !agentId.isEmpty, !displayName.isEmpty, !provider.isEmpty else { return false }
        do {
            let agent = try await client().createAgent(
                GaryxCustomAgentRequest(
                    agentId: agentId,
                    displayName: displayName,
                    providerType: provider,
                    model: model.isEmpty ? nil : model,
                    defaultWorkspaceDir: workspace.isEmpty ? nil : workspace,
                    systemPrompt: prompt
                )
            )
            draftAgentId = ""
            draftAgentName = ""
            draftAgentModel = ""
            draftAgentWorkspace = ""
            draftAgentPrompt = ""
            replaceAgent(agent)
            setSelectedAgentTarget(agent.id)
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func updateAgent(
        _ agent: GaryxAgentSummary,
        agentId: String,
        displayName: String,
        providerType: String,
        modelName: String,
        workspace: String,
        systemPrompt: String
    ) async {
        let nextAgentId = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDisplayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextProviderType = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextModelName = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextAgentId.isEmpty, !nextDisplayName.isEmpty, !nextProviderType.isEmpty else { return }
        do {
            let updated = try await client().updateAgent(
                agentId: agent.id,
                request: GaryxCustomAgentRequest(
                    agentId: nextAgentId,
                    displayName: nextDisplayName,
                    providerType: nextProviderType,
                    model: nextModelName.isEmpty ? nil : nextModelName,
                    modelReasoningEffort: agent.modelReasoningEffort.isEmpty ? nil : agent.modelReasoningEffort,
                    modelServiceTier: agent.modelServiceTier.isEmpty ? nil : agent.modelServiceTier,
                    providerEnv: agent.providerEnv.isEmpty ? nil : agent.providerEnv,
                    authSource: agent.authSource.isEmpty ? nil : agent.authSource,
                    baseUrl: agent.baseUrl.isEmpty ? nil : agent.baseUrl,
                    codexHome: agent.codexHome.isEmpty ? nil : agent.codexHome,
                    maxToolIterations: agent.maxToolIterations,
                    requestTimeoutSeconds: agent.requestTimeoutSeconds,
                    defaultWorkspaceDir: nextWorkspace.isEmpty ? nil : nextWorkspace,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    systemPrompt: systemPrompt
                )
            )
            replaceAgent(updated)
            setSelectedAgentTarget(updated.id)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateTeam(
        _ team: GaryxTeamSummary,
        teamId: String,
        displayName: String,
        leaderAgentId: String,
        memberAgentIds: String,
        workflowText: String
    ) async {
        let nextTeamId = teamId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDisplayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextLeader = leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextMembers = Self.normalizedTeamMemberIds(memberAgentIds, leaderAgentId: nextLeader)
        let nextWorkflow = workflowText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextTeamId.isEmpty, !nextDisplayName.isEmpty, !nextLeader.isEmpty else { return }
        do {
            let updated = try await client().updateTeam(
                teamId: team.id,
                request: GaryxTeamRequest(
                    teamId: nextTeamId,
                    displayName: nextDisplayName,
                    leaderAgentId: nextLeader,
                    memberAgentIds: nextMembers,
                    workflowText: nextWorkflow,
                    avatarDataUrl: team.avatarDataUrl.isEmpty ? nil : team.avatarDataUrl
                )
            )
            replaceTeam(updated)
            setSelectedAgentTarget(updated.id)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteAgent(_ agent: GaryxAgentSummary) async {
        guard !agent.builtIn else { return }
        do {
            _ = try await client().deleteAgent(agentId: agent.id)
            agents.removeAll { $0.id == agent.id }
            ensureSelectedAgentTarget()
        } catch {
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

    func createTeamFromDraft() async -> Bool {
        let teamId = draftTeamId.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = draftTeamName.trimmingCharacters(in: .whitespacesAndNewlines)
        let leader = draftTeamLeaderId.trimmingCharacters(in: .whitespacesAndNewlines)
        let members = Self.normalizedTeamMemberIds(draftTeamMemberIds, leaderAgentId: leader)
        let workflow = draftTeamWorkflow.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !teamId.isEmpty, !name.isEmpty, !leader.isEmpty else { return false }
        do {
            let team = try await client().createTeam(
                GaryxTeamRequest(
                    teamId: teamId,
                    displayName: name,
                    leaderAgentId: leader,
                    memberAgentIds: members,
                    workflowText: workflow
                )
            )
            draftTeamId = ""
            draftTeamName = ""
            draftTeamLeaderId = ""
            draftTeamMemberIds = ""
            draftTeamWorkflow = ""
            replaceTeam(team)
            setSelectedAgentTarget(team.id)
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteTeam(_ team: GaryxTeamSummary) async {
        do {
            _ = try await client().deleteTeam(teamId: team.id)
            teams.removeAll { $0.id == team.id }
            ensureSelectedAgentTarget()
        } catch {
            lastError = displayMessage(for: error)
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

    func localFilePreview(_ target: String) async -> GaryxWorkspaceFilePreview? {
        guard let resolved = GaryxMobileFileLink.previewTarget(
            fromLink: target,
            workspacePaths: workspacePathSuggestions
        ) else {
            lastError = "Garyx could not resolve this local file for preview."
            return nil
        }
        return await workspaceFilePreview(resolved)
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
        from preview: GaryxWorkspaceFilePreview
    ) async -> GaryxWorkspaceFilePreview? {
        let workspacePaths = workspacePathSuggestions + [preview.workspaceDir]
        guard let resolved = GaryxMobileFileLink.previewTarget(
            fromLink: target,
            workspacePaths: workspacePaths,
            currentWorkspaceDir: preview.workspaceDir,
            currentFilePath: preview.path
        ) else {
            lastError = "Garyx could not resolve this local file for preview."
            return nil
        }
        return await workspaceFilePreview(resolved)
    }

    func openWorkspaceFilePreview(_ target: GaryxMobileWorkspaceFileTarget) async {
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
        openPanel(.workspaces)

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

    private func workspaceFilePreview(_ target: GaryxMobileWorkspaceFileTarget) async -> GaryxWorkspaceFilePreview? {
        let workspace = target.workspaceDir.trimmingCharacters(in: .whitespacesAndNewlines)
        let filePath = target.path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, !filePath.isEmpty else { return nil }

        do {
            return try await client().previewWorkspaceFile(
                workspaceDir: workspace,
                path: filePath
            )
        } catch {
            lastError = displayMessage(for: error)
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

    func replaceAgent(_ agent: GaryxAgentSummary) {
        if let index = agents.firstIndex(where: { $0.id == agent.id }) {
            agents[index] = agent
        } else {
            agents.insert(agent, at: 0)
        }
        if !threads.isEmpty {
            persistRecentThreadsWidgetSnapshot()
        }
    }

    func replaceTeam(_ team: GaryxTeamSummary) {
        if let index = teams.firstIndex(where: { $0.id == team.id }) {
            teams[index] = team
        } else {
            teams.insert(team, at: 0)
        }
        if !threads.isEmpty {
            persistRecentThreadsWidgetSnapshot()
        }
    }

    static func normalizedTeamMemberIds(_ rawValue: String, leaderAgentId: String) -> [String] {
        let leader = leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        var ids: [String] = leader.isEmpty ? [] : [leader]
        for token in rawValue.split(whereSeparator: { $0 == "," || $0 == "\n" || $0 == " " }) {
            let id = String(token).trimmingCharacters(in: .whitespacesAndNewlines)
            if !id.isEmpty, !ids.contains(id) {
                ids.append(id)
            }
        }
        return ids
    }
}
