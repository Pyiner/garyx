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
        let providerModels = try decodeProviderModels(unsupportedProviderJSON)
        XCTAssertTrue(
            GaryxThreadModelOverridePresentation.reasoningEffortOptions(
                providerModels: providerModels,
                model: "unsupported-model"
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

    func testSelectedPickerOptionIdFollowsTheThreadCell() {
        // A pinned cell checks its own row — including when that value is also
        // the current default. Resolving the effective value instead checked the
        // follow-default row and left the pinned row with no checkmark.
        XCTAssertEqual(GaryxThreadModelOverridePresentation.selectedPickerOptionId(cell: "max"), "max")
        XCTAssertEqual(
            GaryxThreadModelOverridePresentation.selectedPickerOptionId(cell: "claude-opus-5"),
            "claude-opus-5"
        )
        // An empty cell follows the default, so the empty-id row is checked.
        XCTAssertEqual(GaryxThreadModelOverridePresentation.selectedPickerOptionId(cell: nil), "")
        XCTAssertEqual(GaryxThreadModelOverridePresentation.selectedPickerOptionId(cell: "  "), "")
    }

    /// Regression: the provider default must keep its own real-id row.
    ///
    /// The picker used to label the empty follow-default row with the default
    /// model's own label and then suppress that model's real row. With
    /// `default_model: claude-opus-5` the only row reading "Claude Opus 5" sent
    /// `{"model":""}`, which cleared the thread cell and silently fell back to
    /// the bound agent's model instead of pinning Opus 5.
    func testModelPickerKeepsARealRowForTheProviderDefault() throws {
        let providerModels = try decodeProviderModels(defaultIsFirstModelProviderJSON)
        XCTAssertEqual(providerModels.defaultModel, "claude-opus-5")

        let options = GaryxThreadModelOverridePresentation.modelPickerOptions(
            providerModels: providerModels,
            effectiveModel: "claude-opus-5",
            defaultRowLabel: "Agent default"
        )

        XCTAssertEqual(options.map(\.id), ["", "claude-opus-5", "claude-sonnet-5"])
        // Row 0 never borrows a concrete model's label.
        XCTAssertEqual(options.first?.label, "Agent default")
        XCTAssertEqual(
            options.first(where: { $0.id == "claude-opus-5" })?.label,
            "Claude Opus 5"
        )
    }

    func testModelPickerAppendsAnUnadvertisedRunningModel() throws {
        let providerModels = try decodeProviderModels(defaultIsFirstModelProviderJSON)

        let options = GaryxThreadModelOverridePresentation.modelPickerOptions(
            providerModels: providerModels,
            effectiveModel: "claude-retired-9",
            defaultRowLabel: "Agent default"
        )

        XCTAssertEqual(options.map(\.id), ["", "claude-opus-5", "claude-sonnet-5", "claude-retired-9"])
        XCTAssertEqual(options.last?.label, "claude-retired-9")
    }

    func testModelPickerHasNoRowsWithoutAdvertisedModels() {
        XCTAssertTrue(
            GaryxThreadModelOverridePresentation.modelPickerOptions(
                providerModels: nil,
                effectiveModel: "claude-opus-5",
                defaultRowLabel: "Agent default"
            ).isEmpty
        )
    }

    /// Same contract for thinking levels: the model's default effort keeps its
    /// own row, so choosing it pins that level instead of clearing the cell.
    func testReasoningEffortPickerKeepsARealRowForTheDefaultLevel() throws {
        let providerModels = try decodeProviderModels(configuredClaudeProviderJSON)

        let options = GaryxThreadModelOverridePresentation.reasoningEffortPickerOptions(
            providerModels: providerModels,
            model: nil,
            effectiveReasoningEffort: "max",
            defaultRowLabel: "Agent default"
        )

        XCTAssertEqual(options.first?.label, "Agent default")
        XCTAssertEqual(options.map(\.id), ["", "low", "high", "max"])
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

    /// Shape the live gateway returns for `claude_code`: discovery advertises
    /// the newest models and `default_model` names one of them, so the default
    /// and a real catalog row are the same value.
    private let defaultIsFirstModelProviderJSON = """
    {
        "provider_type": "claude_code",
        "supports_model_selection": true,
        "supports_reasoning_effort_selection": true,
        "default_model": "claude-opus-5",
        "source": "claude_code_discovery",
        "reasoning_efforts": [
            { "id": "low", "label": "Low", "recommended": false },
            { "id": "high", "label": "High", "recommended": true }
        ],
        "models": [
            {
                "id": "claude-opus-5",
                "label": "Claude Opus 5",
                "recommended": true,
                "supported_reasoning_efforts": [
                    { "id": "low", "label": "Low", "recommended": false },
                    { "id": "high", "label": "High", "recommended": true },
                    { "id": "max", "label": "Max", "recommended": false }
                ]
            },
            {
                "id": "claude-sonnet-5",
                "label": "Claude Sonnet 5",
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

    private let unsupportedProviderJSON = """
    {
        "provider_type": "unsupported_provider",
        "supports_model_selection": false,
        "source": "provider",
        "models": []
    }
    """
}
