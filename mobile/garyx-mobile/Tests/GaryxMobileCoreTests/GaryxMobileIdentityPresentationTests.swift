import XCTest
@testable import GaryxMobileCore

final class GaryxMobileIdentityPresentationTests: XCTestCase {
    func testProviderPresentationCentralizesKnownSymbolsAndNames() {
        XCTAssertEqual(GaryxProviderPresentation.make(providerType: "codex_app_server").displayName, "Codex")
        XCTAssertEqual(
            GaryxProviderPresentation.make(providerType: "codex_app_server").symbolName,
            "chevron.left.forwardslash.chevron.right"
        )
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
}
