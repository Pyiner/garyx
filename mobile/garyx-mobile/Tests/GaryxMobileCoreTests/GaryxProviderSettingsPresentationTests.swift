import XCTest
@testable import GaryxMobileCore

/// Characterization tests for the provider-defaults sheet planner
/// (TASK-1753): expectations mirror the behavior previously inlined in
/// `GaryxMobileProviderSettingsViews.swift` so the extraction is provably
/// behavior-preserving for every provider type.
final class GaryxProviderSettingsPresentationTests: XCTestCase {
    private func provider(_ type: String) throws -> GaryxModelProviderDefault {
        try XCTUnwrap(GaryxModelProviderDefaults.provider(for: type))
    }

    private func catalog(_ json: String) throws -> GaryxProviderModels {
        try JSONDecoder().decode(GaryxProviderModels.self, from: Data(json.utf8))
    }

    // MARK: Section planning

    func testAuthSectionPerProviderType() throws {
        let expected: [String: GaryxProviderSettingsPresentation.AuthSection] = [
            "claude_code": .claudeCode,
            "codex_app_server": .managedOAuth,
            "antigravity": .managedOAuth,
            "traex": .managedOAuth,
            "gemini_cli": .managedOAuth,
            "gpt": .native,
            "anthropic": .native,
            "google": .native,
        ]
        XCTAssertEqual(
            Set(GaryxModelProviderDefaults.providers.map(\.providerType)),
            Set(expected.keys)
        )
        for provider in GaryxModelProviderDefaults.providers {
            XCTAssertEqual(
                GaryxProviderSettingsPresentation.authSection(for: provider),
                expected[provider.providerType],
                "authSection mismatch for \(provider.providerType)"
            )
        }
    }

    func testOffersGptTokenAuthSourceOnlyForGpt() {
        for provider in GaryxModelProviderDefaults.providers {
            XCTAssertEqual(
                GaryxProviderSettingsPresentation.offersGptTokenAuthSource(provider),
                provider.providerType == "gpt",
                "gpt-token option mismatch for \(provider.providerType)"
            )
        }
    }

