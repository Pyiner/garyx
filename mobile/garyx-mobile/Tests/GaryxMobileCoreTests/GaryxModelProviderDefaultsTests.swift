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
}
