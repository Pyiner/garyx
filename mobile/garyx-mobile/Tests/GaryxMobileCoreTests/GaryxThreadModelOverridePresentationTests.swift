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

    func testProviderLevelDefaultReasoningEffortDecodesSnakeAndCamelKeys() throws {
        XCTAssertEqual(
            try decodeProviderModels(#"{ "default_reasoning_effort": "max" }"#).defaultReasoningEffort,
            "max"
        )
        XCTAssertEqual(
            try decodeProviderModels(#"{ "defaultReasoningEffort": "high" }"#).defaultReasoningEffort,
            "high"
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

    func testDefaultStateUsesProviderDefaultModelAndConfiguredReasoningEffort() throws {
        let providerModels = try decodeProviderModels(configuredClaudeProviderJSON)

        let defaultOptions = GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: nil
        )
        XCTAssertEqual(defaultOptions.map(\.id), ["low", "high", "max"])

        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.defaultReasoningEffort(
                providerModels: providerModels,
                model: nil
            ),
            "max"
        )

        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.controlLabel(
                providerModels: providerModels,
                model: nil,
                reasoningEffort: nil,
                fallback: "Model"
            ),
            "Claude Opus 4.8 · Max"
        )
    }

    func testConfiguredProviderDefaultReasoningEffortMustBeSupportedByCurrentModel() throws {
        let providerModels = try decodeProviderModels(configuredClaudeProviderJSON)

        let sonnetOptions = GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: "claude-sonnet-4-6"
        )
        XCTAssertEqual(sonnetOptions.map(\.id), ["low", "high"])

        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.defaultReasoningEffort(
                providerModels: providerModels,
                model: "claude-sonnet-4-6"
            ),
            "high"
        )
    }

    func testTraexPerModelReasoningEffortsRenderThroughGatewayShape() throws {
        let providerModels = try decodeProviderModels(traexProviderJSON)

        let reasonerOptions = GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: "traex-reasoner"
        )
        XCTAssertEqual(reasonerOptions.map(\.id), ["medium", "max"])
        XCTAssertEqual(reasonerOptions.map(\.label), ["Medium", "max"])
        XCTAssertTrue(
            GaryxThreadModelOverridePresentation.reasoningEffortOptions(
                providerModels: providerModels,
                model: "traex-fast"
            ).isEmpty
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.modelLabel(
                providerModels: providerModels,
                model: "traex-reasoner"
            ),
            "traex-reasoner"
        )
    }

    func testReasoningEffortOptionsEmptyWhenSelectionUnsupported() throws {
        let providerModels = try decodeProviderModels(googleProviderJSON)
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

    func testDefaultReasoningEffortRequiresActualModel() throws {
        let providerModels = try decodeProviderModels(claudeProviderJSON)

        XCTAssertNil(
            GaryxThreadModelOverridePresentation.defaultReasoningEffort(
                providerModels: providerModels,
                model: nil
            )
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.defaultReasoningEffort(
                providerModels: providerModels,
                model: "claude-opus-4-7"
            ),
            "xhigh"
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.defaultReasoningEffort(
                providerModels: providerModels,
                model: "not-in-catalog"
            ),
            "high"
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

    func testEffortFilterModelPrefersOverrideThenAgentModel() throws {
        let providerModels = try decodeProviderModels(configuredClaudeProviderJSON)

        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.effortFilterModel(
                override: "claude-opus-4-8",
                agentConfiguredModel: "claude-haiku-4-5",
                providerModels: providerModels
            ),
            "claude-opus-4-8"
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.effortFilterModel(
                override: "  ",
                agentConfiguredModel: "claude-haiku-4-5",
                providerModels: providerModels
            ),
            "claude-haiku-4-5"
        )
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.effortFilterModel(
                override: nil,
                agentConfiguredModel: "",
                providerModels: providerModels
            ),
            "claude-opus-4-8"
        )
        XCTAssertNil(
            GaryxThreadModelOverridePresentation.effortFilterModel(
                override: nil,
                agentConfiguredModel: ""
            )
        )
    }

    func testSelectedOptionIdReflectsEffectiveValueNotDefault() {
        // Effective equals the default -> the "use default" row ("") is selected.
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.selectedOptionId(effective: "high", default: "high"),
            ""
        )
        // Effective differs from the default -> the effective value's own row is
        // selected, so the picker checkmark matches the summary row instead of
        // falling back to the default (the "Max outside, High in the picker" bug).
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.selectedOptionId(effective: "max", default: "high"),
            "max"
        )
        // No effective value, or whitespace -> default row.
        XCTAssertEqual(GaryxThreadModelOverridePresentation.selectedOptionId(effective: nil, default: "high"), "")
        XCTAssertEqual(GaryxThreadModelOverridePresentation.selectedOptionId(effective: "  ", default: "high"), "")
        // No default known -> any effective value selects its own row.
        XCTAssertEqual(GaryxThreadModelOverridePresentation.selectedOptionId(effective: "max", default: nil), "max")
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
                "default_reasoning_effort": "xhigh",
                "supported_reasoning_efforts": [
                    { "id": "low", "label": "Low", "recommended": false },
                    { "id": "high", "label": "High", "recommended": true },
                    { "id": "xhigh", "label": "Extra High", "recommended": false }
                ]
            }
        ]
    }
    """

    private let configuredClaudeProviderJSON = """
    {
        "provider_type": "claude_code",
        "supports_model_selection": true,
        "supports_reasoning_effort_selection": true,
        "default_model": "claude-opus-4-8",
        "default_reasoning_effort": "max",
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
                "id": "claude-opus-4-8",
                "label": "Claude Opus 4.8",
                "recommended": false,
                "supported_reasoning_efforts": [
                    { "id": "low", "label": "Low", "recommended": false },
                    { "id": "high", "label": "High", "recommended": true },
                    { "id": "max", "label": "Max", "recommended": false }
                ]
            }
        ]
    }
    """

    private let traexProviderJSON = """
    {
        "provider_type": "traex",
        "supports_model_selection": true,
        "supports_reasoning_effort_selection": true,
        "default_model": "traex-fast",
        "source": "traex_builtin",
        "reasoning_efforts": [],
        "models": [
            {
                "id": "traex-fast",
                "label": "TRAE Fast",
                "recommended": true,
                "supported_reasoning_efforts": []
            },
            {
                "id": "traex-reasoner",
                "recommended": false,
                "supported_reasoning_efforts": [
                    { "id": "medium", "label": "Medium", "recommended": true },
                    { "id": "max", "recommended": false }
                ]
            }
        ]
    }
    """

    private let googleProviderJSON = """
    {
        "provider_type": "google",
        "supports_model_selection": true,
        "supports_reasoning_effort_selection": false,
        "default_model": "gemini-3-pro",
        "source": "native_builtin",
        "models": [
            { "id": "gemini-3-pro", "label": "Gemini 3 Pro", "recommended": true }
        ]
    }
    """

    private let unsupportedProviderJSON = """
    {
        "provider_type": "unsupported_provider",
        "supports_model_selection": false,
        "source": "provider",
        "models": []
    }
    """
}