    func testSupportsServiceTier() throws {
        let gpt = try provider("gpt")
        let anthropic = try provider("anthropic")
        let supporting = try catalog(#"{"supports_service_tier_selection": true}"#)
        let unsupporting = try catalog(#"{"supports_service_tier_selection": false}"#)

        XCTAssertTrue(GaryxProviderSettingsPresentation.supportsServiceTier(provider: gpt, catalog: supporting))
        XCTAssertFalse(GaryxProviderSettingsPresentation.supportsServiceTier(provider: gpt, catalog: unsupporting))
        XCTAssertFalse(GaryxProviderSettingsPresentation.supportsServiceTier(provider: gpt, catalog: nil))
        XCTAssertFalse(GaryxProviderSettingsPresentation.supportsServiceTier(provider: anthropic, catalog: supporting))
    }

    // MARK: Field-level rules

    func testEffectiveAuthSourceFallsBackToProviderDefault() throws {
        let gpt = try provider("gpt")
        let anthropic = try provider("anthropic")
        let google = try provider("google")

        XCTAssertEqual(GaryxProviderSettingsPresentation.effectiveAuthSource(provider: gpt, draft: ""), "codex")
        XCTAssertEqual(GaryxProviderSettingsPresentation.effectiveAuthSource(provider: gpt, draft: "   "), "codex")
        XCTAssertEqual(GaryxProviderSettingsPresentation.effectiveAuthSource(provider: anthropic, draft: ""), "api_key")
        XCTAssertEqual(GaryxProviderSettingsPresentation.effectiveAuthSource(provider: google, draft: ""), "api_key")
        XCTAssertEqual(GaryxProviderSettingsPresentation.effectiveAuthSource(provider: gpt, draft: " api_key "), "api_key")
        XCTAssertEqual(GaryxProviderSettingsPresentation.effectiveAuthSource(provider: anthropic, draft: "codex"), "codex")
    }

    func testAuthSourceLabel() {
        XCTAssertEqual(GaryxProviderSettingsPresentation.authSourceLabel(effectiveAuthSource: "codex"), "Use GPT token")
        XCTAssertEqual(GaryxProviderSettingsPresentation.authSourceLabel(effectiveAuthSource: "api_key"), "Use API key")
        XCTAssertEqual(GaryxProviderSettingsPresentation.authSourceLabel(effectiveAuthSource: "other"), "Use API key")
    }

    func testShowsApiKeyField() throws {
        let claude = try provider("claude_code")
        let codex = try provider("codex_app_server")
        let gpt = try provider("gpt")
        let anthropic = try provider("anthropic")
        let google = try provider("google")

        XCTAssertFalse(GaryxProviderSettingsPresentation.showsApiKeyField(provider: claude, effectiveAuthSource: "api_key"))
        XCTAssertFalse(GaryxProviderSettingsPresentation.showsApiKeyField(provider: codex, effectiveAuthSource: "api_key"))
        XCTAssertFalse(GaryxProviderSettingsPresentation.showsApiKeyField(provider: gpt, effectiveAuthSource: "codex"))
        XCTAssertTrue(GaryxProviderSettingsPresentation.showsApiKeyField(provider: gpt, effectiveAuthSource: "api_key"))
        XCTAssertTrue(GaryxProviderSettingsPresentation.showsApiKeyField(provider: anthropic, effectiveAuthSource: "api_key"))
        // Non-gpt native providers show the key field regardless of source.
        XCTAssertTrue(GaryxProviderSettingsPresentation.showsApiKeyField(provider: anthropic, effectiveAuthSource: "codex"))
        XCTAssertTrue(GaryxProviderSettingsPresentation.showsApiKeyField(provider: google, effectiveAuthSource: "api_key"))
    }

    func testApiKeyPlaceholder() throws {
        XCTAssertEqual(GaryxProviderSettingsPresentation.apiKeyPlaceholder(for: try provider("gpt")), "OPENAI_API_KEY")
        XCTAssertEqual(GaryxProviderSettingsPresentation.apiKeyPlaceholder(for: try provider("anthropic")), "ANTHROPIC_API_KEY")
        XCTAssertEqual(GaryxProviderSettingsPresentation.apiKeyPlaceholder(for: try provider("google")), "GEMINI_API_KEY")
        for type in ["claude_code", "codex_app_server", "antigravity", "traex", "gemini_cli"] {
            XCTAssertEqual(
                GaryxProviderSettingsPresentation.apiKeyPlaceholder(for: try provider(type)),
                "API key",
                "placeholder mismatch for \(type)"
            )
        }
    }

    func testApiKeyDraftAfterSelectingAuthSource() {
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.apiKeyDraft(afterSelectingAuthSource: "codex", current: "sk-x"),
            ""
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.apiKeyDraft(afterSelectingAuthSource: "api_key", current: "sk-x"),
            "sk-x"
        )
    }

    func testDefaultModelLabel() throws {
        let gpt = try provider("gpt")
        let antigravity = try provider("antigravity")
        let claude = try provider("claude_code")

        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(
                provider: gpt,
                catalog: try catalog(#"{"default_model": "gpt-6"}"#)
            ),
            "Provider default: gpt-6"
        )
        // Whitespace-only catalog default falls through to the static fallback.
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(
                provider: antigravity,
                catalog: try catalog(#"{"default_model": "  "}"#)
            ),
            "Provider default: Claude Opus 4.6 (Thinking)"
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(provider: antigravity, catalog: nil),
            "Provider default: Claude Opus 4.6 (Thinking)"
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(provider: claude, catalog: nil),
            "Provider default"
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.defaultModelLabel(provider: claude, catalog: try catalog("{}")),
            "Provider default"
        )
    }

    // MARK: Draft echo

    private var nativeSettings: [String: GaryxJSONValue] {
        [
            "agents": .object([
                "gpt": .object([
                    "default_model": .string("gpt-5.5"),
                    "model_reasoning_effort": .string("high"),
                    "model_service_tier": .string("priority"),
                    "auth_source": .string("api_key"),
                    "base_url": .string("https://example.com/v1"),
                    "env": .object(["OPENAI_API_KEY": .string("sk-test")]),
                ]),
                "claude": .object([
                    "default_model": .string("opus"),
                    "model_reasoning_effort": .string("max"),
                    "model_service_tier": .string("priority"),
                    "auth_source": .string("api_key"),
                    "base_url": .string("https://claude.example.com"),
                    "env": .object(["ANTHROPIC_API_KEY": .string("sk-claude")]),
                ]),
            ])
        ]
    }

    func testDraftMakeEchoesNativeFields() throws {
        let draft = GaryxProviderSettingsPresentation.Draft.make(
            settings: nativeSettings,
            provider: try provider("gpt")
        )
        XCTAssertEqual(draft.modelName, "gpt-5.5")
        XCTAssertEqual(draft.reasoningEffort, "high")
        XCTAssertEqual(draft.serviceTier, "priority")
        XCTAssertEqual(draft.authSource, "api_key")
        XCTAssertEqual(draft.baseUrl, "https://example.com/v1")
        XCTAssertEqual(draft.apiKey, "sk-test")
    }

    func testDraftMakeNonNativeLeavesAuthFieldsEmpty() throws {
        let draft = GaryxProviderSettingsPresentation.Draft.make(
            settings: nativeSettings,
            provider: try provider("claude_code")
        )
        // Defaults echo for every provider; auth fields never echo for
        // non-native providers (matches the old fillDraft guard).
        XCTAssertEqual(draft.modelName, "opus")
        XCTAssertEqual(draft.reasoningEffort, "max")
        XCTAssertEqual(draft.serviceTier, "priority")
        XCTAssertEqual(draft.authSource, "")
        XCTAssertEqual(draft.baseUrl, "")
        XCTAssertEqual(draft.apiKey, "")
    }

    func testDraftMakeEmptySettings() throws {
        let draft = GaryxProviderSettingsPresentation.Draft.make(
            settings: [:],
            provider: try provider("gpt")
        )
        XCTAssertEqual(draft, GaryxProviderSettingsPresentation.Draft(
            modelName: "", reasoningEffort: "", serviceTier: "",
            authSource: "", baseUrl: "", apiKey: ""
        ))
    }

    // MARK: Save payload

    func testSaveRequestNonNativeProvider() throws {
        let request = GaryxProviderSettingsPresentation.SaveRequest.make(
            provider: try provider("claude_code"),
            catalog: try catalog(#"{"supports_service_tier_selection": true}"#),
            modelName: "opus",
            reasoningEffort: "max",
            serviceTier: "priority",
            authSourceDraft: "api_key",
            baseUrl: "https://ignored.example.com",
            apiKeyDraft: "sk-ignored",
            originalApiKey: "sk-old"
        )
        XCTAssertEqual(request.modelName, "opus")
        XCTAssertEqual(request.reasoningEffort, "max")
        XCTAssertNil(request.serviceTier)
        XCTAssertNil(request.authSource)
        XCTAssertNil(request.baseUrl)
        XCTAssertEqual(request.apiKey, .keep)
    }

    func testSaveRequestGptWithTierSupport() throws {
        let gpt = try provider("gpt")
        let supporting = try catalog(#"{"supports_service_tier_selection": true}"#)

        let request = GaryxProviderSettingsPresentation.SaveRequest.make(
            provider: gpt,
            catalog: supporting,
            modelName: "gpt-5.5",
            reasoningEffort: "high",
            serviceTier: "flex",
            authSourceDraft: "",
            baseUrl: "https://example.com/v1",
            apiKeyDraft: "",
            originalApiKey: ""
        )
        XCTAssertEqual(request.serviceTier, "flex")
        // Empty auth draft resolves to the gpt default (shared token).
        XCTAssertEqual(request.authSource, "codex")
        XCTAssertEqual(request.baseUrl, "https://example.com/v1")
        XCTAssertEqual(request.apiKey, .keep)
    }

    func testSaveRequestApiKeyUpdateMatrix() throws {
        let gpt = try provider("gpt")
        func apiKey(draft: String, original: String) throws -> GaryxProviderApiKeyUpdate {
            GaryxProviderSettingsPresentation.SaveRequest.make(
                provider: gpt,
                catalog: nil,
                modelName: "",
                reasoningEffort: "",
                serviceTier: "",
                authSourceDraft: "api_key",
                baseUrl: "",
                apiKeyDraft: draft,
                originalApiKey: original
            ).apiKey
        }
        XCTAssertEqual(try apiKey(draft: " sk-new ", original: "sk-old"), .set("sk-new"))
        XCTAssertEqual(try apiKey(draft: "", original: "sk-old"), .blank)
        XCTAssertEqual(try apiKey(draft: "   ", original: "sk-old"), .blank)
        XCTAssertEqual(try apiKey(draft: "", original: ""), .keep)
        XCTAssertEqual(try apiKey(draft: "", original: "   "), .keep)
    }

    func testSaveRequestGptWithoutTierSupport() throws {
        let gpt = try provider("gpt")
        for catalogJson in [#"{"supports_service_tier_selection": false}"#, nil] {
            let request = GaryxProviderSettingsPresentation.SaveRequest.make(
                provider: gpt,
                catalog: try catalogJson.map { try catalog($0) },
                modelName: "gpt-5.5",
                reasoningEffort: "",
                serviceTier: "flex",
                authSourceDraft: "codex",
                baseUrl: "",
                apiKeyDraft: "",
                originalApiKey: ""
            )
            XCTAssertNil(request.serviceTier)
            XCTAssertEqual(request.authSource, "codex")
        }
    }

    func testSaveRequestAnthropic() throws {
        let request = GaryxProviderSettingsPresentation.SaveRequest.make(
            provider: try provider("anthropic"),
            catalog: try catalog(#"{"supports_service_tier_selection": true}"#),
            modelName: "claude-sonnet-4-6",
            reasoningEffort: "high",
            serviceTier: "flex",
            authSourceDraft: "",
            baseUrl: "https://claude.example.com",
            apiKeyDraft: "sk-a",
            originalApiKey: ""
        )
        // Service tier stays gpt-only even when a catalog claims support.
        XCTAssertNil(request.serviceTier)
        XCTAssertEqual(request.authSource, "api_key")
        XCTAssertEqual(request.baseUrl, "https://claude.example.com")
        XCTAssertEqual(request.apiKey, .set("sk-a"))
    }

    // MARK: Provider list row

    func testRowModelStatus() throws {
        let claude = try provider("claude_code")

        let error = GaryxProviderSettingsPresentation.RowModel.make(
            provider: claude,
            catalog: try catalog(#"{"error": "boom"}"#),
            settings: [:]
        )
        XCTAssertEqual(error.statusText, "Error")
        XCTAssertEqual(error.statusTone, .danger)

        let loading = GaryxProviderSettingsPresentation.RowModel.make(
            provider: claude, catalog: nil, settings: [:]
        )
        XCTAssertEqual(loading.statusText, "Loading")
        XCTAssertEqual(loading.statusTone, .muted)

        let ready = GaryxProviderSettingsPresentation.RowModel.make(
            provider: claude, catalog: try catalog("{}"), settings: [:]
        )
        XCTAssertEqual(ready.statusText, "Ready")
        XCTAssertEqual(ready.statusTone, .good)

        // Whitespace-only error reads as no error.
        let whitespace = GaryxProviderSettingsPresentation.RowModel.make(
            provider: claude, catalog: try catalog(#"{"error": "  "}"#), settings: [:]
        )
        XCTAssertEqual(whitespace.statusText, "Ready")
        XCTAssertEqual(whitespace.statusTone, .good)
    }

    func testRowModelDetailFallbackTexts() throws {
        let claude = try provider("claude_code")

        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: claude, catalog: nil, settings: [:]
            ).detailText,
            "Loading metadata"
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: claude, catalog: try catalog("{}"), settings: [:]
            ).detailText,
            "Provider metadata"
        )
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: claude, catalog: try catalog(#"{"error": "boom"}"#), settings: [:]
            ).detailText,
            "Model metadata unavailable"
        )
    }

    func testRowModelDetailEffectiveModelChain() throws {
        let gpt = try provider("gpt")
        let antigravity = try provider("antigravity")
        let configured: [String: GaryxJSONValue] = [
            "agents": .object([
                "gpt": .object(["default_model": .string("m1")])
            ])
        ]

        // Configured model beats the catalog default.
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: gpt,
                catalog: try catalog(#"{"default_model": "m2"}"#),
                settings: configured
            ).detailText,
            "Default m1"
        )
        // Catalog default when nothing is configured.
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: gpt,
                catalog: try catalog(#"{"default_model": "m2"}"#),
                settings: [:]
            ).detailText,
            "Default m2"
        )
        // Static fallback when neither exists.
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: antigravity, catalog: nil, settings: [:]
            ).detailText,
            "Default Claude Opus 4.6 (Thinking)"
        )
        // An errored catalog still renders configured parts.
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: gpt,
                catalog: try catalog(#"{"error": "boom"}"#),
                settings: configured
            ).detailText,
            "Default m1"
        )
    }

    func testRowModelDetailCapabilityCounts() throws {
        let gpt = try provider("gpt")
        let catalog = try catalog(#"""
        {
            "default_model": "gpt-5.5",
            "supports_model_selection": true,
            "models": [{"id": "a"}, {"id": "b"}],
            "supports_reasoning_effort_selection": true,
            "reasoning_efforts": [{"id": "low"}, {"id": "medium"}, {"id": "high"}],
            "supports_service_tier_selection": true,
            "service_tiers": [{"id": "standard"}, {"id": "flex"}]
        }
        """#)
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "gpt": .object(["model_reasoning_effort": .string("high")])
            ])
        ]
        XCTAssertEqual(
            GaryxProviderSettingsPresentation.RowModel.make(
                provider: gpt, catalog: catalog, settings: settings
            ).detailText,
            "Default gpt-5.5 · Thinking high · 2 models · 3 reasoning · 2 tiers"
        )
    }
}
