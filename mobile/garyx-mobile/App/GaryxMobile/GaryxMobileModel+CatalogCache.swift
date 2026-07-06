import Foundation

extension GaryxMobileModel {
    func resetWorkspaceCatalogState() {
        workspaceCatalogState.reset(to: [])
    }

    func restoreWorkspaceCatalogPaths(_ paths: [String]) {
        workspaceCatalogState.restore(normalizedWorkspacePaths(paths))
    }

    func replaceWorkspaceCatalogPaths(_ paths: [String], persist: Bool = false) {
        workspaceCatalogState.replace(normalizedWorkspacePaths(paths))
        if persist {
            persistCatalogCacheSnapshot()
        }
    }

    func beginWorkspaceCatalogRefresh() {
        workspaceCatalogState.beginRefresh()
    }

    func failWorkspaceCatalogRefresh(_ message: String) {
        workspaceCatalogState.failRefresh(message, keepingStaleValue: !userWorkspacePaths.isEmpty)
    }

    func applyWorkspaceSummaries(_ workspaces: [GaryxWorkspaceSummary], persist: Bool = false) {
        replaceWorkspaceCatalogPaths(workspaces.map(\.path), persist: persist)
    }

    private func normalizedWorkspacePaths(_ paths: [String]) -> [String] {
        GaryxMobileWorkspacePresentation.userWorkspacePaths(savedWorkspacePaths: paths)
    }

    func restoreCachedCatalogState() {
        let key = scopedSettingsKey(GaryxMobileSettingsKeys.catalogCacheSnapshot)
        guard let data = defaults.data(forKey: key) else { return }
        let decoder = JSONDecoder()
        guard let snapshot = try? decoder.decode(GaryxMobileCatalogCacheSnapshot.self, from: data),
              snapshot.version == GaryxMobileCatalogCacheSnapshot.currentVersion else {
            defaults.removeObject(forKey: key)
            return
        }
        catalogSnapshotRestored = true
        let cachedAgents = snapshot.agents.map(\.model)
        GaryxEquatableAssignment.assignIfChanged(current: agents, next: cachedAgents) { agents = $0 }
        let cachedTeams = snapshot.teams.map(\.model)
        GaryxEquatableAssignment.assignIfChanged(current: teams, next: cachedTeams) { teams = $0 }
        skills = snapshot.skills.map(\.model)
        capsules = snapshot.capsules.map(\.model)
        let cachedAutomations = snapshot.automations.map(\.model)
        GaryxEquatableAssignment.assignIfChanged(current: automations, next: cachedAutomations) { automations = $0 }
        slashCommands = snapshot.slashCommands.map(\.model)
        mcpServers = snapshot.mcpServers.map(\.model)
        channelEndpoints = snapshot.channelEndpoints.map(\.model)
        configuredBots = snapshot.configuredBots.map(\.model)
        let configuredBotAccounts = snapshot.configuredBotAccounts.map(\.model)
        if !configuredBotAccounts.isEmpty {
            gatewaySettingsDocument = GaryxConfiguredBotAccountsDocument.settingsDocument(from: configuredBotAccounts)
        }
        botConsoles = snapshot.botConsoles.map(\.model)
        channelPlugins = snapshot.channelPlugins.map(\.model)
        restoreWorkspaceCatalogPaths(snapshot.workspacePaths)
        if !agents.isEmpty || !teams.isEmpty {
            agentTargetsLoadPhase = .loaded
            ensureSelectedAgentTarget()
        }
        ensureSelectedWorkspace()
        if !threads.isEmpty {
            persistRecentThreadsWidgetSnapshot()
        }
    }

    func persistCatalogCacheSnapshot() {
        let snapshot = GaryxMobileCatalogCacheSnapshot(
            agents: agents,
            teams: teams,
            workspacePaths: userWorkspacePaths,
            skills: skills,
            capsules: capsules,
            automations: automations,
            slashCommands: slashCommands,
            mcpServers: mcpServers,
            channelEndpoints: channelEndpoints,
            configuredBots: configuredBots,
            configuredBotAccounts: GaryxConfiguredBotAccountsDocument.accounts(from: gatewaySettingsDocument),
            botConsoles: botConsoles,
            channelPlugins: channelPlugins
        )
        let encoder = JSONEncoder()
        if let data = try? encoder.encode(snapshot) {
            defaults.set(data, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.catalogCacheSnapshot))
        }
    }
}
