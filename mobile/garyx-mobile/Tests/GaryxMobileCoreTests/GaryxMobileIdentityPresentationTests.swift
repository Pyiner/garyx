import XCTest
@testable import GaryxMobileCore

final class GaryxMobileIdentityPresentationTests: XCTestCase {
    func testProviderPresentationCentralizesKnownSymbolsAndNames() {
        XCTAssertEqual(GaryxProviderPresentation.make(providerType: "codex_app_server").displayName, "Codex")
        XCTAssertEqual(
            GaryxProviderPresentation.make(providerType: "codex_app_server").symbolName,
            "chevron.left.forwardslash.chevron.right"
        )
        XCTAssertEqual(GaryxProviderPresentation.make(providerType: "antigravity").displayName, "Antigravity")
        XCTAssertEqual(GaryxProviderPresentation.make(providerType: "antigravity").symbolName, "bolt.fill")
        XCTAssertEqual(GaryxProviderPresentation.make(providerType: "claude_code").symbolName, "sparkles")
        XCTAssertEqual(GaryxProviderPresentation.make(providerType: "gemini_cli").displayName, "Gemini CLI")
        XCTAssertEqual(GaryxProviderPresentation.make(providerType: "gpt").displayName, "OpenAI")
    }

    func testProviderPresentationUsesAgentAndProviderForAvatarFallbacks() {
        let presentation = GaryxProviderPresentation.make(
            agentId: "assistant-codex",
            providerType: nil,
            fallbackName: "Code Agent"
        )
        XCTAssertEqual(presentation.kind, .codex)
        XCTAssertEqual(presentation.symbolName, "chevron.left.forwardslash.chevron.right")
        XCTAssertEqual(presentation.fallbackInitials, "CA")
    }

    func testProviderPresentationPrefersProviderTypeOverAgentId() {
        let presentation = GaryxProviderPresentation.make(
            agentId: "gemini-specialist",
            providerType: "claude_code",
            fallbackName: "Claude Specialist"
        )
        XCTAssertEqual(presentation.kind, .claude)
        XCTAssertEqual(presentation.symbolName, "sparkles")
        XCTAssertEqual(presentation.displayName, "Claude Code")
    }

    func testProviderPresentationExposesAvatarFallbackStyleData() {
        let codex = GaryxProviderPresentation.make(providerType: "codex_app_server")
        XCTAssertEqual(codex.fallbackBackgroundRGB, GaryxProviderFallbackRGB(red: 0.08, green: 0.10, blue: 0.12))
        XCTAssertEqual(codex.iconSizeFactor, 0.32)
        XCTAssertTrue(codex.prefersLightFallbackForeground)

        let generic = GaryxProviderPresentation.make(providerType: "")
        XCTAssertEqual(generic.iconSizeFactor, 0.36)
        XCTAssertFalse(generic.prefersLightFallbackForeground)
    }

    func testChannelPresentationNormalizesDisplayNamesAssetsAndInitials() {
        let telegram = GaryxChannelIdentityPresentation.make(channel: "telegram", label: "Mobile Bot")
        XCTAssertEqual(telegram.displayName, "Telegram")
        XCTAssertEqual(telegram.fallbackAssetName, "ChannelTelegram")
        XCTAssertEqual(telegram.fallbackInitials, "MB")

        let custom = GaryxChannelIdentityPresentation.make(channel: "custom_channel")
        XCTAssertEqual(custom.displayName, "Custom Channel")
        XCTAssertNil(custom.fallbackAssetName)
        XCTAssertEqual(custom.fallbackInitials, "CC")
    }

    func testChannelPresentationPrefersCatalogDisplayName() {
        XCTAssertEqual(
            GaryxChannelIdentityPresentation.displayName(for: "lark_im", catalogDisplayName: "Lark IM"),
            "Lark IM"
        )
        XCTAssertEqual(
            GaryxChannelIdentityPresentation.displayName(for: "lark_im", catalogDisplayName: " "),
            "Lark Im"
        )
    }
}
