import Foundation

extension GaryxMobileModel {
    func resetWorkspaceCatalogState() {
        workspaceCatalogState.reset(to: .empty)
    }

    func restoreWorkspaceCatalog(_ catalog: GaryxWorkspaceCatalog) {
        workspaceCatalogState.restore(catalog)
    }

    /// Server order and names land verbatim: the catalog is rendered exactly
    /// as delivered, with no client-side sorting, renaming, or filtering.
    func replaceWorkspaceCatalog(_ catalog: GaryxWorkspaceCatalog, persist: Bool = false) {
        workspaceCatalogState.replace(catalog)
        if persist {
            persistCatalogCacheSnapshot()
        }
    }

    func beginWorkspaceCatalogRefresh() {
        workspaceCatalogState.beginRefresh()
    }

    func failWorkspaceCatalogRefresh(_ message: String) {
        workspaceCatalogState.failRefresh(
            message,
            keepingStaleValue: !workspaceCatalog.workspaces.isEmpty
        )
    }

    func applyWorkspacesPage(_ page: GaryxWorkspacesPage, persist: Bool = false) {
        replaceWorkspaceCatalog(GaryxWorkspaceCatalog(page: page), persist: persist)
        resolveDraftWorkspaceSelectionIfNeeded()
    }

    func restoreCachedCatalogState() {
        let key = scopedSettingsKey(GaryxMobileSettingsKeys.catalogCacheSnapshot)
        guard let data = defaults.data(forKey: key) else { return }
        let decoder = JSONDecoder()
        guard let snapshot = try? decoder.decode(GaryxMobileCatalogCacheSnapshot.self, from: data),
              GaryxMobileCatalogCachePolicy.shouldRestore(version: snapshot.version) else {
            defaults.removeObject(forKey: key)
            return
        }
        catalogSnapshotRestored = true
        gatewayDefaultAgentId = snapshot.gatewayDefaultAgentId
        effectiveDefaultAgentId = snapshot.effectiveDefaultAgentId
        let cachedAgents = snapshot.agents.map(\.model)
        GaryxEquatableAssignment.assignIfChanged(current: agents, next: cachedAgents) { agents = $0 }
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
        restoreWorkspaceCatalog(snapshot.workspaceCatalog)
        if !agents.isEmpty {
            agentTargetsLoadPhase = .loaded
        }
        ensureSelectedWorkspace()
        if !residentRecentThreadSummaries.isEmpty {
            persistRecentThreadsWidgetSnapshot()
        }
    }

    func persistCatalogCacheSnapshot() {
        let snapshot = GaryxMobileCatalogCacheSnapshot(
            agents: agents,
            gatewayDefaultAgentId: gatewayDefaultAgentId,
            effectiveDefaultAgentId: effectiveDefaultAgentId,
            workspaceCatalog: workspaceCatalog,
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
