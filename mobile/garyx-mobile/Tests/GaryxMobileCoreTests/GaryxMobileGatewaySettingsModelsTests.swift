import XCTest
@testable import GaryxMobileCore

final class GaryxMobileGatewaySettingsModelsTests: XCTestCase {
    func testConfiguredBotAccountsDecodeAndSortGatewaySettingsDocument() {
        let settings: [String: GaryxJSONValue] = [
            "channels": .object([
                "api": .object([
                    "accounts": .object([
                        "ignored": .object(["name": .string("Ignored")]),
                    ]),
                ]),
                "telegram": .object([
                    "accounts": .object([
                        "bot-beta": .object([
                            "enabled": .string("false"),
                            "agentId": .string("agent-beta"),
                            "workspaceDir": .string("/workspace/beta"),
                            "workspaceMode": .string("worktree"),
                            "config": .object(["token": .string("${TOKEN}")]),
                        ]),
                    ]),
                ]),
                "discord": .object([
                    "accounts": .object([
                        "bot-alpha": .object([
                            "name": .string("Alpha Bot"),
                            "enabled": .bool(true),
                            "agent_id": .string("agent-alpha"),
                            "workspace_dir": .string("/workspace/alpha"),
                            "workspace_mode": .string("local"),
                            "config": .object(["guild": .string("1000000001")]),
                        ]),
                    ]),
                ]),
            ]),
        ]

        let accounts = GaryxConfiguredBotAccountsDocument.accounts(from: settings)

        XCTAssertEqual(accounts.map(\.id), ["discord:bot-alpha", "telegram:bot-beta"])
        XCTAssertEqual(accounts[0].displayName, "Alpha Bot")
        XCTAssertTrue(accounts[0].enabled)
        XCTAssertEqual(accounts[0].agentId, "agent-alpha")
        XCTAssertEqual(accounts[0].workspaceDir, "/workspace/alpha")
        XCTAssertEqual(accounts[0].workspaceMode, "local")
        XCTAssertEqual(accounts[0].config, ["guild": .string("1000000001")])
        XCTAssertEqual(accounts[1].displayName, "bot-beta")
        XCTAssertFalse(accounts[1].enabled)
        XCTAssertEqual(accounts[1].agentId, "agent-beta")
        XCTAssertEqual(accounts[1].config, ["token": .string("${TOKEN}")])
    }

    func testSetAccountTrimsValuesAndRenamesOriginalAccount() {
        var settings: [String: GaryxJSONValue] = [
            "channels": .object([
                "Telegram": .object([
                    "accounts": .object([
                        "old-bot": .object(["name": .string("Old Bot")]),
                    ]),
                ]),
            ]),
        ]
        let input = GaryxConfiguredBotAccountInput(
            channel: " telegram ",
            accountId: " new-bot ",
            displayName: " New Bot ",
            enabled: false,
            agentId: " agent-alpha ",
            workspaceDir: " /workspace/alpha ",
            workspaceMode: " ",
            config: ["token": .string("${TOKEN}")]
        )

        let changed = GaryxConfiguredBotAccountsDocument.setAccount(
            in: &settings,
            originalChannel: "Telegram",
            originalAccountId: "old-bot",
            input: input
        )
        let accounts = GaryxConfiguredBotAccountsDocument.accounts(from: settings)

        XCTAssertTrue(changed)
        XCTAssertEqual(accounts.map(\.id), ["telegram:new-bot"])
        XCTAssertEqual(accounts.first?.displayName, "New Bot")
        XCTAssertEqual(accounts.first?.enabled, false)
        XCTAssertEqual(accounts.first?.agentId, "agent-alpha")
        XCTAssertEqual(accounts.first?.workspaceDir, "/workspace/alpha")
        XCTAssertNil(accounts.first?.workspaceMode)
        XCTAssertEqual(accounts.first?.config, ["token": .string("${TOKEN}")])
    }

    func testRemoveAccountMatchesChannelCaseInsensitively() {
        var settings: [String: GaryxJSONValue] = [
            "channels": .object([
                "Discord": .object([
                    "accounts": .object([
                        "bot-alpha": .object(["name": .string("Alpha")]),
                    ]),
                ]),
            ]),
        ]

        XCTAssertTrue(
            GaryxConfiguredBotAccountsDocument.removeAccount(
                from: &settings,
                channel: "discord",
                accountId: "bot-alpha"
            )
        )
        XCTAssertTrue(GaryxConfiguredBotAccountsDocument.accounts(from: settings).isEmpty)
        XCTAssertFalse(
            GaryxConfiguredBotAccountsDocument.removeAccount(
                from: &settings,
                channel: "discord",
                accountId: "bot-alpha"
            )
        )
    }
}
