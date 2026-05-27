import Foundation

extension GaryxMobileModel {
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
        agents = snapshot.agents.map(\.model)
        teams = snapshot.teams.map(\.model)
        skills = snapshot.skills.map(\.model)
        tasks = snapshot.tasks.map(\.model)
        automations = snapshot.automations.map(\.model)
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
        userWorkspacePaths = GaryxMobileWorkspacePresentation.userWorkspacePaths(
            savedWorkspacePaths: snapshot.workspacePaths
        )
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
            tasks: tasks,
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
