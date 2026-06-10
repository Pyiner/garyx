import XCTest
@testable import GaryxMobileCore

final class GaryxThreadModelOverridePresentationTests: XCTestCase {
    func testSupportsOverrideRequiresModelSelection() throws {
        XCTAssertFalse(GaryxThreadModelOverridePresentation.supportsOverride(nil))
        XCTAssertFalse(
            GaryxThreadModelOverridePresentation.supportsOverride(
                try decodeProviderModels(unsupportedProviderJSON)
            )
        )
        XCTAssertTrue(
            GaryxThreadModelOverridePresentation.supportsOverride(
                try decodeProviderModels(claudeProviderJSON)
            )
        )
    }

    func testReasoningEffortOptionsFollowSelectedModel() throws {
        let providerModels = try decodeProviderModels(claudeProviderJSON)

        let defaultOptions = GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: nil
        )
        XCTAssertEqual(defaultOptions.map(\.id), ["low", "high"])

        let opusOptions = GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: "claude-opus-4-7"
        )
        XCTAssertEqual(opusOptions.map(\.id), ["low", "high", "xhigh"])

        let unknownModelOptions = GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: "not-in-catalog"
        )
        XCTAssertEqual(unknownModelOptions.map(\.id), ["low", "high"])
    }

    func testReasoningEffortOptionsEmptyWhenSelectionUnsupported() throws {
        let providerModels = try decodeProviderModels(geminiProviderJSON)
        XCTAssertTrue(
            GaryxThreadModelOverridePresentation.reasoningEffortOptions(
                providerModels: providerModels,
                model: "gemini-3-pro"
            ).isEmpty
        )
    }

    func testSanitizedReasoningEffortDropsUnsupportedLevel() throws {
        let providerModels = try decodeProviderModels(claudeProviderJSON)

        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
                providerModels: providerModels,
                model: "claude-opus-4-7",
                reasoningEffort: "xhigh"
            ),
            "xhigh"
        )
        XCTAssertNil(
            GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
                providerModels: providerModels,
                model: "claude-sonnet-4-6",
                reasoningEffort: "xhigh"
            )
        )
        XCTAssertNil(
            GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
                providerModels: providerModels,
                model: nil,
                reasoningEffort: "   "
            )
        )
    }

    func testControlLabelComposition() throws {
        let providerModels = try decodeProviderModels(claudeProviderJSON)

        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.controlLabel(
                providerModels: providerModels,
                model: nil,
                reasoningEffort: nil,
                fallback: "Model"
            ),
            "Model"
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.controlLabel(
                providerModels: providerModels,
                model: "claude-opus-4-7",
                reasoningEffort: nil,
                fallback: "Model"
            ),
            "Claude Opus 4.7"
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.controlLabel(
                providerModels: providerModels,
                model: "claude-opus-4-7",
                reasoningEffort: "xhigh",
                fallback: "Model"
            ),
            "Claude Opus 4.7 · Extra High"
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.controlLabel(
                providerModels: providerModels,
                model: nil,
                reasoningEffort: "high",
                fallback: "Model"
            ),
            "Model · High"
        )
    }

    func testModelLabelFallsBackToRawIdentifier() throws {
        let providerModels = try decodeProviderModels(claudeProviderJSON)
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.modelLabel(
                providerModels: providerModels,
                model: "custom-model"
            ),
            "custom-model"
        )
        XCTAssertNil(
            GaryxThreadModelOverridePresentation.modelLabel(
                providerModels: providerModels,
                model: "  "
            )
        )
    }

    private func decodeProviderModels(_ json: String) throws -> GaryxProviderModels {
        try JSONDecoder().decode(GaryxProviderModels.self, from: Data(json.utf8))
    }

    private let claudeProviderJSON = """
    {
        "provider_type": "claude_code",
        "supports_model_selection": true,
        "supports_reasoning_effort_selection": true,
        "default_model": "claude-sonnet-4-6",
        "source": "claude_code_builtin",
        "reasoning_efforts": [
            { "id": "low", "label": "Low", "recommended": false },
            { "id": "high", "label": "High", "recommended": true }
        ],
        "models": [
            {
                "id": "claude-sonnet-4-6",
                "label": "Claude Sonnet 4.6",
                "recommended": true,
                "supported_reasoning_efforts": [
                    { "id": "low", "label": "Low", "recommended": false },
                    { "id": "high", "label": "High", "recommended": true }
                ]
            },
            {
                "id": "claude-opus-4-7",
                "label": "Claude Opus 4.7",
                "recommended": false,
                "supported_reasoning_efforts": [
                    { "id": "low", "label": "Low", "recommended": false },
                    { "id": "high", "label": "High", "recommended": true },
                    { "id": "xhigh", "label": "Extra High", "recommended": false }
                ]
            }
        ]
    }
    """

    private let geminiProviderJSON = """
    {
        "provider_type": "gemini_cli",
        "supports_model_selection": true,
        "supports_reasoning_effort_selection": false,
        "default_model": "gemini-3-pro",
        "source": "gemini_acp",
        "models": [
            { "id": "gemini-3-pro", "label": "Gemini 3 Pro", "recommended": true }
        ]
    }
    """

    private let unsupportedProviderJSON = """
    {
        "provider_type": "agent_team",
        "supports_model_selection": false,
        "source": "provider",
        "models": []
    }
    """
}
