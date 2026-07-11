import XCTest
@testable import GaryxMobileCore

final class GaryxProviderSettingsPresentationTests: XCTestCase {
    private func provider(_ type: String) throws -> GaryxModelProviderDefault {
        try XCTUnwrap(GaryxModelProviderDefaults.provider(for: type))
    }

    private func catalog(_ json: String) throws -> GaryxProviderModels {
        try JSONDecoder().decode(GaryxProviderModels.self, from: Data(json.utf8))
    }

    func testAuthSectionPerProviderType() {
        let expected: [String: GaryxProviderSettingsPresentation.AuthSection] = [
            "claude_code": .claudeCode,
            "codex_app_server": .managedOAuth,
            "antigravity": .managedOAuth,
            "traex": .managedOAuth,
        ]
        XCTAssertEqual(Set(GaryxModelProviderDefaults.providers.map(\.providerType)), Set(expected.keys))
        for provider in GaryxModelProviderDefaults.providers {
            XCTAssertEqual(
                GaryxProviderSettingsPresentation.authSection(for: provider),
                expected[provider.providerType]
            )
        }
    }

    func testServiceTierFollowsCatalogCapability() throws {
        let codex = try provider("codex_app_server")
        XCTAssertTrue(
            GaryxProviderSettingsPresentation.supportsServiceTier(
                provider: codex,
                catalog: try catalog(#"{"supports_service_tier_selection": true}"#)
            )
        )
        XCTAssertFalse(
            GaryxProviderSettingsPresentation.supportsServiceTier(
                provider: codex,
                catalog: try catalog(#"{"supports_service_tier_selection": false}"#)
            )
        )
        XCTAssertFalse(GaryxProviderSettingsPresentation.supportsServiceTier(provider: codex, catalog: nil))
    }

    func testDefaultModelLabelUsesCatalogThenStaticFallback() throws {
        let antigravity = try provider("antigravity")
        let claude = try provider("claude_code")
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(
                provider: antigravity,
                catalog: try catalog(#"{"default_model": "catalog-model"}"#)
            ),
            "Provider default: catalog-model"
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(provider: antigravity, catalog: nil),
            "Provider default: Claude Opus 4.6 (Thinking)"
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(provider: claude, catalog: nil),
            "Provider default"
        )
    }

    func testDraftEchoesConfiguredDefaults() throws {
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "codex": .object([
                    "default_model": .string("codex-model"),
                    "model_reasoning_effort": .string("high"),
                    "model_service_tier": .string("priority"),
                ]),
            ]),
        ]
        let draft = GaryxProviderSettingsPresentation.Draft.make(
            settings: settings,
            provider: try provider("codex_app_server")
        )
        XCTAssertEqual(
            draft,
            GaryxProviderSettingsPresentation.Draft(
                modelName: "codex-model",
                reasoningEffort: "high",
                serviceTier: "priority"
            )
        )
    }

    func testSaveRequestIncludesOnlyCatalogSupportedTier() throws {
        let codex = try provider("codex_app_server")
        let supporting = GaryxProviderSettingsPresentation.SaveRequest.make(
            provider: codex,
            catalog: try catalog(#"{"supports_service_tier_selection": true}"#),
            modelName: "codex-model",
            reasoningEffort: "high",
            serviceTier: "priority"
        )
        XCTAssertEqual(supporting.serviceTier, "priority")

        let unsupported = GaryxProviderSettingsPresentation.SaveRequest.make(
            provider: codex,
            catalog: nil,
            modelName: "codex-model",
            reasoningEffort: "high",
            serviceTier: "priority"
        )
        XCTAssertNil(unsupported.serviceTier)
    }

    func testRowModelStatusAndFallbackText() throws {
        let claude = try provider("claude_code")
        let error = GaryxProviderSettingsPresentation.RowModel.make(
            provider: claude,
            catalog: try catalog(#"{"error": "boom"}"#),
            settings: [:]
        )
        XCTAssertEqual(error.statusText, "Error")
        XCTAssertEqual(error.statusTone, .danger)
        XCTAssertEqual(error.detailText, "Model metadata unavailable")

        let loading = GaryxProviderSettingsPresentation.RowModel.make(
            provider: claude,
            catalog: nil,
            settings: [:]
        )
        XCTAssertEqual(loading.statusText, "Loading")
        XCTAssertEqual(loading.statusTone, .muted)
        XCTAssertEqual(loading.detailText, "Loading metadata")
    }

    func testRowModelUsesConfiguredModelAndCapabilityCounts() throws {
        let codex = try provider("codex_app_server")
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "codex": .object([
                    "default_model": .string("configured-model"),
                    "model_reasoning_effort": .string("high"),
                ]),
            ]),
        ]
        let models = try catalog(#"""
        {
            "default_model": "catalog-model",
            "supports_model_selection": true,
            "models": [{"id": "a"}, {"id": "b"}],
            "supports_reasoning_effort_selection": true,
            "reasoning_efforts": [{"id": "low"}, {"id": "high"}],
            "supports_service_tier_selection": true,
            "service_tiers": [{"id": "standard"}, {"id": "priority"}]
        }
        """#)
        let row = GaryxProviderSettingsPresentation.RowModel.make(
            provider: codex,
            catalog: models,
            settings: settings
        )
        XCTAssertEqual(row.statusText, "Ready")
        XCTAssertEqual(row.statusTone, .good)
        XCTAssertEqual(
            row.detailText,
            "Default configured-model · Thinking high · 2 models · 2 reasoning · 2 tiers"
        )
    }
}
