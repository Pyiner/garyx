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
        activeGatewayScopeId = currentGatewayScopeId
        catalogSnapshotRestored = false
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
        restoreCachedCatalogState()
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
        connectRefreshRequestId = nil
        remoteStateRefreshRequestId = nil
        agentTargetsRefreshRequestId = nil
        agentTargetsStateRequestId = nil
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
        catalogSnapshotRestored = false
        gatewaySettingsDocument = [:]
        isSavingBotSettings = false
        providerModelsByType = [:]
        selectedWorkspacePath = ""
        selectedWorkspaceDirectory = ""
        draftWorkspacePath = ""
        clearPendingBotDraft()
        clearPendingNewThreadAgentTarget()
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
            let didResetRuntime = currentURLChanged || activeTokenChanged
            if didResetRuntime {
                resetGatewayRuntimeState()
            }
            gatewayURL = normalizedURL
            gatewayAuthToken = trimmedToken
            defaults.set(gatewayURL, forKey: GaryxMobileSettingsKeys.gatewayUrl)
            defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayURL)
            defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayToken)
            keychain.saveGatewayAuthToken(gatewayAuthToken)
            if didResetRuntime {
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
        if activeGatewayScopeId != currentGatewayScopeId {
            resetGatewayRuntimeState()
            loadGatewayScopedUserState(fallbackToLegacy: false)
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        let gatewayScopeId = currentGatewayScopeId
        let requestId = UUID()
        connectRefreshRequestId = requestId
        connectionState = .checking
        lastError = nil
        gatewaySettingsStatus = nil
        do {
            let gateway = try client()
            let status = try await gateway.status()
            _ = try await gateway.chatHealth()
            guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                return
            }
            saveGatewaySettings()
            rememberCurrentGatewayProfile()
            gatewaySettingsStatus = "Saved and connected"
            connectionState = .ready(version: status.version)
            startGlobalEventStream()
            async let agentTargetsRefresh: Void = refreshAgentTargets()
            await refreshThreads()
            await agentTargetsRefresh
            await refreshRemoteState()
            guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                return
            }
            connectRefreshRequestId = nil
            await openPendingThreadLinkIfNeeded()
            startSelectedThreadReconcileLoop()
        } catch {
            guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                return
            }
            connectRefreshRequestId = nil
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
        agentTargetsStateRequestId = requestId
        let hadAgentTargetsBeforeRefresh = !agentTargets.isEmpty
        let ownsAgentTargetsLoadPhase = agentTargets.isEmpty
        if ownsAgentTargetsLoadPhase {
            agentTargetsLoadPhase = .loading
        }
        do {
            let gateway = try client()
            async let agentsResult = garyxCaptureCatalog { try await gateway.listAgents() }
            async let teamsResult = garyxCaptureCatalog { try await gateway.listTeams() }
            let (agentsOutcome, teamsOutcome) = await (agentsResult, teamsResult)
            guard isCurrentAgentTargetsRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            agentTargetsRefreshRequestId = nil
            if agentTargetsStateRequestId == requestId {
                agentTargetsStateRequestId = nil
            }
            let nextAgents = agentsOutcome.successValue
            let nextTeams = teamsOutcome.successValue
            if nextAgents != nil || nextTeams != nil {
                applyAgentTargets(agents: nextAgents, teams: nextTeams)
            }
            if agentsOutcome.isFailure || teamsOutcome.isFailure {
                let message = catalogRefreshFailureMessage(
                    from: [AnyCatalogResult(agentsOutcome), AnyCatalogResult(teamsOutcome)]
                )
                    ?? "Agents could not be loaded."
                agentTargetsLoadPhase = hadAgentTargetsBeforeRefresh ? .loaded : .failed(message)
                lastError = message
            } else {
                agentTargetsLoadPhase = .loaded
            }
        } catch {
            guard isCurrentAgentTargetsRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            agentTargetsRefreshRequestId = nil
            if agentTargetsStateRequestId == requestId {
                agentTargetsStateRequestId = nil
            }
            let message = displayMessage(for: error)
            agentTargetsLoadPhase = hadAgentTargetsBeforeRefresh ? .loaded : .failed(message)
            lastError = message
        }
    }

    func refreshRemoteState() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let requestId = UUID()
        remoteStateRefreshRequestId = requestId
        let supersededAgentTargetsRefresh = agentTargetsRefreshRequestId != nil
        agentTargetsRefreshRequestId = nil
        agentTargetsStateRequestId = requestId
        let hadAgentTargetsBeforeRefresh = !agentTargets.isEmpty
        let ownsAgentTargetsLoadPhase = agentTargets.isEmpty
            || supersededAgentTargetsRefresh
            || agentTargetsLoadPhase.isLoading
        remoteStateLoadPhase = .loading
        if ownsAgentTargetsLoadPhase {
            agentTargetsLoadPhase = .loading
        }
        do {
            let gateway = try client()
            async let agentsResult = garyxCaptureCatalog { try await gateway.listAgents() }
            async let teamsResult = garyxCaptureCatalog { try await gateway.listTeams() }
            async let skillsResult = garyxCaptureCatalog { try await gateway.listSkills() }
            async let tasksResult = garyxCaptureCatalog { try await gateway.listTasks(includeDone: true, limit: 120) }
            async let dreamsResult: GaryxDreamsPage? = try? gateway.listDreams(sinceHours: 24, limit: 80)
            async let gatewaySettingsResult: [String: GaryxJSONValue]? = try? gateway.gatewaySettings()
            async let automationsResult = garyxCaptureCatalog { try await gateway.listAutomations() }
            async let slashCommandsResult = garyxCaptureCatalog { try await gateway.listSlashCommands() }
            async let mcpServersResult = garyxCaptureCatalog { try await gateway.listMcpServers() }
            async let autoResearchRunsResult: [GaryxAutoResearchRun]? = try? gateway.listAutoResearchRuns()
            async let channelEndpointsResult = garyxCaptureCatalog { try await gateway.listChannelEndpoints() }
            async let workspacesResult = garyxCaptureCatalog { try await gateway.listWorkspaces() }
            async let configuredBotsResult = garyxCaptureCatalog { try await gateway.listConfiguredBots() }
            async let botConsolesResult = garyxCaptureCatalog { try await gateway.listBotConsoles() }
            async let channelPluginsResult = garyxCaptureCatalog { try await gateway.listChannelPlugins() }

            let nextAgents = await agentsResult
            let nextTeams = await teamsResult
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            let ownsAgentTargetsState = agentTargetsStateRequestId == requestId
            if ownsAgentTargetsState, (nextAgents.successValue != nil || nextTeams.successValue != nil) {
                applyAgentTargets(agents: nextAgents.successValue, teams: nextTeams.successValue)
            }

            let nextSkills = await skillsResult
            let nextTasksPage = await tasksResult
            let nextDreamsPage = await dreamsResult
            let nextGatewaySettings = await gatewaySettingsResult
            let nextAutomations = await automationsResult
            let nextSlashCommands = await slashCommandsResult
            let nextMcpServers = await mcpServersResult
            let nextAutoResearchRuns = await autoResearchRunsResult
            let nextChannelEndpoints = await channelEndpointsResult
            let nextWorkspaces = await workspacesResult
            let nextConfiguredBots = await configuredBotsResult
            let nextBotConsoles = await botConsolesResult
            let nextChannelPlugins = await channelPluginsResult
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }

            let cacheableResults: [AnyCatalogResult] = [
                .init(nextAgents),
                .init(nextTeams),
                .init(nextSkills),
                .init(nextTasksPage),
                .init(nextAutomations),
                .init(nextSlashCommands),
                .init(nextMcpServers),
                .init(nextChannelEndpoints),
                .init(nextWorkspaces),
                .init(nextConfiguredBots),
                .init(nextBotConsoles),
                .init(nextChannelPlugins),
            ]
            let cacheableRefreshSucceeded = cacheableResults.allSatisfy(\.isSuccess)

            if case let .success(value) = nextSkills {
                skills = value
            }
            if case let .success(page) = nextTasksPage {
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
            if case let .success(value) = nextAutomations {
                automations = value
            }
            if case let .success(value) = nextSlashCommands {
                slashCommands = value
            }
            if case let .success(value) = nextMcpServers {
                mcpServers = value
            }
            autoResearchRuns = nextAutoResearchRuns ?? autoResearchRuns
            if case let .success(value) = nextChannelEndpoints {
                channelEndpoints = value
            }
            if case let .success(nextWorkspaces) = nextWorkspaces {
                userWorkspacePaths = GaryxMobileWorkspacePresentation.userWorkspacePaths(
                    savedWorkspacePaths: nextWorkspaces.map(\.path)
                )
            }
            if case let .success(value) = nextConfiguredBots {
                configuredBots = value
            }
            if case let .success(value) = nextBotConsoles {
                botConsoles = value
            }
            if case let .success(value) = nextChannelPlugins {
                channelPlugins = value
            }
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
            let stillOwnsAgentTargetsState = agentTargetsStateRequestId == requestId
            if cacheableRefreshSucceeded, stillOwnsAgentTargetsState {
                persistCatalogCacheSnapshot()
                catalogSnapshotRestored = false
            }
            if stillOwnsAgentTargetsState, nextAgents.isFailure || nextTeams.isFailure {
                let message = catalogRefreshFailureMessage(
                    from: [AnyCatalogResult(nextAgents), AnyCatalogResult(nextTeams)]
                )
                    ?? "Agents could not be loaded."
                agentTargetsLoadPhase = hadAgentTargetsBeforeRefresh ? .loaded : .failed(message)
                lastError = message
            } else if ownsAgentTargetsLoadPhase, stillOwnsAgentTargetsState {
                agentTargetsLoadPhase = agentTargets.isEmpty
                    ? .failed("Agents could not be loaded.")
                    : .loaded
            }
            if stillOwnsAgentTargetsState {
                agentTargetsStateRequestId = nil
            }
            if cacheableRefreshSucceeded {
                remoteStateLoadPhase = .loaded
            } else {
                let message = catalogRefreshFailureMessage(
                    from: cacheableResults
                ) ?? "Some catalog data could not be loaded."
                remoteStateLoadPhase = .failed(message)
            }
        } catch {
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            let message = displayMessage(for: error)
            remoteStateLoadPhase = .failed(message)
            if agentTargetsStateRequestId == requestId {
                agentTargetsLoadPhase = hadAgentTargetsBeforeRefresh ? .loaded : .failed(message)
            }
            if agentTargetsStateRequestId == requestId {
                agentTargetsStateRequestId = nil
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

    func isCurrentConnectRefresh(_ requestId: UUID, runtimeGeneration: UUID, scopeId: String? = nil) -> Bool {
        connectRefreshRequestId == requestId && isCurrentGatewayRuntime(runtimeGeneration, scopeId: scopeId)
    }

    func isCurrentGatewayRuntime(_ runtimeGeneration: UUID, scopeId: String? = nil) -> Bool {
        guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
        guard let scopeId else { return true }
        return scopeId == currentGatewayScopeId
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

    private func catalogRefreshFailureMessage(from outcomes: [AnyCatalogResult]) -> String? {
        guard let error = outcomes.first(where: { !$0.isSuccess })?.error else { return nil }
        return displayMessage(for: error)
    }
}

private struct AnyCatalogResult {
    let isSuccess: Bool
    let error: Error?

    init<Value>(_ result: Result<Value, Error>) {
        switch result {
        case .success:
            isSuccess = true
            error = nil
        case .failure(let failure):
            isSuccess = false
            error = failure
        }
    }
}

private func garyxCaptureCatalog<Value>(_ operation: () async throws -> Value) async -> Result<Value, Error> {
    do {
        return .success(try await operation())
    } catch {
        return .failure(error)
    }
}

private extension Result {
    var successValue: Success? {
        if case let .success(value) = self {
            return value
        }
        return nil
    }

    var isFailure: Bool {
        if case .failure = self {
            return true
        }
        return false
    }
}
