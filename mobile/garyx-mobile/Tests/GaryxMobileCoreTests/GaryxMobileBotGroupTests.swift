import XCTest
@testable import GaryxMobileCore

final class GaryxMobileBotGroupTests: XCTestCase {
    func testConversationNodesDriveSidebarEntriesWhenAvailable() throws {
        let root = try makeEndpoint(key: "telegram:test-bot:root", label: "Root", threadId: "thread-root")
        let alpha = try makeEndpoint(
            key: "telegram:test-bot:alpha",
            label: "Alpha endpoint",
            threadId: "thread-alpha",
            threadLabel: "Alpha thread",
            conversationLabel: "Alpha chat"
        )
        let duplicate = try makeEndpoint(
            key: "telegram:test-bot:alpha-duplicate",
            label: "Duplicate endpoint",
            threadId: "thread-alpha"
        )
        let endpointOnly = try makeEndpoint(
            key: "telegram:test-bot:endpoint-only",
            label: "Endpoint only",
            threadId: "thread-endpoint"
        )
        let group = makeGroup(
            mainThreadId: "thread-root",
            endpoints: [endpointOnly],
            conversationNodes: [
                try makeNode(id: "root-node", title: "Root", endpoint: root),
                try makeNode(
                    id: "alpha-node",
                    title: "Alpha node",
                    badge: "Direct",
                    latestActivity: "2026-01-02T03:04:05Z",
                    openable: true,
                    endpoint: alpha
                ),
                try makeNode(id: "duplicate-node", title: "Duplicate", endpoint: duplicate),
            ]
        )

        let entries = group.sidebarChildConversationEntries()

        XCTAssertEqual(entries.map(\.id), ["alpha-node"])
        XCTAssertEqual(entries.first?.title, "Alpha node")
        XCTAssertEqual(entries.first?.subtitle, "Direct")
        XCTAssertEqual(entries.first?.threadId, "thread-alpha")
        XCTAssertEqual(entries.first?.latestActivity, "2026-01-02T03:04:05Z")
        XCTAssertEqual(entries.first?.openable, true)
    }

    func testEndpointsBecomeSidebarEntriesWhenConversationNodesAreAbsent() throws {
        let root = try makeEndpoint(key: "discord:test-bot:root", label: "Root", threadId: "thread-root")
        let alpha = try makeEndpoint(
            key: "discord:test-bot:alpha",
            label: "Alpha",
            threadId: "thread-alpha",
            threadLabel: "Alpha thread",
            lastInboundAt: "2026-02-03T04:05:06Z",
            conversationLabel: "Alpha room"
        )
        let duplicate = try makeEndpoint(
            key: "discord:test-bot:alpha-duplicate",
            label: "Alpha duplicate",
            threadId: "thread-alpha"
        )
        let beta = try makeEndpoint(
            key: "discord:test-bot:beta",
            label: "",
            threadId: "thread-beta",
            threadLabel: "Beta thread",
            workspaceDir: "/workspace/test-project",
            lastDeliveryAt: "2026-02-04T04:05:06Z"
        )
        let unbound = try makeEndpoint(key: "discord:test-bot:unbound", label: "Unbound")
        let group = makeGroup(
            mainThreadId: "thread-root",
            endpoints: [beta, alpha, duplicate, root, unbound],
            conversationNodes: []
        )

        let entries = group.sidebarChildConversationEntries()

        XCTAssertEqual(entries.map { $0.threadId }, ["thread-alpha", "thread-beta"])
        XCTAssertEqual(entries.map { $0.title }, ["Alpha", "Beta thread"])
        XCTAssertEqual(entries.map { $0.subtitle }, ["Alpha room", "Beta thread"])
        XCTAssertEqual(entries.map { $0.latestActivity }, ["2026-02-03T04:05:06Z", "2026-02-04T04:05:06Z"])
        XCTAssertTrue(entries.allSatisfy { $0.openable })
    }

    func testRootOpenabilityAndCompactDetailLine() {
        let expandOnly = makeGroup(
            channel: "custom_channel",
            accountId: " bot-1 ",
            agentId: "agent-alpha",
            rootBehavior: "expand_only",
            workspaceDir: "/workspace/project-alpha"
        )
        XCTAssertFalse(expandOnly.rootCanOpen)
        XCTAssertEqual(expandOnly.compactDetailLine, "Custom Channel · bot-1 / agent-alpha / project-alpha")

        let openDefault = makeGroup(rootBehavior: "open_default")
        XCTAssertTrue(openDefault.rootCanOpen)

        let expandWithDefaultThread = makeGroup(rootBehavior: "expand_only", defaultOpenThreadId: "thread-default")
        XCTAssertTrue(expandWithDefaultThread.rootCanOpen)
    }

