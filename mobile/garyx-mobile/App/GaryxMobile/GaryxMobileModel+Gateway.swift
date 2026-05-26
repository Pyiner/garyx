import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func saveGatewaySettings() {
        gatewaySettingsStatus = nil
        gatewayURL = normalizedGatewayURL(gatewayURL)
        gatewayAuthToken = gatewayAuthToken.trimmingCharacters(in: .whitespacesAndNewlines)
        defaults.set(gatewayURL, forKey: GaryxMobileSettingsKeys.gatewayUrl)
        defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayURL)
        defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayToken)
        saveGatewayScopedUserState()
        keychain.saveGatewayAuthToken(gatewayAuthToken)
    }

    func saveGatewaySettingsFromUI() {
        saveGatewaySettings()
        rememberCurrentGatewayProfile()
        gatewaySettingsStatus = "Saved"
    }

    var currentGatewayScopeId: String {
        let normalized = normalizedGatewayURL(gatewayURL)
        guard !normalized.isEmpty else { return "unconfigured" }
        return GaryxGatewayProfileStorage.stableId(for: normalized)
    }

    func scopedSettingsKey(_ key: String) -> String {
        "\(key).\(currentGatewayScopeId)"
    }

    func loadGatewayScopedUserState(fallbackToLegacy: Bool) {
        let agentKey = scopedSettingsKey(GaryxMobileSettingsKeys.selectedAgentTargetId)
        let workspaceKey = scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspace)
        let workspaceModeKey = scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspaceMode)
        selectedAgentTargetId = defaults.string(forKey: agentKey)
            ?? (fallbackToLegacy ? defaults.string(forKey: GaryxMobileSettingsKeys.selectedAgentTargetId) : nil)
            ?? "claude"
        newThreadWorkspace = defaults.string(forKey: workspaceKey)
            ?? (fallbackToLegacy ? defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspace) : nil)
            ?? ""
        newThreadWorkspaceMode = Self.normalizedWorkspaceMode(
            defaults.string(forKey: workspaceModeKey)
                ?? (fallbackToLegacy ? defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspaceMode) : nil)
        )
        userWorkspacePaths = []
    }

    func saveGatewayScopedUserState() {
        defaults.set(selectedAgentTargetId, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.selectedAgentTargetId))
        defaults.set(
            newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines),
            forKey: scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspace)
        )
        defaults.set(
            Self.normalizedWorkspaceMode(newThreadWorkspaceMode),
            forKey: scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspaceMode)
        )
        defaults.removeObject(forKey: scopedSettingsKey(GaryxMobileSettingsKeys.userWorkspacePaths))
        defaults.removeObject(forKey: GaryxMobileSettingsKeys.userWorkspacePaths)
    }

    func resetGatewayRuntimeState() {
        gatewayRuntimeGeneration = UUID()
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
        selectedThreadHistoryRequestId = nil
        pendingThreadLinkId = nil
        completedThreadHistoryHydrationTasks.values.forEach { $0.cancel() }
        completedThreadHistoryHydrationTasks = [:]
        resetSelectedThreadHistoryPagination()
        resetThreadListPagination()
        sceneRefreshTask?.cancel()
        sceneRefreshTask = nil
        cancelSelectedThreadReconcileLoop()
        selectedThreadActivitySignatures = [:]
        cancelGlobalEventStream()
        cancelActiveSocket()
        isSending = false
        remoteStateRefreshRequestId = nil
        agentTargetsRefreshRequestId = nil
        remoteBusyThreadIds = []
        agentTargetsLoadPhase = .idle
        connectionState = .disconnected
        threads = []
        pinnedThreadIds = []
        recentThreadIds = []
        GaryxMobileWidgetStore.clear()
        WidgetCenter.shared.reloadTimelines(ofKind: GaryxRecentThreadsWidgetConstants.kind)
        selectedThread = nil
        messages = []
        messagesByThread = [:]
        messageSignaturesByThread = [:]
        activeAssistantMessageIdsByThread = [:]
        pendingAssistantDeltasByThread = [:]
        assistantDeltaFlushTasksByThread.values.forEach { $0.cancel() }
        assistantDeltaFlushTasksByThread = [:]
        resetComposerDraft()
        draftThreadTitle = ""
        agents = []
        teams = []
        skills = []
        tasks = []
        automations = []
        slashCommands = []
        mcpServers = []
        autoResearchRuns = []
        channelEndpoints = []
        configuredBots = []
        botConsoles = []
        userWorkspacePaths = []
        botStatusesById = [:]
        channelPlugins = []
        gatewaySettingsDocument = [:]
        isSavingBotSettings = false
        providerModelsByType = [:]
        selectedWorkspacePath = ""
        selectedWorkspaceDirectory = ""
        draftWorkspacePath = ""
        clearPendingBotDraft()
        workspaceListing = nil
        workspacePreview = nil
        workspaceGitStatuses = [:]
        isUploadingWorkspaceFiles = false
        workspaceUploadStatus = nil
        selectedSkillEditor = nil
        selectedSkillDocument = nil
        selectedSkillFileContent = ""
        researchCandidatesByRunId = [:]
        autoResearchDetailsByRunId = [:]
        autoResearchIterationsByRunId = [:]
        isLoadingThreads = false
        remoteStateLoadPhase = .idle
        isLoadingSelectedThreadHistory = false
    }

    func selectGatewayProfile(_ profile: GaryxGatewayProfile) {
        saveGatewayScopedUserState()
        resetGatewayRuntimeState()
        gatewayURL = profile.gatewayUrl
        gatewayAuthToken = keychain.readGatewayProfileToken(profileId: profile.id)
        loadGatewayScopedUserState(fallbackToLegacy: false)
        gatewaySettingsStatus = "Selected \(profile.label)"
        lastError = nil
    }

    func activateGatewayProfile(_ profile: GaryxGatewayProfile) async {
        selectGatewayProfile(profile)
        await connectAndRefresh()
    }

    func gatewayProfileToken(_ profile: GaryxGatewayProfile) -> String {
        keychain.readGatewayProfileToken(profileId: profile.id)
    }

    @discardableResult
    func updateGatewayProfile(
        _ profile: GaryxGatewayProfile,
        label: String,
        gatewayUrl: String,
        token: String
    ) -> Bool {
        let normalizedURL = normalizedGatewayURL(gatewayUrl)
        guard parsedGatewayURL(from: normalizedURL) != nil else {
            lastError = "Invalid gateway URL"
            return false
        }
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextId = GaryxGatewayProfileStorage.stableId(for: normalizedURL)
        let currentURL = normalizedGatewayURL(gatewayURL)
        let currentProfileId = currentGatewayProfile?.id
        let affectsCurrentProfile = currentProfileId == profile.id
            || currentProfileId == nextId
            || currentURL.lowercased() == normalizedURL.lowercased()
        let currentURLChanged = currentURL.lowercased() != normalizedURL.lowercased()
        let activeTokenChanged = gatewayAuthToken != trimmedToken
        var nextProfile = profile
        nextProfile.id = nextId
        nextProfile.label = trimmedLabel.isEmpty ? GaryxGatewayProfileStorage.label(for: normalizedURL) : trimmedLabel
        nextProfile.gatewayUrl = normalizedURL
        nextProfile.updatedAt = Date()
        nextProfile.hasToken = !trimmedToken.isEmpty

        gatewayProfiles.removeAll { candidate in
            candidate.id == profile.id
                || candidate.gatewayUrl.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                    == normalizedURL.lowercased()
        }
        gatewayProfiles = GaryxGatewayProfileStorage.normalizedProfiles([nextProfile] + gatewayProfiles)
        persistGatewayProfiles()
        if profile.id != nextId {
            keychain.deleteGatewayProfileToken(profileId: profile.id)
        }
        keychain.saveGatewayProfileToken(trimmedToken, profileId: nextId)

        if affectsCurrentProfile {
            saveGatewayScopedUserState()
            if currentURLChanged || activeTokenChanged {
                resetGatewayRuntimeState()
            }
            gatewayURL = normalizedURL
            gatewayAuthToken = trimmedToken
            defaults.set(gatewayURL, forKey: GaryxMobileSettingsKeys.gatewayUrl)
            defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayURL)
            defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayToken)
            keychain.saveGatewayAuthToken(gatewayAuthToken)
            if currentURLChanged {
                loadGatewayScopedUserState(fallbackToLegacy: false)
            }
        }
        gatewaySettingsStatus = "Updated \(nextProfile.label)"
        lastError = nil
        return true
    }

    func removeGatewayProfile(_ profile: GaryxGatewayProfile) {
        gatewayProfiles.removeAll { $0.id == profile.id }
        persistGatewayProfiles()
        keychain.deleteGatewayProfileToken(profileId: profile.id)
        if currentGatewayProfile?.id == profile.id {
            gatewaySettingsStatus = nil
        }
    }

    func clearPendingBotDraft() {
        pendingBotId = nil
        pendingBotWorkspace = nil
        pendingBotAgentId = nil
        pendingBotDraftGeneration = nil
    }

    func handleScenePhase(_ phase: ScenePhase) {
        switch phase {
        case .active:
            sceneRefreshTask?.cancel()
            let selectedThreadId = selectedThread?.id
            sceneRefreshTask = Task { [weak self] in
                guard let self else { return }
                switch connectionState {
                case .ready:
                    startGlobalEventStream()
                    startSelectedThreadReconcileLoop()
                    async let agentTargetsRefresh: Void = refreshAgentTargets()
                    await refreshThreads()
                    await agentTargetsRefresh
                    guard !Task.isCancelled else { return }
                    if let selectedThreadId, selectedThread?.id == selectedThreadId {
                        await loadSelectedThreadHistory()
                    }
                case .checking:
                    break
                case .disconnected, .failed:
                    break
                }
            }
        case .background:
            sceneRefreshTask?.cancel()
            sceneRefreshTask = nil
            cancelSelectedThreadReconcileLoop()
            cancelGlobalEventStream()
            let runningThreadIds = Array(activeTasksByThread.keys)
            if !runningThreadIds.isEmpty {
                for threadId in runningThreadIds {
                    let activeAssistantMessageId = suspendStreamingAssistantForBackground(threadId: threadId)
                    remoteBusyThreadIds.insert(threadId)
                    cancelActiveSocket(for: threadId)
                    if let activeAssistantMessageId,
                       cachedMessages(for: threadId).contains(where: { $0.id == activeAssistantMessageId }) {
                        activeAssistantMessageIdsByThread[threadId] = activeAssistantMessageId
                    }
                }
                isSending = false
            }
        default:
            break
        }
    }

    func rememberCurrentGatewayProfile() {
        let url = normalizedGatewayURL(gatewayURL)
        guard !url.isEmpty else { return }
        let profile = GaryxGatewayProfile(
            id: GaryxGatewayProfileStorage.stableId(for: url),
            label: GaryxGatewayProfileStorage.label(for: url),
            gatewayUrl: url,
            updatedAt: Date(),
            hasToken: !gatewayAuthToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        )
        gatewayProfiles = GaryxGatewayProfileStorage.normalizedProfiles([profile] + gatewayProfiles)
        persistGatewayProfiles()
        keychain.saveGatewayProfileToken(gatewayAuthToken, profileId: profile.id)
    }

    func persistGatewayProfiles() {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        if let data = try? encoder.encode(gatewayProfiles) {
            defaults.set(data, forKey: GaryxMobileSettingsKeys.gatewayProfiles)
        }
    }

    func applyMobileConnectLink(_ url: URL) async {
        guard let payload = GaryxMobileConnectLink.parse(url) else {
            return
        }
        saveGatewayScopedUserState()
        resetGatewayRuntimeState()
        gatewayURL = payload.gatewayUrl
        gatewayAuthToken = payload.gatewayAuthToken
        loadGatewayScopedUserState(fallbackToLegacy: false)
        await connectAndRefresh()
    }

    func handleOpenURL(_ url: URL) async {
        if let threadId = GaryxMobileThreadLink.parse(url) {
            queuePendingThreadLink(threadId)
            if case .ready = connectionState {
                await openPendingThreadLinkIfNeeded()
            } else if canConnectGateway, case .checking = connectionState {
                return
            } else if canConnectGateway {
                await connectAndRefresh()
            }
            return
        }
        await applyMobileConnectLink(url)
    }

    func connectAndRefresh() async {
        gatewayURL = normalizedGatewayURL(gatewayURL)
        gatewayAuthToken = gatewayAuthToken.trimmingCharacters(in: .whitespacesAndNewlines)
        connectionState = .checking
        lastError = nil
        gatewaySettingsStatus = nil
        do {
            let status = try await client().status()
            _ = try await client().chatHealth()
            saveGatewaySettings()
            rememberCurrentGatewayProfile()
            gatewaySettingsStatus = "Saved and connected"
            connectionState = .ready(version: status.version)
            startGlobalEventStream()
            async let agentTargetsRefresh: Void = refreshAgentTargets()
            await refreshThreads()
            await agentTargetsRefresh
            await refreshRemoteState()
            startSelectedThreadReconcileLoop()
            await openPendingThreadLinkIfNeeded()
        } catch {
            cancelGlobalEventStream()
            cancelSelectedThreadReconcileLoop()
            let message = displayMessage(for: error)
            connectionState = .failed(message)
            lastError = message
        }
    }

    func refreshAgentTargetsIfNeeded() async {
        guard agentTargets.isEmpty, !agentTargetsLoadPhase.isLoading else { return }
        await refreshAgentTargets()
    }

    func refreshAgentTargets() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let requestId = UUID()
        agentTargetsRefreshRequestId = requestId
        agentTargetsLoadPhase = .loading
        do {
            let gateway = try client()
            async let agentsResult: [GaryxAgentSummary]? = try? gateway.listAgents()
            async let teamsResult: [GaryxTeamSummary]? = try? gateway.listTeams()
            let (nextAgents, nextTeams) = await (agentsResult, teamsResult)
            guard isCurrentAgentTargetsRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            agentTargetsRefreshRequestId = nil
            if !applyAgentTargets(agents: nextAgents, teams: nextTeams) {
                agentTargetsLoadPhase = .failed("Agents could not be loaded.")
            }
        } catch {
            guard isCurrentAgentTargetsRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            agentTargetsRefreshRequestId = nil
            let message = displayMessage(for: error)
            agentTargetsLoadPhase = .failed(message)
            lastError = message
        }
    }

    func refreshRemoteState() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let requestId = UUID()
        remoteStateRefreshRequestId = requestId
        let ownsAgentTargetsLoadPhase = agentTargets.isEmpty && agentTargetsRefreshRequestId == nil
        remoteStateLoadPhase = .loading
        if ownsAgentTargetsLoadPhase {
            agentTargetsLoadPhase = .loading
        }
        do {
            let gateway = try client()
            async let agentsResult = gateway.listAgents()
            async let teamsResult = gateway.listTeams()
            async let skillsResult = gateway.listSkills()
            async let tasksResult = gateway.listTasks(includeDone: true, limit: 120)
            async let dreamsResult = gateway.listDreams(sinceHours: 24, limit: 80)
            async let gatewaySettingsResult = gateway.gatewaySettings()
            async let automationsResult = gateway.listAutomations()
            async let slashCommandsResult = gateway.listSlashCommands()
            async let mcpServersResult = gateway.listMcpServers()
            async let autoResearchRunsResult = gateway.listAutoResearchRuns()
            async let channelEndpointsResult = gateway.listChannelEndpoints()
            async let workspacesResult = gateway.listWorkspaces()
            async let configuredBotsResult = gateway.listConfiguredBots()
            async let botConsolesResult = gateway.listBotConsoles()
            async let channelPluginsResult = gateway.listChannelPlugins()

            let nextAgents = try? await agentsResult
            let nextTeams = try? await teamsResult
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            applyAgentTargets(agents: nextAgents, teams: nextTeams)

            let nextSkills = try? await skillsResult
            let nextTasksPage = try? await tasksResult
            let nextDreamsPage = try? await dreamsResult
            let nextGatewaySettings = try? await gatewaySettingsResult
            let nextAutomations = try? await automationsResult
            let nextSlashCommands = try? await slashCommandsResult
            let nextMcpServers = try? await mcpServersResult
            let nextAutoResearchRuns = try? await autoResearchRunsResult
            let nextChannelEndpoints = try? await channelEndpointsResult
            let nextWorkspaces = try? await workspacesResult
            let nextConfiguredBots = try? await configuredBotsResult
            let nextBotConsoles = try? await botConsolesResult
            let nextChannelPlugins = try? await channelPluginsResult
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }

            skills = nextSkills ?? skills
            if let page = nextTasksPage {
                tasks = page.tasks
            }
            if let page = nextDreamsPage {
                dreams = page.dreams
                latestDreamScan = page.scan ?? page.latestScan
            }
            if let settings = nextGatewaySettings {
                gatewaySettingsDocument = settings
                applyGatewayRuntimeSettings(settings)
            }
            automations = nextAutomations ?? automations
            slashCommands = nextSlashCommands ?? slashCommands
            mcpServers = nextMcpServers ?? mcpServers
            autoResearchRuns = nextAutoResearchRuns ?? autoResearchRuns
            channelEndpoints = nextChannelEndpoints ?? channelEndpoints
            if let nextWorkspaces {
                userWorkspacePaths = GaryxMobileWorkspacePresentation.userWorkspacePaths(
                    savedWorkspacePaths: nextWorkspaces.map(\.path)
                )
            }
            configuredBots = nextConfiguredBots ?? configuredBots
            botConsoles = nextBotConsoles ?? botConsoles
            channelPlugins = nextChannelPlugins ?? channelPlugins
            await mergeMissingSidebarRequiredThreads(
                using: gateway,
                extraThreadIds: [selectedThread?.id],
                runtimeGeneration: runtimeGeneration,
                remoteStateRefreshRequestId: requestId
            )
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            ensureSelectedWorkspace()
            await refreshProviderModelsForVisibleAgents(
                runtimeGeneration: runtimeGeneration,
                remoteStateRefreshRequestId: requestId
            )
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            if ownsAgentTargetsLoadPhase,
               agentTargetsRefreshRequestId == nil,
               agentTargetsLoadPhase.isLoading {
                agentTargetsLoadPhase = agentTargets.isEmpty
                    ? .failed("Agents could not be loaded.")
                    : .loaded
            }
            remoteStateLoadPhase = .loaded
        } catch {
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            let message = displayMessage(for: error)
            remoteStateLoadPhase = .failed(message)
            if ownsAgentTargetsLoadPhase,
               agentTargetsRefreshRequestId == nil,
               agentTargetsLoadPhase.isLoading {
                agentTargetsLoadPhase = .failed(message)
            }
            lastError = message
        }
    }

    func isCurrentRemoteStateRefresh(_ requestId: UUID, runtimeGeneration: UUID) -> Bool {
        runtimeGeneration == gatewayRuntimeGeneration && remoteStateRefreshRequestId == requestId
    }

    func isCurrentAgentTargetsRefresh(_ requestId: UUID, runtimeGeneration: UUID) -> Bool {
        runtimeGeneration == gatewayRuntimeGeneration && agentTargetsRefreshRequestId == requestId
    }

    func applyGatewayRuntimeSettings(_ settings: [String: GaryxJSONValue]) {
        dreamsAutoScanEnabled = settings
            .objectValue(forKeys: ["dreams"])?
            .boolValue(forKeys: ["enabled"]) ?? false
        if !dreamsAutoScanEnabled {
            dreams = []
            latestDreamScan = nil
            if activePanel == .dreams {
                activePanel = .chat
            }
        }
    }
}
