import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func saveGatewaySettings() {
        gatewaySettingsStatus = nil
        gatewayURL = normalizedGatewayURL(gatewayURL)
        gatewayAuthToken = gatewayAuthToken.trimmingCharacters(in: .whitespacesAndNewlines)
        gatewayHeaders = GaryxGatewayHeaders.normalizedBlock(gatewayHeaders)
        defaults.set(gatewayURL, forKey: GaryxMobileSettingsKeys.gatewayUrl)
        defaults.set(gatewayHeaders, forKey: GaryxMobileSettingsKeys.gatewayHeaders)
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
        resetWorkspaceCatalogState()
        restoreCachedCatalogState()
        let workspace = newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
        if !workspace.isEmpty, workspaceGitStatuses[workspace] == nil {
            Task { await refreshWorkspaceGitStatus(for: workspace) }
        }
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
        hasAttemptedLastOpenedThreadRestore = false
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
        selectedThreadHistoryRequestId = nil
        selectedThreadHistoryRetryTask?.cancel()
        selectedThreadHistoryRetryTask = nil
        selectedThreadHistoryRetryThreadId = nil
        selectedThreadHistoryRetryCount = 0
        threadHistoryLoadedIds = []
        threadOpenState.invalidate()
        completedThreadHistoryHydrationTasks.values.forEach { $0.cancel() }
        completedThreadHistoryHydrationTasks = [:]
        resetSelectedThreadHistoryPagination()
        resetThreadListPagination()
        sceneRefreshTask?.cancel()
        sceneRefreshTask = nil
        cancelSelectedThreadReconcileLoop()
        cancelBackgroundCommittedRunReconcileLoop()
        stopSelectedThreadStream()
        resetClaudeCodeAuthFlow()
        selectedThreadActivitySignatures = [:]
        clearActiveRunState()
        connectRefreshRequestId = nil
        remoteStateRefreshRequestId = nil
        agentTargetsRefreshRequestId = nil
        agentTargetsStateRequestId = nil
        workspaceRefreshRequestId = nil
        agentTargetsLoadPhase = .idle
        connectionState = .disconnected
        threads = []
        pinnedThreadIds = []
        recentThreadIds = []
        pendingThreadArchives = GaryxPendingThreadArchiveState()
        GaryxMobileWidgetStore.clear()
        WidgetCenter.shared.reloadTimelines(ofKind: GaryxRecentThreadsWidgetConstants.kind)
        GaryxUsageWidgetStore.clear()
        WidgetCenter.shared.reloadTimelines(ofKind: GaryxCodingUsageWidgetConstants.kind)
        selectedThread = nil
        messages = []
        messagesByThread = [:]
        messageSignaturesByThread = [:]
        // The persisted transcript cache is keyed by thread id only, so drop it on
        // a gateway/profile switch to avoid showing another backend's cached thread
        // (and to bound on-disk growth). clearAll bumps every present thread's
        // mirror generation (monotonic) so an in-flight cold-open restore aborts.
        transcriptMirror.clearAll()
        threadResidencyTracker.removeAll()
        transcriptCacheStore.clearAll()
        activeAssistantMessageIdsByThread = [:]
        pendingDirectFollowUpsByThread = [:]
        clearAllComposerDrafts()
        draftThreadTitle = ""
        agents = []
        skills = []
        galleryFocusedCapsule = nil
        conversationCapsulePreview = nil
        capsules = []
        capsuleHTMLCache = [:]
        automations = []
        slashCommands = []
        mcpServers = []
        channelEndpoints = []
        configuredBots = []
        botConsoles = []
        resetWorkspaceCatalogState()
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
        skillEditorLoadRequestId = nil
        skillFileLoadRequestId = nil
        selectedSkillEditor = nil
        selectedSkillDocument = nil
        selectedAutomationEditor = nil
        selectedAgentDetail = nil
        routeNotFoundStore.selection = nil
        isLoadingThreads = false
        remoteStateLoadPhase = .idle
        isLoadingSelectedThreadHistory = false
    }

    func selectGatewayProfile(_ profile: GaryxGatewayProfile) {
        saveGatewayScopedUserState()
        resetGatewayRuntimeState()
        gatewayURL = profile.gatewayUrl
        gatewayAuthToken = keychain.readGatewayProfileToken(profileId: profile.id)
        gatewayHeaders = profile.gatewayHeaders
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
        token: String,
        headers: String
    ) -> Bool {
        let normalizedURL = normalizedGatewayURL(gatewayUrl)
        guard parsedGatewayURL(from: normalizedURL) != nil else {
            lastError = "Invalid gateway URL"
            return false
        }
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedHeaders = GaryxGatewayHeaders.normalizedBlock(headers)
        let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextId = GaryxGatewayProfileStorage.stableId(for: normalizedURL)
        let currentURL = normalizedGatewayURL(gatewayURL)
        let currentProfileId = currentGatewayProfile?.id
        let affectsCurrentProfile = currentProfileId == profile.id
            || currentProfileId == nextId
            || currentURL.lowercased() == normalizedURL.lowercased()
        let currentURLChanged = currentURL.lowercased() != normalizedURL.lowercased()
        let activeTokenChanged = gatewayAuthToken != trimmedToken
        let activeHeadersChanged = gatewayHeaders != normalizedHeaders
        var nextProfile = profile
        nextProfile.id = nextId
        nextProfile.label = trimmedLabel.isEmpty ? GaryxGatewayProfileStorage.label(for: normalizedURL) : trimmedLabel
        nextProfile.gatewayUrl = normalizedURL
        nextProfile.gatewayHeaders = normalizedHeaders
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
            let didResetRuntime = currentURLChanged || activeTokenChanged || activeHeadersChanged
            if didResetRuntime {
                resetGatewayRuntimeState()
            }
            gatewayURL = normalizedURL
            gatewayAuthToken = trimmedToken
            gatewayHeaders = normalizedHeaders
            defaults.set(gatewayURL, forKey: GaryxMobileSettingsKeys.gatewayUrl)
            defaults.set(gatewayHeaders, forKey: GaryxMobileSettingsKeys.gatewayHeaders)
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
            let plan = GaryxForegroundSyncPlan.plan(
                connectionState: connectionState,
                selectedThreadId: selectedThreadId
            )
            sceneRefreshTask = Task { [weak self] in
                guard let self else { return }
                // If the connection dropped/changed while backgrounded, reconnect
                // first so the open thread can converge to the server's latest
                // state on return. The previous logic no-op'd whenever the cached
                // connection state was not `.ready`, leaving the open thread frozen
                // until a manual re-open (#TASK-1449 symptom 2).
                if plan.reconnect, canConnectGateway {
                    await connectAndRefresh()
                    guard !Task.isCancelled else { return }
                }
                guard case .ready = connectionState else { return }
                startBackgroundCommittedRunReconcileLoop()
                startSelectedThreadReconcileLoop()
                async let agentTargetsRefresh: Void = refreshAgentTargets()
                await refreshThreads(source: .userAction)
                await agentTargetsRefresh
                guard !Task.isCancelled else { return }
                if plan.resyncOpenThread, let selectedThreadId, selectedThread?.id == selectedThreadId {
                    await loadSelectedThreadHistory()
                    // Re-establish the resumable per-thread stream (it was stopped
                    // on background); it resumes from the cursor and cancels the
                    // baseline reconcile poll started above.
                    if plan.restartStream {
                        startSelectedThreadStream(for: selectedThreadId)
                    }
                }
                guard !Task.isCancelled else { return }
                await refreshCodingUsageWidget()
            }
        case .background:
            // Remember where the user left: only an exit from the
            // conversation page restores that thread on the next launch.
            persistLastSessionLocation()
            sceneRefreshTask?.cancel()
            sceneRefreshTask = nil
            cancelSelectedThreadReconcileLoop()
            cancelBackgroundCommittedRunReconcileLoop()
            stopSelectedThreadStream()
        default:
            break
        }
    }

    func rememberCurrentGatewayProfile(label: String? = nil) {
        let url = normalizedGatewayURL(gatewayURL)
        guard !url.isEmpty else { return }
        let profile = GaryxGatewayProfile(
            id: GaryxGatewayProfileStorage.stableId(for: url),
            label: GaryxGatewayProfileStorage.preservedLabel(
                explicit: label,
                existing: currentGatewayProfile?.label,
                gatewayUrl: url
            ),
            gatewayUrl: url,
            gatewayHeaders: GaryxGatewayHeaders.normalizedBlock(gatewayHeaders),
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
        pendingMobileRoute = nil
        saveGatewayScopedUserState()
        resetGatewayRuntimeState()
        gatewayURL = payload.gatewayUrl
        gatewayAuthToken = payload.gatewayAuthToken
        gatewayHeaders = payload.gatewayHeaders
        loadGatewayScopedUserState(fallbackToLegacy: false)
        await connectAndRefresh()
    }

    func handleOpenURL(_ url: URL) async {
        if let route = GaryxMobileRouteLink.parse(url) {
            await openMobileRouteFromLink(route)
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
            // Bound the initial reachability check so an unreachable gateway
            // fails into the chooser quickly instead of hanging on the
            // system request timeout.
            let status = try await withConnectTimeout { try await gateway.status() }
            _ = try await withConnectTimeout { try await gateway.chatHealth() }
            guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                return
            }
            saveGatewaySettings()
            rememberCurrentGatewayProfile()
            gatewaySettingsStatus = "Saved and connected"
            connectionState = .ready(version: status.version)
            startBackgroundCommittedRunReconcileLoop()
            if threadOpenState.hasPendingIntent {
                await openPendingThreadLinkIfNeeded()
                guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                    return
                }
            }
            await restoreLastOpenedThreadIfNeeded()
            guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                return
            }
            async let agentTargetsRefresh: Void = refreshAgentTargets()
            await refreshThreads(source: .userAction)
            await agentTargetsRefresh
            await refreshRemoteState()
            guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                return
            }
            await refreshCodingUsageWidget()
            connectRefreshRequestId = nil
            await openPendingMobileRouteIfNeeded()
            startBackgroundCommittedRunReconcileLoop()
            startSelectedThreadReconcileLoop()
        } catch {
            guard isCurrentConnectRefresh(requestId, runtimeGeneration: runtimeGeneration, scopeId: gatewayScopeId) else {
                return
            }
            connectRefreshRequestId = nil
            cancelBackgroundCommittedRunReconcileLoop()
            cancelSelectedThreadReconcileLoop()
            let message = displayMessage(for: error)
            connectionState = .failed(message)
            lastError = message
        }
    }

    private static let connectCheckTimeoutNanos: UInt64 = 5_000_000_000

    private func withConnectTimeout<T: Sendable>(
        _ operation: @escaping @Sendable () async throws -> T
    ) async throws -> T {
        try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask { try await operation() }
            group.addTask {
                try await Task.sleep(nanoseconds: Self.connectCheckTimeoutNanos)
                throw GaryxGatewayConnectTimeoutError()
            }
            guard let result = try await group.next() else {
                throw GaryxGatewayConnectTimeoutError()
            }
            group.cancelAll()
            return result
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
            let agentsOutcome = await garyxCaptureCatalog { try await gateway.listAgents() }
            guard isCurrentAgentTargetsRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            agentTargetsRefreshRequestId = nil
            if agentTargetsStateRequestId == requestId {
                agentTargetsStateRequestId = nil
            }
            let nextAgents = agentsOutcome.successValue
            if let nextAgents {
                applyAgentTargets(agents: nextAgents)
            }
            if agentsOutcome.isFailure {
                let message = catalogRefreshFailureMessage(
                    from: [AnyCatalogResult(agentsOutcome)]
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
        workspaceRefreshRequestId = requestId
        let supersededAgentTargetsRefresh = agentTargetsRefreshRequestId != nil
        agentTargetsRefreshRequestId = nil
        agentTargetsStateRequestId = requestId
        let hadAgentTargetsBeforeRefresh = !agentTargets.isEmpty
        let ownsAgentTargetsLoadPhase = agentTargets.isEmpty
            || supersededAgentTargetsRefresh
            || agentTargetsLoadPhase.isLoading
        remoteStateLoadPhase = .loading
        beginWorkspaceCatalogRefresh()
        if ownsAgentTargetsLoadPhase {
            agentTargetsLoadPhase = .loading
        }
        do {
            let gateway = try client()
            async let agentsResult = garyxCaptureCatalog { try await gateway.listAgents() }
            async let skillsResult = garyxCaptureCatalog { try await gateway.listSkills() }
            async let capsulesResult = garyxCaptureCatalog { try await gateway.listCapsules() }
            async let gatewaySettingsResult: [String: GaryxJSONValue]? = try? gateway.gatewaySettings()
            async let automationsResult = garyxCaptureCatalog { try await gateway.listAutomations() }
            async let slashCommandsResult = garyxCaptureCatalog { try await gateway.listSlashCommands() }
            async let mcpServersResult = garyxCaptureCatalog { try await gateway.listMcpServers() }
            async let channelEndpointsResult = garyxCaptureCatalog { try await gateway.listChannelEndpoints() }
            async let workspacesResult = garyxCaptureCatalog { try await gateway.listWorkspaces() }
            async let configuredBotsResult = garyxCaptureCatalog { try await gateway.listConfiguredBots() }
            async let botConsolesResult = garyxCaptureCatalog { try await gateway.listBotConsoles() }
            async let channelPluginsResult = garyxCaptureCatalog { try await gateway.listChannelPlugins() }

            let nextWorkspaces = await workspacesResult
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            if workspaceRefreshRequestId == requestId {
                workspaceRefreshRequestId = nil
                switch nextWorkspaces {
                case .success(let workspaces):
                    applyWorkspaceSummaries(workspaces, persist: true)
                    ensureSelectedWorkspace()
                case .failure(let error):
                    failWorkspaceCatalogRefresh(displayMessage(for: error))
                }
            }

            let nextAgents = await agentsResult
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }
            let ownsAgentTargetsState = agentTargetsStateRequestId == requestId
            if ownsAgentTargetsState, let agents = nextAgents.successValue {
                applyAgentTargets(agents: agents)
            }

            let nextSkills = await skillsResult
            let nextCapsules = await capsulesResult
            let nextGatewaySettings = await gatewaySettingsResult
            let nextAutomations = await automationsResult
            let nextSlashCommands = await slashCommandsResult
            let nextMcpServers = await mcpServersResult
            let nextChannelEndpoints = await channelEndpointsResult
            let nextConfiguredBots = await configuredBotsResult
            let nextBotConsoles = await botConsolesResult
            let nextChannelPlugins = await channelPluginsResult
            guard isCurrentRemoteStateRefresh(requestId, runtimeGeneration: runtimeGeneration) else { return }

            let cacheableResults: [AnyCatalogResult] = [
                .init(nextAgents),
                .init(nextSkills),
                .init(nextCapsules),
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
            if case let .success(value) = nextCapsules {
                capsules = value
            }
            if let settings = nextGatewaySettings {
                gatewaySettingsDocument = settings
            }
            if case let .success(value) = nextAutomations {
                GaryxEquatableAssignment.assignIfChanged(
                    current: automations,
                    next: value
                ) { automations = $0 }
            }
            if case let .success(value) = nextSlashCommands {
                slashCommands = value
            }
            if case let .success(value) = nextMcpServers {
                mcpServers = value
            }
            if case let .success(value) = nextChannelEndpoints {
                channelEndpoints = value
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
            if stillOwnsAgentTargetsState, nextAgents.isFailure {
                let message = catalogRefreshFailureMessage(
                    from: [AnyCatalogResult(nextAgents)]
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
            if workspaceRefreshRequestId == requestId {
                workspaceRefreshRequestId = nil
                failWorkspaceCatalogRefresh(message)
            }
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
