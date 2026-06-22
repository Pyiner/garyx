import Foundation

internal struct GaryxMobileBotGroup: Identifiable, Equatable, Sendable {
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
        let consolesByGroup = Dictionary(grouping: botConsoles) { console in
            botGroupKey(channel: console.channel, accountId: console.accountId)
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

        return configuredBots.map { bot in
            let key = botGroupKey(channel: bot.channel, accountId: bot.accountId)
            let console = consolesByGroup[key]?.first
            let endpoints = endpointsByGroup[key] ?? []
            let decodedEndpointCount = endpoints.count
            let decodedBoundCount = endpoints.filter { $0.threadId?.isEmpty == false }.count
            return GaryxMobileBotGroup(
                id: "\(bot.channel)::\(bot.accountId)",
                channel: bot.channel,
                channelDisplayName: displayName(for: bot.channel),
                accountId: bot.accountId,
                title: bot.displayName,
                subtitle: "\(displayName(for: bot.channel)) Bot · \(bot.accountId)",
                agentId: nonEmpty(bot.agentId) ?? nonEmpty(console?.agentId),
                rootBehavior: bot.rootBehavior,
                status: bot.enabled ? (console?.status ?? "idle") : "disabled",
                endpointCount: max(console?.endpointCount ?? 0, decodedEndpointCount),
                boundEndpointCount: max(console?.boundEndpointCount ?? 0, decodedBoundCount),
                workspaceDir: nonEmpty(bot.workspaceDir) ?? nonEmpty(console?.workspaceDir),
                mainThreadId: nonEmpty(bot.mainThreadId) ?? nonEmpty(console?.mainThreadId),
                defaultOpenThreadId: nonEmpty(bot.defaultOpenThreadId)
                    ?? nonEmpty(bot.mainThreadId)
                    ?? nonEmpty(console?.defaultOpenThreadId)
                    ?? nonEmpty(console?.mainThreadId),
                endpoints: endpoints,
                conversationNodes: console?.conversationNodes ?? [],
                iconDataUrl: iconDataUrl(for: bot.channel)
            )
        }
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

internal struct GaryxBotSidebarConversationEntry: Identifiable, Equatable, Sendable {
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
