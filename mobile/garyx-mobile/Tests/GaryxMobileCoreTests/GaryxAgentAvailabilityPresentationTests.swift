import XCTest
@testable import GaryxMobileCore

final class GaryxAgentAvailabilityPresentationTests: XCTestCase {
    func testDefaultBadgeCoversRawInactiveActingAndAutoStates() {
        XCTAssertEqual(
            GaryxAgentAvailabilityPresentation.defaultBadge(
                agentId: "codex",
                enabled: true,
                defaultAgentId: "codex",
                effectiveDefaultAgentId: "codex"
            ),
            .default
        )
        XCTAssertEqual(
            GaryxAgentAvailabilityPresentation.defaultBadge(
                agentId: "codex",
                enabled: false,
                defaultAgentId: "codex",
                effectiveDefaultAgentId: "claude"
            ),
            .defaultInactive
        )
        XCTAssertEqual(
            GaryxAgentAvailabilityPresentation.defaultBadge(
                agentId: "claude",
                enabled: true,
                defaultAgentId: "codex",
                effectiveDefaultAgentId: "claude"
            ),
            .actingDefault
        )
        XCTAssertEqual(
            GaryxAgentAvailabilityPresentation.defaultBadge(
                agentId: "claude",
                enabled: true,
                defaultAgentId: nil,
                effectiveDefaultAgentId: "claude"
            ),
            .defaultAuto
        )
        XCTAssertNil(
            GaryxAgentAvailabilityPresentation.defaultBadge(
                agentId: "claude",
                enabled: false,
                defaultAgentId: nil,
                effectiveDefaultAgentId: nil
            )
        )
        XCTAssertTrue(
            GaryxAgentAvailabilityPresentation.allowsNewBindingActions(
                enabled: true,
                standalone: true
            )
        )
        XCTAssertFalse(
            GaryxAgentAvailabilityPresentation.allowsNewBindingActions(
                enabled: false,
                standalone: true
            )
        )
        XCTAssertFalse(
            GaryxAgentAvailabilityPresentation.allowsNewBindingActions(
                enabled: true,
                standalone: false
            )
        )
    }

    func testNewThreadSelectionUsesEffectiveDefaultAndNeverFallsBackExplicitOverride() {
        XCTAssertEqual(
            GaryxNewThreadAgentSelection.agentId(
                draftOverrideAgentId: nil,
                effectiveDefaultAgentId: "codex"
            ),
            "codex"
        )
        XCTAssertEqual(
            GaryxNewThreadAgentSelection.agentId(
                draftOverrideAgentId: " disabled-agent ",
                effectiveDefaultAgentId: "codex"
            ),
            "disabled-agent"
        )
        XCTAssertFalse(
            GaryxNewThreadAgentSelection.isAvailable(
                draftOverrideAgentId: "disabled-agent",
                effectiveDefaultAgentId: "codex",
                enabledAgentIds: ["codex"]
            )
        )
        XCTAssertFalse(
            GaryxNewThreadAgentSelection.isAvailable(
                draftOverrideAgentId: nil,
                effectiveDefaultAgentId: nil,
                enabledAgentIds: []
            )
        )
    }

    func testBotPickerStartsWithFollowGlobalAndKeepsUnavailableConfiguredAgentRepairable() {
        let targets = [
            target(id: "claude", title: "Claude"),
            target(id: "codex", title: "Codex"),
        ]
        let options = GaryxBotAgentPickerPresentation.makeOptions(
            targets: targets,
            effectiveDefaultAgentId: "codex",
            configuredAgentId: "disabled-agent"
        )

        XCTAssertEqual(options.first?.selection, .followGlobal)
        XCTAssertEqual(options.first?.title, "Follow global default (currently Codex)")
        XCTAssertFalse(options.first?.isRecommended ?? true)
        XCTAssertEqual(
            GaryxBotAgentPickerPresentation.selection(configuredAgentId: nil),
            .followGlobal
        )
        XCTAssertEqual(
            GaryxBotAgentPickerPresentation.selection(configuredAgentId: " disabled-agent "),
            .agent("disabled-agent")
        )
        XCTAssertEqual(options.dropFirst().first?.selection, .agent("disabled-agent"))
        XCTAssertFalse(options.dropFirst().first?.isAvailable ?? true)
        XCTAssertEqual(options.compactMap(\.target).map(\.id), ["codex", "claude"])
        XCTAssertEqual(options.first(where: \.isRecommended)?.selection, .agent("codex"))
        XCTAssertEqual(
            GaryxBotAgentPickerPresentation.preferredConfiguredAgentId(
                targets: targets,
                effectiveDefaultAgentId: " codex "
            ),
            "codex"
        )
    }

