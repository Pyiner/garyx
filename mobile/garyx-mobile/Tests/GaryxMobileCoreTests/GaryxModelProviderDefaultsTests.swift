import XCTest
@testable import GaryxMobileCore

final class GaryxModelProviderDefaultsTests: XCTestCase {
    func testProviderUsageIdsAreExplicitAndCoverCodingProviders() throws {
        let claude = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "claude_code"))
        XCTAssertEqual(claude.usageProviderId, "claude_code")
        XCTAssertEqual(claude.configKey, "claude")

        let codex = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "codex_app_server"))
        XCTAssertEqual(codex.usageProviderId, "codex")
        XCTAssertEqual(codex.configKey, "codex")

        let antigravity = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "antigravity"))
        XCTAssertEqual(antigravity.usageProviderId, "antigravity")
        XCTAssertEqual(antigravity.configKey, "antigravity")
        XCTAssertEqual(antigravity.fallbackDefaultModel, "Claude Opus 4.6 (Thinking)")
    }

    func testUsageLookupUsesUsageProviderIdNotProviderType() throws {
        let codex = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "codex_app_server"))
        let usage = GaryxCodingUsage(providers: [
            GaryxProviderUsage(id: "codex", name: "Codex", available: true),
            GaryxProviderUsage(id: "codex_app_server", name: "Wrong", available: false),
        ])

        XCTAssertEqual(
            GaryxModelProviderDefaults.usage(in: usage, provider: codex)?.id,
            "codex"
        )
    }

    func testUpdateWritesCanonicalProviderConfig() {
        let provider = GaryxModelProviderDefaults.provider(for: "claude_code")
        XCTAssertNotNil(provider)
        guard let provider else { return }

        var settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "codex": .object([
                    "provider_type": .string("codex_app_server"),
                    "default_model": .string("gpt-5.5"),
                ]),
            ]),
        ]

        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: " claude-opus-4-8 ",
            reasoningEffort: " max "
        )

        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredDefaultModel(in: settings, provider: provider),
            "claude-opus-4-8"
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredReasoningEffort(in: settings, provider: provider),
            "max"
        )

        guard case let .object(agents)? = settings["agents"],
              case let .object(claude)? = agents["claude"],
              case let .object(codex)? = agents["codex"] else {
            return XCTFail("expected provider configs")
        }
        XCTAssertEqual(claude["provider_type"], .string("claude_code"))
        XCTAssertEqual(codex["default_model"], .string("gpt-5.5"))
    }

    // MARK: - Native auth editing (design §6.2 / D1)

    func testApiKeyEnvNameMapMatchesMac() {
        XCTAssertEqual(GaryxModelProviderDefaults.apiKeyEnvName(forProviderType: "gpt"), "OPENAI_API_KEY")
        XCTAssertEqual(GaryxModelProviderDefaults.apiKeyEnvName(forProviderType: "anthropic"), "ANTHROPIC_API_KEY")
        XCTAssertEqual(GaryxModelProviderDefaults.apiKeyEnvName(forProviderType: "google"), "GEMINI_API_KEY")
        XCTAssertNil(GaryxModelProviderDefaults.apiKeyEnvName(forProviderType: "claude_code"))
        XCTAssertNil(GaryxModelProviderDefaults.apiKeyEnvName(forProviderType: "codex_app_server"))
    }

    func testNativeProviderPartitionAndDefaultAuthSource() {
        for providerType in ["gpt", "anthropic", "google"] {
            let provider = GaryxModelProviderDefaults.provider(for: providerType)
            XCTAssertEqual(provider?.isNative, true, providerType)
        }
        for providerType in ["claude_code", "codex_app_server", "antigravity", "traex", "gemini_cli"] {
            let provider = GaryxModelProviderDefaults.provider(for: providerType)
            XCTAssertEqual(provider?.isNative, false, providerType)
        }
        XCTAssertEqual(GaryxModelProviderDefaults.defaultNativeAuthSource(forProviderType: "gpt"), "codex")
        XCTAssertEqual(GaryxModelProviderDefaults.defaultNativeAuthSource(forProviderType: "anthropic"), "api_key")
        XCTAssertEqual(GaryxModelProviderDefaults.defaultNativeAuthSource(forProviderType: "google"), "api_key")
    }

    func testUpdateWritesNativeAuthFieldsAndMergesApiKeyEnv() throws {
        let provider = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "anthropic"))
        var settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "anthropic": .object([
                    "provider_type": .string("anthropic"),
                    "env": .object([
                        "ANTHROPIC_API_KEY": .string("sk-ant-OLD"),
                        "OTHER_VAR": .string("kept"),
                    ]),
                ]),
            ]),
        ]

        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: "claude-sonnet-4-6",
            reasoningEffort: "",
            authSource: "  ",
            baseUrl: " https://proxy.example.com/v1 ",
            apiKey: .set(" sk-ant-EXAMPLE ")
        )

        guard case let .object(agents)? = settings["agents"],
              case let .object(config)? = agents["anthropic"],
              case let .object(env)? = config["env"] else {
            return XCTFail("expected anthropic provider config with env")
        }
        // Blank auth source falls back to the provider's default, like Mac.
        XCTAssertEqual(config["auth_source"], .string("api_key"))
        XCTAssertEqual(config["base_url"], .string("https://proxy.example.com/v1"))
        XCTAssertEqual(env["ANTHROPIC_API_KEY"], .string("sk-ant-EXAMPLE"))
        // Sibling env vars survive the merge.
        XCTAssertEqual(env["OTHER_VAR"], .string("kept"))
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredApiKey(in: settings, provider: provider),
            "sk-ant-EXAMPLE"
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredBaseUrl(in: settings, provider: provider),
            "https://proxy.example.com/v1"
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredAuthSource(in: settings, provider: provider),
            "api_key"
        )
    }

    func testUpdateApiKeyKeepAndBlankSemantics() throws {
        let provider = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "google"))
        var settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "google": .object([
                    "env": .object(["GEMINI_API_KEY": .string("old-key")]),
                ]),
            ]),
        ]

        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: "",
            reasoningEffort: "",
            apiKey: .keep
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredApiKey(in: settings, provider: provider),
            "old-key"
        )

        // Deep-merge cannot delete env keys, so clearing writes an empty string.
        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: "",
            reasoningEffort: "",
            apiKey: .blank
        )
        guard case let .object(agents)? = settings["agents"],
              case let .object(config)? = agents["google"],
              case let .object(env)? = config["env"] else {
            return XCTFail("expected google provider config with env")
        }
        XCTAssertEqual(env["GEMINI_API_KEY"], .string(""))
    }

    func testApiKeyUpdateDecisionFromDraftAndExisting() {
        XCTAssertEqual(GaryxProviderApiKeyUpdate.make(draft: " sk-new ", existing: ""), .set("sk-new"))
        XCTAssertEqual(GaryxProviderApiKeyUpdate.make(draft: "", existing: "sk-old"), .blank)
        XCTAssertEqual(GaryxProviderApiKeyUpdate.make(draft: "  ", existing: ""), .keep)
        XCTAssertEqual(GaryxProviderApiKeyUpdate.make(draft: "", existing: "  "), .keep)
    }

    func testUpdateDoesNotWriteAuthFieldsForCliProvidersOrWhenOmitted() throws {
        let claude = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "claude_code"))
        var settings: [String: GaryxJSONValue] = [:]
        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: claude,
            model: "claude-opus-4-8",
            reasoningEffort: "max",
            serviceTier: "flex",
            authSource: "api_key",
            baseUrl: "https://example.com",
            apiKey: .set("sk-should-not-write")
        )
        guard case let .object(agents)? = settings["agents"],
              case let .object(config)? = agents["claude"] else {
            return XCTFail("expected claude provider config")
        }
        XCTAssertNil(config["auth_source"])
        XCTAssertNil(config["base_url"])
        XCTAssertNil(config["model_service_tier"])
        XCTAssertNil(config["env"])

        // Native update with everything omitted keeps the legacy two-field shape.
        let anthropic = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "anthropic"))
        var nativeSettings: [String: GaryxJSONValue] = [:]
        GaryxModelProviderDefaults.update(
            settings: &nativeSettings,
            provider: anthropic,
            model: "claude-sonnet-4-6",
            reasoningEffort: ""
        )
        guard case let .object(nativeAgents)? = nativeSettings["agents"],
              case let .object(nativeConfig)? = nativeAgents["anthropic"] else {
            return XCTFail("expected anthropic provider config")
        }
        XCTAssertNil(nativeConfig["auth_source"])
        XCTAssertNil(nativeConfig["base_url"])
        XCTAssertNil(nativeConfig["env"])
    }

    func testUpdateWritesServiceTierOnlyForGpt() throws {
        let gpt = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "gpt"))
        var settings: [String: GaryxJSONValue] = [:]
        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: gpt,
            model: "gpt-5.5",
            reasoningEffort: "high",
            serviceTier: " flex ",
            authSource: "codex",
            baseUrl: ""
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredServiceTier(in: settings, provider: gpt),
            "flex"
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredAuthSource(in: settings, provider: gpt),
            "codex"
        )

        let anthropic = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "anthropic"))
        var anthropicSettings: [String: GaryxJSONValue] = [:]
        GaryxModelProviderDefaults.update(
            settings: &anthropicSettings,
            provider: anthropic,
            model: "",
            reasoningEffort: "",
            serviceTier: "flex"
        )
        XCTAssertEqual(
            GaryxModelProviderDefaults.configuredServiceTier(in: anthropicSettings, provider: anthropic),
            ""
        )
        guard case let .object(agents)? = anthropicSettings["agents"],
              case let .object(config)? = agents["anthropic"] else {
            return XCTFail("expected anthropic provider config")
        }
        XCTAssertNil(config["model_service_tier"])
    }

    // MARK: - Host runtime fields (read-only mirror)

    func testHostRuntimeFieldsListOnlyApplicableNonEmptyFields() throws {
        let claude = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "claude_code"))
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "claude": .object([
                    "claude_cli_mode": .string("cctty"),
                    "claude_cli_path": .string("/Users/test/.local/bin/claude"),
                    "permission_mode": .string(""),
                    "codex_home": .string("/Users/test/.codex"),
                ]),
            ]),
        ]

        let fields = GaryxModelProviderDefaults.hostRuntimeFields(in: settings, provider: claude)
        XCTAssertEqual(fields.map(\.label), ["CLI mode", "CLI path"])
        XCTAssertEqual(fields[0].value, "cctty")
        XCTAssertEqual(fields[1].value, "/Users/test/.local/bin/claude")
    }

    func testHostRuntimeFieldsEnvMirrorExcludesApiKeyEnv() throws {
        let google = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "google"))
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "google": .object([
                    "env": .object([
                        "GEMINI_API_KEY": .string("sk-live-secret"),
                        "HTTPS_PROXY": .string("http://127.0.0.1:7890"),
                        "B_VAR": .string("b"),
                    ]),
                ]),
            ]),
        ]

        let fields = GaryxModelProviderDefaults.hostRuntimeFields(in: settings, provider: google)
        XCTAssertEqual(fields.map(\.label), ["Environment"])
        XCTAssertEqual(fields[0].value, "B_VAR=b\nHTTPS_PROXY=http://127.0.0.1:7890")
        XCTAssertFalse(fields[0].value.contains("GEMINI_API_KEY"))
    }

    func testHostRuntimeFieldsForCliBinaryProviders() throws {
        let gemini = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "gemini_cli"))
        let settings: [String: GaryxJSONValue] = [
            "agents": .object([
                "gemini": .object([
                    "gemini_bin": .string("/opt/homebrew/bin/gemini"),
                    "approval_mode": .string("yolo"),
                ]),
                "antigravity": .object([
                    "antigravity_bin": .string("/usr/local/bin/antigravity"),
                ]),
            ]),
        ]
        XCTAssertEqual(
            GaryxModelProviderDefaults.hostRuntimeFields(in: settings, provider: gemini).map(\.label),
            ["Gemini binary", "Approval mode"]
        )

        let antigravity = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "antigravity"))
        XCTAssertEqual(
            GaryxModelProviderDefaults.hostRuntimeFields(in: settings, provider: antigravity).map(\.label),
            ["Antigravity binary"]
        )
    }
}