    func testCompactDetailLineHandlesMissingOptionalPartsAndKnownChannels() {
        XCTAssertEqual(
            makeGroup(channel: "feishu", accountId: "", agentId: nil, workspaceDir: nil).compactDetailLine,
            "Feishu"
        )
        XCTAssertEqual(
            makeGroup(channel: "weixin", accountId: "bot-1", agentId: nil, workspaceDir: nil).compactDetailLine,
            "Weixin · bot-1"
        )
        XCTAssertEqual(
            makeGroup(channel: "custom_channel", accountId: "bot-1", agentId: nil, workspaceDir: "/workspace/project-alpha")
                .compactDetailLine,
            "Custom Channel · bot-1 / project-alpha"
        )
    }

    func testFallbackThreadSummaryUsesEntryDisplayData() throws {
        let endpoint = try makeEndpoint(key: "telegram:test-bot:alpha", label: "Alpha", threadId: "thread-alpha")
        let entry = GaryxBotSidebarConversationEntry(
            id: "entry-alpha",
            title: "",
            subtitle: "Recent chat",
            threadId: " thread-alpha ",
            latestActivity: "2026-03-04T05:06:07Z",
            openable: true,
            endpoint: endpoint
        )

        let summary = try XCTUnwrap(entry.fallbackThreadSummary(workspacePath: "/workspace/project-alpha"))

        XCTAssertEqual(summary.id, "thread-alpha")
        XCTAssertEqual(summary.title, "Thread")
        XCTAssertEqual(summary.updatedAt, "2026-03-04T05:06:07Z")
        XCTAssertEqual(summary.lastMessagePreview, "Recent chat")
        XCTAssertEqual(summary.workspacePath, "/workspace/project-alpha")
    }

    func testBuilderMergesConsoleConfiguredBotEndpointAndIconData() throws {
        let endpoint = try makeEndpoint(
            key: "telegram:test-bot:alpha",
            label: "Alpha",
            threadId: "thread-alpha"
        )
        let configured = try makeConfiguredBot(
            channel: "telegram",
            accountId: "test-bot",
            displayName: "Configured Bot",
            agentId: "agent-config",
            workspaceDir: "/workspace/config",
            mainThreadId: "thread-main-config"
        )
        let console = try makeConsole(
            id: "console-test-bot",
            channel: "telegram",
            accountId: "test-bot",
            title: "Console Bot",
            subtitle: "Console subtitle",
            agentId: nil,
            workspaceDir: nil,
            mainThreadId: nil,
            defaultOpenThreadId: nil
        )
        let iconDataUrl = "data:image/png;base64,AA=="

        let groups = GaryxMobileBotGroupBuilder.groups(
            channelEndpoints: [endpoint],
            configuredBots: [configured],
            botConsoles: [console],
            channelPlugins: [try makePlugin(id: "telegram", iconDataUrl: iconDataUrl)]
        )

        let group = try XCTUnwrap(groups.first)
        XCTAssertEqual(groups.count, 1)
        XCTAssertEqual(group.id, "console-test-bot")
        XCTAssertEqual(group.title, "Console Bot")
        XCTAssertEqual(group.agentId, "agent-config")
        XCTAssertEqual(group.workspaceDir, "/workspace/config")
        XCTAssertEqual(group.mainThreadId, "thread-main-config")
        XCTAssertEqual(group.defaultOpenThreadId, "thread-main-config")
        XCTAssertEqual(group.endpointCount, 1)
        XCTAssertEqual(group.boundEndpointCount, 1)
        XCTAssertEqual(group.iconDataUrl, iconDataUrl)
        XCTAssertEqual(
            GaryxMobileBotGroupBuilder.selectedGroup(threadId: "thread-alpha", groups: groups)?.id,
            "console-test-bot"
        )
        XCTAssertEqual(
            GaryxMobileBotGroupBuilder.selectedGroup(threadId: "thread-main-config", groups: groups)?.id,
            "console-test-bot"
        )
    }

    func testBuilderUsesConfiguredBotWhenNoConsoleExists() throws {
        let endpoint = try makeEndpoint(
            key: "telegram:test-bot:alpha",
            label: "Alpha",
            threadId: "thread-alpha"
        )
        let configured = try makeConfiguredBot(
            channel: "telegram",
            accountId: "test-bot",
            displayName: "Configured Bot",
            enabled: false,
            agentId: "agent-config",
            workspaceDir: "/workspace/config",
            mainThreadId: "thread-main",
            defaultOpenThreadId: "thread-default"
        )

        let groups = GaryxMobileBotGroupBuilder.groups(
            channelEndpoints: [endpoint],
            configuredBots: [configured],
            botConsoles: [],
            channelPlugins: []
        )

        let group = try XCTUnwrap(groups.first)
        XCTAssertEqual(group.id, "telegram::test-bot")
        XCTAssertEqual(group.title, "Configured Bot")
        XCTAssertEqual(group.subtitle, "Telegram Bot · test-bot")
        XCTAssertEqual(group.agentId, "agent-config")
        XCTAssertEqual(group.status, "disabled")
        XCTAssertEqual(group.defaultOpenThreadId, "thread-default")
    }