    func testBotPickerFollowGlobalSurvivesAllDisabled() {
        let options = GaryxBotAgentPickerPresentation.makeOptions(
            targets: [],
            effectiveDefaultAgentId: nil,
            configuredAgentId: nil
        )
        XCTAssertEqual(options.map(\.selection), [.followGlobal])
        XCTAssertEqual(options.first?.title, "Follow global default (currently no enabled agent)")
        XCTAssertNil(
            GaryxBotAgentPickerPresentation.preferredConfiguredAgentId(
                targets: [],
                effectiveDefaultAgentId: "codex"
            )
        )
    }

    func testAgentCatalogAndBotWireDecodeAvailabilityFields() throws {
        let page = try JSONDecoder().decode(
            GaryxAgentsPage.self,
            from: Data(
                #"{"agents":[{"agent_id":"claude","enabled":false},{"agent_id":"codex"}],"default_agent_id":"claude","effective_default_agent_id":"codex"}"#.utf8
            )
        )
        XCTAssertEqual(page.defaultAgentId, "claude")
        XCTAssertEqual(page.effectiveDefaultAgentId, "codex")
        XCTAssertFalse(page.agents[0].enabled)
        XCTAssertTrue(page.agents[1].enabled, "missing enabled remains true for legacy/cache decoding")

        let bots = try JSONDecoder().decode(
            GaryxConfiguredBotsPage.self,
            from: Data(
                #"{"bots":[{"channel":"telegram","account_id":"main","agent_id":null,"effective_agent_id":"codex"}]}"#.utf8
            )
        )
        XCTAssertNil(bots.bots.first?.agentId)
        XCTAssertEqual(bots.bots.first?.effectiveAgentId, "codex")
    }

    func testCustomAgentRequestEnabledIsTriStateOnWire() throws {
        let encoder = JSONEncoder()
        let omitted = try XCTUnwrap(
            try JSONSerialization.jsonObject(
                with: encoder.encode(
                    GaryxCustomAgentRequest(
                        agentId: "agent-test",
                        displayName: "Test Agent",
                        providerType: "codex"
                    )
                )
            ) as? [String: Any]
        )
        XCTAssertNil(omitted["enabled"])

        let disabled = try XCTUnwrap(
            try JSONSerialization.jsonObject(
                with: encoder.encode(
                    GaryxCustomAgentRequest(
                        agentId: "agent-test",
                        displayName: "Test Agent",
                        providerType: "codex",
                        enabled: false
                    )
                )
            ) as? [String: Any]
        )
        XCTAssertEqual(disabled["enabled"] as? Bool, false)
    }

    func testTargetAgentLabelUsesTypedResolutionWithoutSentinels() {
        let agents = [
            GaryxAgentSummary(
                id: "codex",
                displayName: "Codex",
                providerType: "codex",
                model: ""
            ),
        ]
        XCTAssertEqual(
            GaryxAutomationAgentPresentation.followsThreadLabel(
                resolution: .followThread,
                effectiveAgentId: "codex",
                agents: agents
            ),
            "Follows target thread · Codex"
        )
        XCTAssertEqual(
            GaryxAutomationAgentPresentation.followsThreadLabel(
                resolution: .targetMissing,
                effectiveAgentId: nil,
                agents: agents
            ),
            "Follows target thread · target unavailable"
        )
    }

    private func target(id: String, title: String) -> GaryxMobileAgentTarget {
        GaryxMobileAgentTarget(
            id: id,
            title: title,
            subtitle: "",
            avatarDataUrl: "",
            providerType: id,
            builtIn: true
        )
    }
}
