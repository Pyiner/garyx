import XCTest
@testable import GaryxMobileCore

final class GaryxMobileGatewaySettingsModelsTests: XCTestCase {
    func testGatewaySetupShowsDetailsWhileCheckingExistingGateway() {
        XCTAssertTrue(
            GaryxGatewaySetupPresentation.showsDetails(
                isSheet: false,
                startsEmpty: false,
                hasGatewaySettings: true,
                phase: .checking
            )
        )
    }

    func testGatewaySetupShowsDetailsAfterExistingGatewayFailure() {
        XCTAssertTrue(
            GaryxGatewaySetupPresentation.showsDetails(
                isSheet: false,
                startsEmpty: false,
                hasGatewaySettings: true,
                phase: .failed
            )
        )
    }

    func testGatewaySetupCanHideDetailsWhenExistingGatewayIsReady() {
        XCTAssertFalse(
            GaryxGatewaySetupPresentation.showsDetails(
                isSheet: false,
                startsEmpty: false,
                hasGatewaySettings: true,
                phase: .ready
            )
        )
    }

    func testGatewayProfileStorageNormalizesDedupesAndLabelsProfiles() {
        let older = makeProfile(
            id: "old",
            label: "",
            gatewayUrl: " http://127.0.0.1:31337/ ",
            updatedAt: Date(timeIntervalSince1970: 100)
        )
        let newer = makeProfile(
            id: "new",
            label: "Local Gateway",
            gatewayUrl: "http://127.0.0.1:31337",
            updatedAt: Date(timeIntervalSince1970: 200)
        )
        let remote = makeProfile(
            id: "remote",
            label: "",
            gatewayUrl: "https://gateway.example.test/",
            updatedAt: Date(timeIntervalSince1970: 150)
        )

        let profiles = GaryxGatewayProfileStorage.normalizedProfiles([older, remote, newer])

        XCTAssertEqual(profiles.map(\.gatewayUrl), ["http://127.0.0.1:31337", "https://gateway.example.test"])
        XCTAssertEqual(profiles.map(\.label), ["Local Gateway", "gateway.example.test"])
        XCTAssertEqual(profiles[0].id, GaryxGatewayProfileStorage.stableId(for: "http://127.0.0.1:31337"))
        XCTAssertEqual(profiles[1].id, GaryxGatewayProfileStorage.stableId(for: "https://gateway.example.test"))
    }

    func testGatewayProfileStorageLoadsIso8601ProfilesFromDefaults() throws {
        let key = "garyx.mobile.gatewayProfiles.test"
        let suiteName = "GaryxMobileGatewaySettingsModelsTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode([
            makeProfile(
                id: "raw",
                label: "",
                gatewayUrl: "https://gateway.example.test/",
                updatedAt: Date(timeIntervalSince1970: 100)
            ),
        ])
        defaults.set(data, forKey: key)

        let profiles = GaryxGatewayProfileStorage.load(defaults: defaults, key: key)

        XCTAssertEqual(profiles.count, 1)
        XCTAssertEqual(profiles.first?.gatewayUrl, "https://gateway.example.test")
        XCTAssertEqual(profiles.first?.label, "gateway.example.test")
    }

    func testGatewayProfileStorageKeepsOnlyEightRecentProfiles() {
        let profiles = (0..<10).map { index in
            makeProfile(
                id: "profile-\(index)",
                label: "Gateway \(index)",
                gatewayUrl: "https://gateway-\(index).example.test",
                updatedAt: Date(timeIntervalSince1970: TimeInterval(index))
            )
        }

        let normalized = GaryxGatewayProfileStorage.normalizedProfiles(profiles)

        XCTAssertEqual(normalized.count, 8)
        XCTAssertEqual(normalized.first?.gatewayUrl, "https://gateway-9.example.test")
        XCTAssertEqual(normalized.last?.gatewayUrl, "https://gateway-2.example.test")
    }

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
        XCTAssertFalse(accounts.contains { $0.channel == "api" })
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

    func testSettingsDocumentRoundTripsConfiguredBotAccounts() {
        let account = GaryxConfiguredBotAccountSettings(
            channel: "telegram",
            accountId: "bot-alpha",
            displayName: "Alpha",
            enabled: true,
            agentId: "agent-alpha",
            workspaceDir: "/workspace/alpha",
            workspaceMode: "local",
            config: ["token": .string("${TOKEN}")]
        )

        let settings = GaryxConfiguredBotAccountsDocument.settingsDocument(from: [account])
        let accounts = GaryxConfiguredBotAccountsDocument.accounts(from: settings)

        XCTAssertEqual(accounts, [account])
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
        XCTAssertFalse(
            GaryxConfiguredBotAccountsDocument.removeAccount(
                from: &settings,
                channel: "telegram",
                accountId: "bot-alpha"
            )
        )
    }

    private func makeProfile(
        id: String,
        label: String,
        gatewayUrl: String,
        updatedAt: Date
    ) -> GaryxGatewayProfile {
        GaryxGatewayProfile(
            id: id,
            label: label,
            gatewayUrl: gatewayUrl,
            updatedAt: updatedAt,
            hasToken: true
        )
    }
}