    func testBuilderPrefersConsoleValuesOverConfiguredFallbacks() throws {
        let configured = try makeConfiguredBot(
            channel: "telegram",
            accountId: "test-bot",
            displayName: "Configured Bot",
            agentId: "agent-config",
            workspaceDir: "/workspace/config",
            mainThreadId: "thread-main-config",
            defaultOpenThreadId: "thread-default-config"
        )
        let console = try makeConsole(
            id: "console-test-bot",
            channel: "telegram",
            accountId: "test-bot",
            title: "Console Bot",
            subtitle: "Console subtitle",
            agentId: "agent-console",
            workspaceDir: "/workspace/console",
            mainThreadId: "thread-main-console",
            defaultOpenThreadId: "thread-default-console"
        )

        let group = try XCTUnwrap(
            GaryxMobileBotGroupBuilder.groups(
                channelEndpoints: [],
                configuredBots: [configured],
                botConsoles: [console],
                channelPlugins: []
            )
            .first
        )

        XCTAssertEqual(group.agentId, "agent-console")
        XCTAssertEqual(group.workspaceDir, "/workspace/console")
        XCTAssertEqual(group.mainThreadId, "thread-main-console")
        XCTAssertEqual(group.defaultOpenThreadId, "thread-default-console")
    }

    func testBuilderFallsBackToEndpointOnlyGroupsWhenNoConfiguredBotsExist() throws {
        let endpoint = try makeEndpoint(
            key: "custom_channel:test-bot:alpha",
            channel: "custom_channel",
            accountId: "test-bot",
            label: "Alpha",
            threadId: "thread-alpha"
        )

        let groups = GaryxMobileBotGroupBuilder.groups(
            channelEndpoints: [endpoint],
            configuredBots: [],
            botConsoles: [],
            channelPlugins: []
        )

        let group = try XCTUnwrap(groups.first)
        XCTAssertEqual(group.id, "custom_channel::test-bot")
        XCTAssertEqual(group.title, "Custom Channel / test-bot")
        XCTAssertEqual(group.subtitle, "Custom Channel Bot · test-bot")
        XCTAssertEqual(group.defaultOpenThreadId, "thread-alpha")
    }

    func testSelectedGroupRejectsEmptyThreadIdsAndMatchesDefaultThread() {
        let group = makeGroup(
            rootBehavior: "expand_only",
            defaultOpenThreadId: "thread-default"
        )
        let groups = [group]

        XCTAssertNil(GaryxMobileBotGroupBuilder.selectedGroup(threadId: nil, groups: groups))
        XCTAssertNil(GaryxMobileBotGroupBuilder.selectedGroup(threadId: "   ", groups: groups))
        XCTAssertEqual(
            GaryxMobileBotGroupBuilder.selectedGroup(threadId: " thread-default ", groups: groups)?.id,
            group.id
        )
    }

    private func makeGroup(
        channel: String = "telegram",
        accountId: String = "test-bot",
        agentId: String? = nil,
        rootBehavior: String = "expand_only",
        workspaceDir: String? = nil,
        mainThreadId: String? = nil,
        defaultOpenThreadId: String? = nil,
        endpoints: [GaryxChannelEndpoint] = [],
        conversationNodes: [GaryxBotConversationNode] = []
    ) -> GaryxMobileBotGroup {
        GaryxMobileBotGroup(
            id: "\(channel)::\(accountId)",
            channel: channel,
            accountId: accountId,
            title: "Test Bot",
            subtitle: "Synthetic bot",
            agentId: agentId,
            rootBehavior: rootBehavior,
            status: "active",
            endpointCount: endpoints.count,
            boundEndpointCount: endpoints.filter { ($0.threadId ?? "").isEmpty == false }.count,
            workspaceDir: workspaceDir,
            mainThreadId: mainThreadId,
            defaultOpenThreadId: defaultOpenThreadId,
            endpoints: endpoints,
            conversationNodes: conversationNodes,
            iconDataUrl: nil
        )
    }

