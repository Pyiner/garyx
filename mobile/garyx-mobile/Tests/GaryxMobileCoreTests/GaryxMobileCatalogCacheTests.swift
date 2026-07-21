import XCTest
@testable import GaryxMobileCore

final class GaryxMobileCatalogCacheTests: XCTestCase {
    func testSnapshotPreservesNullableAutomationAgentResolutionAndValidation() throws {
        let automation = GaryxAutomationSummary(
            id: "cron::target",
            label: "Target job",
            prompt: "Continue",
            agentId: nil,
            agentResolution: .followThread,
            effectiveAgentId: "codex",
            workspacePath: "",
            targetThreadId: "thread::target",
            validationState: .invalid,
            validationError: "target has no canonical binding"
        )
        let snapshot = GaryxMobileCatalogCacheSnapshot(
            agents: [],
            workspaceCatalog: .empty,
            skills: [],
            automations: [automation],
            slashCommands: [],
            mcpServers: [],
            channelEndpoints: [],
            configuredBots: [],
            configuredBotAccounts: [],
            botConsoles: [],
            channelPlugins: []
        )

        let data = try JSONEncoder().encode(snapshot)
        let decoded = try JSONDecoder().decode(GaryxMobileCatalogCacheSnapshot.self, from: data)
        XCTAssertEqual(decoded.version, 6)
        let restored = try XCTUnwrap(decoded.automations.first?.model)
        XCTAssertNil(restored.agentId)
        XCTAssertEqual(restored.agentResolution, .followThread)
        XCTAssertEqual(restored.effectiveAgentId, "codex")
        XCTAssertEqual(restored.validationState, .invalid)
        XCTAssertEqual(restored.validationError, "target has no canonical binding")
    }

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
            gatewayDefaultAgentId: "disabled-agent",
            effectiveDefaultAgentId: "agent-alpha",
            workspaceCatalog: GaryxWorkspaceCatalog(
                gatewayHome: "/Users/test",
                workspaces: [
                    GaryxWorkspaceSummary(
                        name: "project-alpha",
                        path: "/Users/test/project-alpha",
                        pinned: true,
                        threadCount: 4,
                        lastActivityAt: "2026-07-20T16:44:00Z",
                        gitRepo: true
                    )
                ]
            ),
            skills: [],
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
        XCTAssertEqual(decoded.workspaceCatalog.gatewayHome, "/Users/test")
        XCTAssertEqual(decoded.workspaceCatalog.paths, ["/Users/test/project-alpha"])
        let restoredWorkspace = try XCTUnwrap(decoded.workspaceCatalog.workspaces.first)
        XCTAssertEqual(restoredWorkspace.name, "project-alpha")
        XCTAssertTrue(restoredWorkspace.pinned)
        XCTAssertEqual(restoredWorkspace.threadCount, 4)
        XCTAssertEqual(restoredWorkspace.lastActivityAt, "2026-07-20T16:44:00Z")
        XCTAssertTrue(restoredWorkspace.gitRepo)
        XCTAssertEqual(decoded.gatewayDefaultAgentId, "disabled-agent")
        XCTAssertEqual(decoded.effectiveDefaultAgentId, "agent-alpha")
        let restoredAgent = try XCTUnwrap(decoded.agents.first?.model)
        XCTAssertEqual(restoredAgent.id, "agent-alpha")
        XCTAssertEqual(restoredAgent.displayName, "Agent Alpha")
        XCTAssertEqual(restoredAgent.providerType, "codex")
        XCTAssertEqual(restoredAgent.model, "gpt-test")
        XCTAssertTrue(restoredAgent.enabled)
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
            agentId: nil,
            effectiveAgentId: "agent-alpha",
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
            workspaceCatalog: .empty,
            skills: [],
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
        XCTAssertNil(restoredConsole.agentId)
        XCTAssertEqual(restoredConsole.effectiveAgentId, "agent-alpha")
        XCTAssertEqual(restoredConsole.conversationNodes, [])
    }

    func testSnapshotPreservesConfiguredBotWorkspaceModeAndAccountConfig() throws {
        let bot = GaryxConfiguredBot(
            channel: "telegram",
            accountId: "bot-main",
            displayName: "Telegram Main",
            agentId: "agent-alpha",
            effectiveAgentId: "agent-alpha",
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
            workspaceCatalog: .empty,
            skills: [],
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
        XCTAssertEqual(restoredBot.effectiveAgentId, "agent-alpha")
        let restoredAccount = try XCTUnwrap(decoded.configuredBotAccounts.first?.model)
        XCTAssertEqual(restoredAccount.config, ["token": .string("${TOKEN}")])
        XCTAssertEqual(restoredAccount.workspaceMode, "worktree")
    }

    func testSnapshotPreservesCapsuleMetadataWithoutHTMLBody() throws {
        let capsule = GaryxCapsuleSummary(
            id: "01900000-0000-7000-8000-000000000001",
            title: "Synthetic Capsule",
            description: "A safe synthetic HTML demo.",
            threadId: "thread::capsule",
            runId: "run-capsule",
            agentId: "codex",
            providerType: "codex_app_server",
            htmlSha256: "abc123",
            byteSize: 42,
            revision: 3,
            createdAt: "2026-06-28T10:00:00Z",
            updatedAt: "2026-06-28T11:00:00Z",
            favoritedAt: "2026-06-28T11:30:00Z"
        )
        let snapshot = GaryxMobileCatalogCacheSnapshot(
            agents: [],
            workspaceCatalog: .empty,
            skills: [],
            capsules: [capsule],
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
        XCTAssertTrue(encoded.contains("Synthetic Capsule"))
        XCTAssertFalse(encoded.contains("<html"))

        let decoded = try JSONDecoder().decode(GaryxMobileCatalogCacheSnapshot.self, from: data)
        XCTAssertEqual(decoded.version, GaryxMobileCatalogCacheSnapshot.currentVersion)
        let restored = try XCTUnwrap(decoded.capsules.first?.model)
        XCTAssertEqual(restored, capsule)
        XCTAssertEqual(restored.favoritedAt, "2026-06-28T11:30:00Z")
    }

    func testLegacyCachedAgentDefaultsEnabledButOldSnapshotVersionIsDiscarded() throws {
        let legacyAgent = try JSONDecoder().decode(
            GaryxCachedAgent.self,
            from: Data(
                #"{"id":"agent-test","displayName":"Test Agent","providerType":"codex","modelName":"","defaultWorkspaceDir":"","avatarDataUrl":"","builtIn":false,"standalone":true}"#.utf8
            )
        )
        XCTAssertTrue(legacyAgent.enabled)
        XCTAssertFalse(GaryxMobileCatalogCachePolicy.shouldRestore(version: 4))
        XCTAssertTrue(
            GaryxMobileCatalogCachePolicy.shouldRestore(
                version: GaryxMobileCatalogCacheSnapshot.currentVersion
            )
        )
    }

}
