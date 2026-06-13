import XCTest
@testable import GaryxMobileCore

final class GaryxModelProviderDefaultsTests: XCTestCase {
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
