import Foundation

internal struct GaryxMobileBotGroup: Identifiable, Equatable {
    let id: String
    let channel: String
    let channelDisplayName: String
    let accountId: String
    let title: String
    let subtitle: String
    let agentId: String?
    let rootBehavior: String
    let status: String
    let endpointCount: Int
    let boundEndpointCount: Int
    let workspaceDir: String?
    let mainThreadId: String?
    let defaultOpenThreadId: String?
    let endpoints: [GaryxChannelEndpoint]
    let conversationNodes: [GaryxBotConversationNode]
    let iconDataUrl: String?
}

internal enum GaryxMobileBotGroupBuilder {
    internal static func groups(
        channelEndpoints: [GaryxChannelEndpoint],
        configuredBots: [GaryxConfiguredBot],
        botConsoles: [GaryxBotConsoleSummary],
        channelPlugins: [GaryxChannelPluginCatalogEntry]
    ) -> [GaryxMobileBotGroup] {
        let endpointsByGroup = Dictionary(grouping: channelEndpoints) { endpoint in
            botGroupKey(channel: endpoint.channel, accountId: endpoint.accountId)
        }
        var configuredByGroup: [String: GaryxConfiguredBot] = [:]
        var groups: [String: GaryxMobileBotGroup] = [:]
        var order: [String] = []
        var orderedKeys = Set<String>()

        func rememberOrder(_ key: String) {
            if orderedKeys.insert(key).inserted {
                order.append(key)
            }
        }

        for bot in configuredBots {
            let key = botGroupKey(channel: bot.channel, accountId: bot.accountId)
            if configuredByGroup[key] == nil {
                configuredByGroup[key] = bot
            }
            rememberOrder(key)
        }

        func remember(_ group: GaryxMobileBotGroup) {
            let key = botGroupKey(channel: group.channel, accountId: group.accountId)
            rememberOrder(key)
            groups[key] = group
        }

        func iconDataUrl(for channel: String) -> String? {
            GaryxChannelIconResolver.iconDataUrl(for: channel, plugins: channelPlugins)
        }

        func displayName(for channel: String) -> String {
            GaryxChannelIdentityPresentation.displayName(
                for: channel,
                catalogDisplayName: GaryxChannelIconResolver.displayName(for: channel, plugins: channelPlugins)
            )
        }

        for console in botConsoles {
            let key = botGroupKey(channel: console.channel, accountId: console.accountId)
            let endpoints = endpointsByGroup[key] ?? []
            let decodedEndpointCount = endpoints.count
            let decodedBoundCount = endpoints.filter { $0.threadId?.isEmpty == false }.count
            let configured = configuredByGroup[key]
            remember(
                GaryxMobileBotGroup(
                    id: console.id.isEmpty ? "\(console.channel)::\(console.accountId)" : console.id,
                    channel: console.channel,
                    channelDisplayName: displayName(for: console.channel),
                    accountId: console.accountId,
                    title: console.title,
                    subtitle: console.subtitle,
                    agentId: nonEmpty(console.agentId) ?? nonEmpty(configured?.agentId),
                    rootBehavior: console.rootBehavior,
                    status: console.status,
                    endpointCount: max(console.endpointCount, decodedEndpointCount),
                    boundEndpointCount: max(console.boundEndpointCount, decodedBoundCount),
                    workspaceDir: nonEmpty(console.workspaceDir) ?? nonEmpty(configured?.workspaceDir),
                    mainThreadId: nonEmpty(console.mainThreadId) ?? nonEmpty(configured?.mainThreadId),
                    defaultOpenThreadId: nonEmpty(console.defaultOpenThreadId)
                        ?? nonEmpty(configured?.defaultOpenThreadId)
                        ?? nonEmpty(configured?.mainThreadId),
                    endpoints: endpoints,
                    conversationNodes: console.conversationNodes,
                    iconDataUrl: iconDataUrl(for: console.channel)
                )
            )
        }

        for bot in configuredBots {
            let key = botGroupKey(channel: bot.channel, accountId: bot.accountId)
            if groups[key] != nil {
                continue
            }
            let endpoints = endpointsByGroup[key] ?? []
            remember(
                GaryxMobileBotGroup(
                    id: "\(bot.channel)::\(bot.accountId)",
                    channel: bot.channel,
                    channelDisplayName: displayName(for: bot.channel),
                    accountId: bot.accountId,
                    title: bot.displayName,
                    subtitle: "\(displayName(for: bot.channel)) Bot · \(bot.accountId)",
                    agentId: nonEmpty(bot.agentId),
                    rootBehavior: bot.rootBehavior,
                    status: bot.enabled ? "idle" : "disabled",
                    endpointCount: endpoints.count,
                    boundEndpointCount: endpoints.filter { $0.threadId?.isEmpty == false }.count,
                    workspaceDir: nonEmpty(bot.workspaceDir),
                    mainThreadId: nonEmpty(bot.mainThreadId),
                    defaultOpenThreadId: nonEmpty(bot.defaultOpenThreadId) ?? nonEmpty(bot.mainThreadId),
                    endpoints: endpoints,
                    conversationNodes: [],
                    iconDataUrl: iconDataUrl(for: bot.channel)
                )
            )
        }

        for (key, endpoints) in endpointsByGroup.sorted(by: { $0.key < $1.key }) where groups[key] == nil {
            guard let first = endpoints.first else { continue }
            remember(
                GaryxMobileBotGroup(
                    id: key,
                    channel: first.channel,
                    channelDisplayName: displayName(for: first.channel),
                    accountId: first.accountId,
                    title: "\(displayName(for: first.channel)) / \(first.accountId)",
                    subtitle: "\(displayName(for: first.channel)) Bot · \(first.accountId)",
                    agentId: nil,
                    rootBehavior: "open_default",
                    status: "idle",
                    endpointCount: endpoints.count,
                    boundEndpointCount: endpoints.filter { $0.threadId?.isEmpty == false }.count,
                    workspaceDir: nil,
                    mainThreadId: nil,
                    defaultOpenThreadId: endpoints.first(where: { $0.threadId?.isEmpty == false })?.threadId,
                    endpoints: endpoints,
                    conversationNodes: [],
                    iconDataUrl: iconDataUrl(for: first.channel)
                )
            )
        }

        return order.compactMap { groups[$0] }
    }

