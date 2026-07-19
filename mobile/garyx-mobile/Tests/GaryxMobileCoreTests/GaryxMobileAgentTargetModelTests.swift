import XCTest
@testable import GaryxMobileCore

final class GaryxMobileAgentTargetModelTests: XCTestCase {
    func testTargetsIncludeOnlyEnabledStandaloneAgents() throws {
        let agents = try JSONDecoder().decode(GaryxAgentsPage.self, from: Data(#"{"agents":[{"agent_id":"codex","display_name":"Codex","provider_type":"codex","built_in":true,"standalone":true,"enabled":true},{"agent_id":"disabled","display_name":"Disabled","standalone":true,"enabled":false},{"agent_id":"embedded","display_name":"Embedded","standalone":false,"enabled":true}]}"#.utf8)).agents
        let targets = GaryxMobileAgentTargetMapper.makeTargets(agents: agents)
        XCTAssertEqual(targets.map(\.id), ["codex"])
        XCTAssertEqual(targets.first?.title, "Codex")
        XCTAssertTrue(targets.first?.builtIn == true)
    }

    func testSelectedThreadUsesAgentIdentityAndFallback() throws {
        let agent = GaryxAgentSummary(id: "codex", displayName: "Codex", providerType: "codex", model: "")
        let thread = try JSONDecoder().decode(GaryxThreadSummary.self, from: Data(#"{"id":"thread-1","title":"Review","agent_id":"codex"}"#.utf8))
        let targets = GaryxMobileAgentTargetMapper.makeTargets(agents: [agent])
        let target = GaryxMobileAgentTargetMapper.selectedThreadTarget(thread: thread, selectedAgentTargetId: "", targets: targets)
        XCTAssertEqual(target?.id, "codex")
        XCTAssertEqual(GaryxMobileAgentTargetMapper.selectedThreadAgentLabel(thread: thread, target: target, fallbackSelectedAgentLabel: "Agent"), "Codex")
    }
}

final class GaryxMobileNavigationStateTests: XCTestCase {
    func testProjectionReadsOnlyCanonicalTop() {
        let state = GaryxMobileNavigationState(projecting: [
            entry("skills", .panel(GaryxMobilePanel.skills.rawValue)),
            entry("automation", .panel(GaryxMobilePanel.automations.rawValue)),
        ])

        XCTAssertTrue(state.presentsContent)
        XCTAssertEqual(state.activePanel, .automations)
        XCTAssertEqual(state.activeSettingsTab, .manage)
        XCTAssertNil(state.workspaceBotsDrilldown)
    }

    func testConversationOpenedFromCurrentPanelReturnsToThatPanel() {
        var route = GaryxCanonicalRouteState(path: [
            entry("skills", .panel(GaryxMobilePanel.skills.rawValue)),
        ])
        _ = route.open(entry("conversation-a", .conversation(threadID: "A")))
        XCTAssertEqual(
            GaryxMobileNavigationState(projecting: route.path).activePanel,
            .chat
        )

        _ = route.pop()
        XCTAssertEqual(
            GaryxMobileNavigationState(projecting: route.path).activePanel,
            .skills
        )
    }

    func testSettingsAndDrilldownProjectionCarryTypedPayload() {
        let settings = GaryxMobileNavigationState(projecting: [
            entry("settings", .settingsDetail(GaryxMobileSettingsTab.provider.rawValue)),
        ])
        XCTAssertEqual(settings.activePanel, .settings)
        XCTAssertEqual(settings.activeSettingsTab, .provider)

        let drilldown = GaryxMobileNavigationState(projecting: [
            entry(
                "workspace",
                .workspaceDrilldown(.workspace(path: "/workspace"))
            ),
        ])
        XCTAssertEqual(drilldown.activePanel, .workspaceBots)
        XCTAssertEqual(drilldown.workspaceBotsDrilldown, .workspace("/workspace"))
    }

    func testHomeProjectionHasNoContent() {
        let state = GaryxMobileNavigationState(projecting: [])
        XCTAssertFalse(state.presentsContent)
        XCTAssertEqual(state.activePanel, .chat)
        XCTAssertEqual(state.activeSettingsTab, .manage)
        XCTAssertNil(state.workspaceBotsDrilldown)
    }

    private func entry(
        _ id: String,
        _ destination: GaryxRouteDestination
    ) -> GaryxRouteEntry {
        GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: id),
            destination: destination
        )
    }
}
