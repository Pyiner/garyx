import XCTest
@testable import GaryxMobileCore

final class GaryxModelProviderDefaultsTests: XCTestCase {
    func testProviderTableContainsOnlyExternalRuntimes() {
        XCTAssertEqual(
            GaryxModelProviderDefaults.providers.map(\.providerType),
            ["claude_code", "codex_app_server", "antigravity", "traex"]
        )
        XCTAssertEqual(GaryxModelProviderDefaults.provider(for: "claude_code")?.configKey, "claude")
        XCTAssertEqual(GaryxModelProviderDefaults.provider(for: "codex_app_server")?.configKey, "codex")
        XCTAssertEqual(GaryxModelProviderDefaults.provider(for: "antigravity")?.configKey, "antigravity")
        XCTAssertEqual(GaryxModelProviderDefaults.provider(for: "traex")?.configKey, "traex")
        XCTAssertNil(GaryxModelProviderDefaults.provider(for: "unknown"))
    }

    func testProviderUsageIdsAreExplicit() {
        XCTAssertEqual(GaryxModelProviderDefaults.provider(for: "claude_code")?.usageProviderId, "claude_code")
        XCTAssertEqual(GaryxModelProviderDefaults.provider(for: "codex_app_server")?.usageProviderId, "codex")
        XCTAssertEqual(GaryxModelProviderDefaults.provider(for: "antigravity")?.usageProviderId, "antigravity")
        XCTAssertNil(GaryxModelProviderDefaults.provider(for: "traex")?.usageProviderId)
    }

    func testUpdateWritesDefaultsAndPreservesExistingRuntimeConfig() throws {
        let provider = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "codex_app_server"))
        var settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "codex": .object([
                    "codex_home": .string("/Users/test/.codex"),
                    "env": .object(["TEST_TOKEN": .string("${TOKEN}")]),
                ]),
            ]),
        ]
        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: " codex-model ",
            reasoningEffort: " high ",
            serviceTier: " priority "
        )

        let config = GaryxModelProviderDefaults.providerConfig(in: settings, provider: provider)
        XCTAssertEqual(config["provider_type"], .string("codex_app_server"))
        XCTAssertEqual(config["default_model"], .string("codex-model"))
        XCTAssertEqual(config["model_reasoning_effort"], .string("high"))
        XCTAssertEqual(config["model_service_tier"], .string("priority"))
        XCTAssertEqual(config["codex_home"], .string("/Users/test/.codex"))
        XCTAssertEqual(config["env"], .object(["TEST_TOKEN": .string("${TOKEN}")]))
    }

    func testUpdateOmitsServiceTierWhenNotRequested() throws {
        let provider = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "claude_code"))
        var settings: [String: GaryxJSONValue] = [:]
        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: "opus",
            reasoningEffort: "max"
        )
        XCTAssertEqual(GaryxModelProviderDefaults.configuredDefaultModel(in: settings, provider: provider), "opus")
        XCTAssertEqual(GaryxModelProviderDefaults.configuredReasoningEffort(in: settings, provider: provider), "max")
        XCTAssertEqual(GaryxModelProviderDefaults.configuredServiceTier(in: settings, provider: provider), "")
    }

    func testHostRuntimeFieldsExposeApplicableFieldsAndEnvironment() throws {
        let claude = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "claude_code"))
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "claude": .object([
                    "claude_cli_mode": .string("cctty"),
                    "claude_cli_path": .string("/Users/test/.local/bin/claude"),
                    "permission_mode": .string(""),
                    "env": .object([
                        "B_VAR": .string("b"),
                        "A_VAR": .string("a"),
                    ]),
                ]),
            ]),
        ]
        let fields = GaryxModelProviderDefaults.hostRuntimeFields(in: settings, provider: claude)
        XCTAssertEqual(fields.map(\.label), ["CLI mode", "CLI path", "Environment"])
        XCTAssertEqual(fields[0].value, "cctty")
        XCTAssertEqual(fields[1].value, "/Users/test/.local/bin/claude")
        XCTAssertEqual(fields[2].value, "A_VAR=a\nB_VAR=b")
    }

    func testHostRuntimeFieldsCoverCodexAndAntigravity() throws {
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "codex": .object(["codex_home": .string("/Users/test/.codex")]),
                "antigravity": .object(["antigravity_bin": .string("/usr/local/bin/antigravity")]),
            ]),
        ]
        let codex = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "codex_app_server"))
        let antigravity = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "antigravity"))
        XCTAssertEqual(
            GaryxModelProviderDefaults.hostRuntimeFields(in: settings, provider: codex),
            [GaryxProviderHostField(label: "Codex home", value: "/Users/test/.codex")]
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.hostRuntimeFields(in: settings, provider: antigravity),
            [GaryxProviderHostField(label: "Antigravity binary", value: "/usr/local/bin/antigravity")]
        )
    }
}