    internal static func selectedGroup(threadId: String?, groups: [GaryxMobileBotGroup]) -> GaryxMobileBotGroup? {
        guard let threadId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return nil
        }
        return groups.first { group in
            if group.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) == threadId {
                return true
            }
            if group.defaultOpenThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) == threadId {
                return true
            }
            return group.endpoints.contains { endpoint in
                endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) == threadId
            }
        }
    }

    private static func botGroupKey(channel: String, accountId: String) -> String {
        "\(channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())::\(accountId.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())"
    }

    private static func nonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}

internal struct GaryxBotSidebarConversationEntry: Identifiable, Equatable {
    let id: String
    let title: String
    let subtitle: String?
    let threadId: String?
    let latestActivity: String?
    let openable: Bool
    let endpoint: GaryxChannelEndpoint
}

extension GaryxBotSidebarConversationEntry {
    internal func fallbackThreadSummary(workspacePath: String?) -> GaryxThreadSummary? {
        guard let threadId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return nil
        }
        return GaryxThreadSummary(
            id: threadId,
            title: title.isEmpty ? "Thread" : title,
            createdAt: nil,
            updatedAt: latestActivity,
            lastMessagePreview: subtitle ?? "",
            workspacePath: workspacePath,
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
}

extension GaryxMobileBotGroup {
    internal var rootCanOpen: Bool {
        let mainThreadId = self.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let defaultOpenThreadId = self.defaultOpenThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return rootBehavior != "expand_only" || !mainThreadId.isEmpty || !defaultOpenThreadId.isEmpty
    }

    internal var compactDetailLine: String {
        let channelName = channelDisplayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? GaryxChannelIdentityPresentation.displayName(for: channel)
            : channelDisplayName
        let account = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        let botId = account.isEmpty ? channelName : "\(channelName) · \(account)"
        let agent = agentId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = workspaceDir?.trimmingCharacters(in: .whitespacesAndNewlines)
        return [
            botId,
            agent.flatMap { $0.isEmpty ? nil : $0 },
            workspace.flatMap { $0.isEmpty ? nil : $0.garyxLastPathComponent },
        ]
        .compactMap { $0 }
        .joined(separator: " / ")
    }

    internal func sidebarChildConversationEntries() -> [GaryxBotSidebarConversationEntry] {
        var entries: [GaryxBotSidebarConversationEntry] = []
        var seenThreadIds = Set<String>()
        let rootThreadId = mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        if !conversationNodes.isEmpty {
            for node in conversationNodes {
                let threadId = node.endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let threadId, !threadId.isEmpty else {
                    continue
                }
                if threadId == rootThreadId {
                    continue
                }
                if seenThreadIds.contains(threadId) {
                    continue
                }
                seenThreadIds.insert(threadId)
                entries.append(
                    GaryxBotSidebarConversationEntry(
                        id: node.id.isEmpty ? node.endpoint.endpointKey : node.id,
                        title: node.title.isEmpty ? node.endpoint.displayLabel : node.title,
                        subtitle: node.badge ?? node.endpoint.conversationLabel ?? node.endpoint.threadLabel,
                        threadId: threadId,
                        latestActivity: node.latestActivity,
                        openable: node.openable,
                        endpoint: node.endpoint
                    )
                )
            }
            return entries.sorted(by: garyxBotConversationEntrySort)
        }

        for endpoint in endpoints {
            let threadId = endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let threadId, !threadId.isEmpty else {
                continue
            }
            if threadId == rootThreadId {
                continue
            }
            if seenThreadIds.contains(threadId) {
                continue
            }
            seenThreadIds.insert(threadId)
            entries.append(
                GaryxBotSidebarConversationEntry(
                    id: endpoint.endpointKey,
                    title: endpoint.displayLabel.isEmpty ? (endpoint.threadLabel ?? "Thread") : endpoint.displayLabel,
                    subtitle: endpoint.conversationLabel ?? endpoint.threadLabel ?? endpoint.workspaceDir?.garyxLastPathComponent,
                    threadId: threadId,
                    latestActivity: endpoint.lastInboundAt ?? endpoint.lastDeliveryAt,
                    openable: true,
                    endpoint: endpoint
                )
            )
        }

        return entries.sorted(by: garyxBotConversationEntrySort)
    }
}

private func garyxBotConversationEntrySort(
    _ lhs: GaryxBotSidebarConversationEntry,
    _ rhs: GaryxBotSidebarConversationEntry
) -> Bool {
    let titleOrder = lhs.title.localizedCaseInsensitiveCompare(rhs.title)
    if titleOrder != .orderedSame {
        return titleOrder == .orderedAscending
    }
    return lhs.id.localizedCaseInsensitiveCompare(rhs.id) == .orderedAscending
}