    private func makeEndpoint(
        key: String,
        channel: String = "telegram",
        accountId: String = "test-bot",
        label: String,
        threadId: String? = nil,
        threadLabel: String? = nil,
        workspaceDir: String? = nil,
        lastInboundAt: String? = nil,
        lastDeliveryAt: String? = nil,
        conversationLabel: String? = nil
    ) throws -> GaryxChannelEndpoint {
        var object: [String: Any] = [
            "endpoint_key": key,
            "channel": channel,
            "account_id": accountId,
            "display_label": label,
        ]
        if let threadId { object["thread_id"] = threadId }
        if let threadLabel { object["thread_label"] = threadLabel }
        if let workspaceDir { object["workspace_dir"] = workspaceDir }
        if let lastInboundAt { object["last_inbound_at"] = lastInboundAt }
        if let lastDeliveryAt { object["last_delivery_at"] = lastDeliveryAt }
        if let conversationLabel { object["conversation_label"] = conversationLabel }
        return try decode(GaryxChannelEndpoint.self, from: object)
    }

    private func makeNode(
        id: String,
        title: String,
        badge: String? = nil,
        latestActivity: String? = nil,
        openable: Bool = true,
        endpoint: GaryxChannelEndpoint
    ) throws -> GaryxBotConversationNode {
        var object: [String: Any] = [
            "id": id,
            "endpoint": endpointJSONObject(endpoint),
            "kind": "conversation",
            "title": title,
            "openable": openable,
        ]
        if let badge { object["badge"] = badge }
        if let latestActivity { object["latest_activity"] = latestActivity }
        return try decode(GaryxBotConversationNode.self, from: object)
    }

    private func endpointJSONObject(_ endpoint: GaryxChannelEndpoint) -> [String: Any] {
        var object: [String: Any] = [
            "endpoint_key": endpoint.endpointKey,
            "channel": endpoint.channel,
            "account_id": endpoint.accountId,
            "display_label": endpoint.displayLabel,
        ]
        if let threadId = endpoint.threadId { object["thread_id"] = threadId }
        if let threadLabel = endpoint.threadLabel { object["thread_label"] = threadLabel }
        if let workspaceDir = endpoint.workspaceDir { object["workspace_dir"] = workspaceDir }
        if let lastInboundAt = endpoint.lastInboundAt { object["last_inbound_at"] = lastInboundAt }
        if let lastDeliveryAt = endpoint.lastDeliveryAt { object["last_delivery_at"] = lastDeliveryAt }
        if let conversationLabel = endpoint.conversationLabel { object["conversation_label"] = conversationLabel }
        return object
    }

    private func makeConfiguredBot(
        channel: String,
        accountId: String,
        displayName: String,
        enabled: Bool = true,
        agentId: String? = nil,
        workspaceDir: String? = nil,
        rootBehavior: String = "open_default",
        mainThreadId: String? = nil,
        defaultOpenThreadId: String? = nil
    ) throws -> GaryxConfiguredBot {
        var object: [String: Any] = [
            "channel": channel,
            "account_id": accountId,
            "display_name": displayName,
            "enabled": enabled,
            "root_behavior": rootBehavior,
            "main_endpoint_status": "bound",
        ]
        if let agentId { object["agent_id"] = agentId }
        if let workspaceDir { object["workspace_dir"] = workspaceDir }
        if let mainThreadId { object["main_endpoint_thread_id"] = mainThreadId }
        if let defaultOpenThreadId { object["default_open_thread_id"] = defaultOpenThreadId }
        return try decode(GaryxConfiguredBot.self, from: object)
    }

    private func makeConsole(
        id: String,
        channel: String,
        accountId: String,
        title: String,
        subtitle: String,
        agentId: String?,
        workspaceDir: String?,
        mainThreadId: String?,
        defaultOpenThreadId: String?
    ) throws -> GaryxBotConsoleSummary {
        var object: [String: Any] = [
            "id": id,
            "channel": channel,
            "account_id": accountId,
            "title": title,
            "subtitle": subtitle,
            "root_behavior": "open_default",
            "status": "idle",
            "endpoint_count": 0,
            "bound_endpoint_count": 0,
            "conversation_nodes": [],
        ]
        if let agentId { object["agent_id"] = agentId }
        if let workspaceDir { object["workspace_dir"] = workspaceDir }
        if let mainThreadId { object["main_endpoint_thread_id"] = mainThreadId }
        if let defaultOpenThreadId { object["default_open_thread_id"] = defaultOpenThreadId }
        return try decode(GaryxBotConsoleSummary.self, from: object)
    }

    private func makePlugin(id: String, iconDataUrl: String) throws -> GaryxChannelPluginCatalogEntry {
        try decode(
            GaryxChannelPluginCatalogEntry.self,
            from: [
                "id": id,
                "display_name": id.capitalized,
                "icon_data_url": iconDataUrl,
                "schema": [:],
                "config_methods": [],
            ]
        )
    }

    private func decode<T: Decodable>(_ type: T.Type, from object: [String: Any]) throws -> T {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try JSONDecoder().decode(T.self, from: data)
    }
}
