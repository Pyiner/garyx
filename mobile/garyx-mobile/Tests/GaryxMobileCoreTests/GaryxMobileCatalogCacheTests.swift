import XCTest
@testable import GaryxMobileCore

final class GaryxMobileCatalogCacheTests: XCTestCase {
    func testSnapshotRestoresCatalogFieldsWithoutHiddenAgentConfiguration() throws {
        let agent = GaryxAgentSummary(
            id: "agent-alpha",
            displayName: "Agent Alpha",
            providerType: "codex",
            model: "gpt-test",
            modelReasoningEffort: "high",
            providerEnv: ["SYNTHETIC_SECRET": "synthetic-hidden-env"],
            defaultWorkspaceDir: "/Users/test/project-alpha",
            avatarDataUrl: "data:image/png;base64,AAAA",
            systemPrompt: "synthetic hidden prompt"
        )
        let snapshot = GaryxMobileCatalogCacheSnapshot(
            agents: [agent],
            teams: [],
            workspacePaths: ["/Users/test/project-alpha"],
            skills: [],
            tasks: [],
            automations: [],
            slashCommands: [],
            mcpServers: [],
            channelEndpoints: [],
            configuredBots: [],
            configuredBotAccounts: [],
            botConsoles: [],
            channelPlugins: [],
            savedAt: Date(timeIntervalSince1970: 1)
        )

        let data = try JSONEncoder().encode(snapshot)
        let encoded = String(decoding: data, as: UTF8.self)
        XCTAssertFalse(encoded.contains("synthetic-hidden-env"))
        XCTAssertFalse(encoded.contains("synthetic hidden prompt"))

        let decoded = try JSONDecoder().decode(GaryxMobileCatalogCacheSnapshot.self, from: data)
        XCTAssertEqual(decoded.version, GaryxMobileCatalogCacheSnapshot.currentVersion)
        XCTAssertEqual(decoded.workspacePaths, ["/Users/test/project-alpha"])
        let restoredAgent = try XCTUnwrap(decoded.agents.first?.model)
        XCTAssertEqual(restoredAgent.id, "agent-alpha")
        XCTAssertEqual(restoredAgent.displayName, "Agent Alpha")
        XCTAssertEqual(restoredAgent.providerType, "codex")
        XCTAssertEqual(restoredAgent.model, "gpt-test")
        XCTAssertEqual(restoredAgent.providerEnv, [:])
        XCTAssertEqual(restoredAgent.systemPrompt, "")
    }

    func testSnapshotDropsVolatileMcpAndBotConversationDetails() throws {
        let mcpServer = GaryxMcpServer(
            name: "filesystem",
            command: "node",
            args: ["server.js"],
            env: ["SYNTHETIC_SECRET": "synthetic-mcp-env"],
            workingDir: "/Users/test/project-alpha",
            bearerTokenEnv: "SYNTHETIC_BEARER_ENV",
            headers: ["Authorization": "Bearer synthetic-token"]
        )
        let endpoint = GaryxChannelEndpoint(
            endpointKey: "telegram:bot:chat",
            channel: "telegram",
            accountId: "bot",
            displayLabel: "Chat",
            threadId: "thread-old"
        )
        let console = GaryxBotConsoleSummary(
            id: "telegram:bot",
            channel: "telegram",
            accountId: "bot",
            title: "Telegram Bot",
            conversationNodes: [
                GaryxBotConversationNode(
                    id: "old-node",
                    endpoint: endpoint,
                    kind: "conversation",
                    title: "Old Conversation",
                    openable: true
                ),
            ]
        )
        let snapshot = GaryxMobileCatalogCacheSnapshot(
            agents: [],
            teams: [],
            workspacePaths: [],
            skills: [],
            tasks: [],
            automations: [],
            slashCommands: [],
            mcpServers: [mcpServer],
            channelEndpoints: [endpoint],
            configuredBots: [],
            configuredBotAccounts: [],
            botConsoles: [console],
            channelPlugins: [],
            savedAt: Date(timeIntervalSince1970: 1)
        )

        let data = try JSONEncoder().encode(snapshot)
        let encoded = String(decoding: data, as: UTF8.self)
        XCTAssertFalse(encoded.contains("synthetic-mcp-env"))
        XCTAssertFalse(encoded.contains("Authorization"))
        XCTAssertFalse(encoded.contains("old-node"))

        let decoded = try JSONDecoder().decode(GaryxMobileCatalogCacheSnapshot.self, from: data)
        let restoredServer = try XCTUnwrap(decoded.mcpServers.first?.model)
        XCTAssertEqual(restoredServer.name, "filesystem")
        XCTAssertEqual(restoredServer.command, "node")
        XCTAssertEqual(restoredServer.env, [:])
        XCTAssertEqual(restoredServer.bearerTokenEnv, "SYNTHETIC_BEARER_ENV")
        XCTAssertEqual(restoredServer.headers, [:])
        let restoredConsole = try XCTUnwrap(decoded.botConsoles.first?.model)
        XCTAssertEqual(restoredConsole.title, "Telegram Bot")
        XCTAssertEqual(restoredConsole.conversationNodes, [])
    }

    func testSnapshotPreservesConfiguredBotWorkspaceModeAndAccountConfig() throws {
        let bot = GaryxConfiguredBot(
            channel: "telegram",
            accountId: "bot-main",
            displayName: "Telegram Main",
            agentId: "agent-alpha",
            workspaceDir: "/Users/test/project-alpha",
            workspaceMode: "worktree"
        )
        let account = GaryxConfiguredBotAccountSettings(
            channel: "telegram",
            accountId: "bot-main",
            displayName: "Telegram Main",
            enabled: true,
            agentId: "agent-alpha",
            workspaceDir: "/Users/test/project-alpha",
            workspaceMode: "worktree",
            config: ["token": .string("${TOKEN}")]
        )
        let snapshot = GaryxMobileCatalogCacheSnapshot(
            agents: [],
            teams: [],
            workspacePaths: [],
            skills: [],
            tasks: [],
            automations: [],
            slashCommands: [],
            mcpServers: [],
            channelEndpoints: [],
            configuredBots: [bot],
            configuredBotAccounts: [account],
            botConsoles: [],
            channelPlugins: [],
            savedAt: Date(timeIntervalSince1970: 1)
        )

        let data = try JSONEncoder().encode(snapshot)
        let decoded = try JSONDecoder().decode(GaryxMobileCatalogCacheSnapshot.self, from: data)
        let restoredBot = try XCTUnwrap(decoded.configuredBots.first?.model)
        XCTAssertEqual(restoredBot.workspaceMode, "worktree")
        XCTAssertEqual(restoredBot.workspaceDir, "/Users/test/project-alpha")
        let restoredAccount = try XCTUnwrap(decoded.configuredBotAccounts.first?.model)
        XCTAssertEqual(restoredAccount.config, ["token": .string("${TOKEN}")])
        XCTAssertEqual(restoredAccount.workspaceMode, "worktree")
    }
}
