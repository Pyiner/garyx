import XCTest
@testable import GaryxMobileCore

final class GaryxMobileAgentTargetModelTests: XCTestCase {
    func testMakeTargetsKeepsStandaloneAgentsAndTeams() throws {
        let agents = try decodeAgents(
            """
            {
              "agents": [
                {
                  "agent_id": "codex",
                  "display_name": "Codex",
                  "provider_type": "codex",
                  "avatar_data_url": "data:image/png;base64,Y29kZXg=",
                  "built_in": true,
                  "standalone": true
                },
                {
                  "agent_id": "embedded",
                  "display_name": "Embedded",
                  "standalone": false
                }
              ]
            }
            """
        )
        let teams = try decodeTeams(
            """
            {
              "teams": [
                {
                  "team_id": "review-team",
                  "display_name": "Review Team",
                  "member_agent_ids": ["codex", "claude"],
                  "avatar_data_url": "data:image/png;base64,dGVhbQ=="
                }
              ]
            }
            """
        )

        let targets = GaryxMobileAgentTargetMapper.makeTargets(agents: agents, teams: teams)

        XCTAssertEqual(targets.map(\.id), ["codex", "review-team"])
        XCTAssertEqual(targets[0].kind, .agent)
        XCTAssertEqual(targets[0].title, "Codex")
        XCTAssertEqual(targets[0].providerType, "codex")
        XCTAssertTrue(targets[0].builtIn)
        XCTAssertEqual(targets[1].kind, .team)
        XCTAssertEqual(targets[1].subtitle, "2 agents")
    }

    func testSelectedThreadTargetPrefersTeamOverAgent() throws {
        let agents = try decodeAgents(
            """
            {
              "agents": [
                { "agent_id": "codex", "display_name": "Codex", "standalone": true }
              ]
            }
            """
        )
        let teams = try decodeTeams(
            """
            {
              "teams": [
                { "team_id": "review-team", "display_name": "Review Team", "member_agent_ids": ["codex"] }
              ]
            }
            """
        )
        let thread = try decodeThread(
            """
            {
              "id": "thread-1",
              "title": "Architecture review",
              "agent_id": "codex",
              "team_id": "review-team"
            }
            """
        )
        let targets = GaryxMobileAgentTargetMapper.makeTargets(agents: agents, teams: teams)

        let target = GaryxMobileAgentTargetMapper.selectedThreadTarget(
            thread: thread,
            selectedAgentTargetId: "codex",
            targets: targets
        )

        XCTAssertEqual(target?.id, "review-team")
        XCTAssertEqual(
            GaryxMobileAgentTargetMapper.selectedThreadAgentLabel(
                thread: thread,
                target: target,
                fallbackSelectedAgentLabel: "Codex"
            ),
            "Review Team"
        )
    }

    func testSelectedThreadLabelFallsBackToThreadMetadata() throws {
        let threadWithTeamName = try decodeThread(
            """
            {
              "id": "thread-2",
              "title": "Planning",
              "team_display_name": "Planning Team",
              "agent_id": "missing-agent"
            }
            """
        )
        let threadWithAgent = try decodeThread(
            """
            {
              "id": "thread-3",
              "title": "Follow up",
              "agent_id": "missing-agent"
            }
            """
        )

        XCTAssertEqual(
            GaryxMobileAgentTargetMapper.selectedThreadAgentLabel(
                thread: threadWithTeamName,
                target: nil,
                fallbackSelectedAgentLabel: "Codex"
            ),
            "Planning Team"
        )
        XCTAssertEqual(
            GaryxMobileAgentTargetMapper.selectedThreadAgentLabel(
                thread: threadWithAgent,
                target: nil,
                fallbackSelectedAgentLabel: "Codex"
            ),
            "missing-agent"
        )
        XCTAssertEqual(
            GaryxMobileAgentTargetMapper.selectedThreadAgentLabel(
                thread: nil,
                target: nil,
                fallbackSelectedAgentLabel: "Codex"
            ),
            "Codex"
        )
    }

    private func decodeAgents(_ json: String) throws -> [GaryxAgentSummary] {
        try JSONDecoder().decode(GaryxAgentsPage.self, from: Data(json.utf8)).agents
    }

    private func decodeTeams(_ json: String) throws -> [GaryxTeamSummary] {
        try JSONDecoder().decode(GaryxTeamsPage.self, from: Data(json.utf8)).teams
    }

    private func decodeThread(_ json: String) throws -> GaryxThreadSummary {
        try JSONDecoder().decode(GaryxThreadSummary.self, from: Data(json.utf8))
    }
}

final class GaryxMobileNavigationStateTests: XCTestCase {
    func testOpeningPanelFromHomeStartsFreshContentStack() {
        var state = GaryxMobileNavigationState()
        XCTAssertFalse(state.presentsContent)
        XCTAssertEqual(state.rootNavigationPath, [])

        state.openPanel(.automations, dreamsAutoScanEnabled: true, source: .current)

        XCTAssertEqual(state.activePanel, .automations)
        XCTAssertTrue(state.presentsContent)
        XCTAssertEqual(state.rootNavigationPath, [.panel(.automations)])
        // The home list is not a back-stack entry; back means pop to home.
        XCTAssertTrue(state.mainPanelBackStack.isEmpty)
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
    }

    func testCurrentPanelNavigationPushesPreviousPresentedRoute() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.tasks, dreamsAutoScanEnabled: true, source: .current)

        state.openPanel(.automations, dreamsAutoScanEnabled: true, source: .current)

        XCTAssertEqual(state.activePanel, .automations)
        XCTAssertEqual(state.mainPanelBackStack, [GaryxMobilePanelRoute(panel: .tasks, settingsTab: .manage)])
        XCTAssertEqual(state.leadingEdgeAction, .mainPanelBack)
    }

    func testPopToHomeResetsContentState() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.workspaceBots, dreamsAutoScanEnabled: true, source: .current)
        state.setWorkspaceBotsDrilldown(.bot("bot-1"))

        state.popToHome()

        XCTAssertFalse(state.presentsContent)
        XCTAssertEqual(state.rootNavigationPath, [])
        XCTAssertTrue(state.mainPanelBackStack.isEmpty)
        XCTAssertNil(state.workspaceBotsDrilldown)
        XCTAssertEqual(state.leadingEdgeAction, .openSidebar)
    }

    func testChatPanelPresentsConversationRoute() {
        var state = GaryxMobileNavigationState()

        state.setActivePanel(.chat)

        XCTAssertTrue(state.presentsContent)
        XCTAssertEqual(state.rootNavigationPath, [.conversation])
    }

    func testSidebarNavigationClearsPreviousRouteStack() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.tasks, dreamsAutoScanEnabled: true, source: .current)

        state.openPanel(.automations, dreamsAutoScanEnabled: true, source: .sidebar)

        XCTAssertEqual(state.activePanel, .automations)
        XCTAssertTrue(state.mainPanelBackStack.isEmpty)
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
    }

    func testLeadingEdgePrioritizesLocalDrilldowns() {
        var state = GaryxMobileNavigationState()

        state.openSettings(tab: .provider, source: .current)
        XCTAssertEqual(state.leadingEdgeAction, .settingsOverview)
        state.showSettingsOverview()
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)

        state.openPanel(.workspaceBots, dreamsAutoScanEnabled: true, source: .replace)
        state.setWorkspaceBotsDrilldown(.bot("agent-1"))
        XCTAssertEqual(state.leadingEdgeAction, .workspaceBotsOverview)
        state.showWorkspaceBotsOverview()
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
    }

    func testDirectPanelMutationClearsStackAndWorkspaceDrilldown() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.workspaceBots, dreamsAutoScanEnabled: true, source: .current)
        state.setWorkspaceBotsDrilldown(.workspace("/workspace"))

        state.setActivePanel(.chat)

        XCTAssertEqual(state.activePanel, .chat)
        XCTAssertTrue(state.mainPanelBackStack.isEmpty)
        XCTAssertNil(state.workspaceBotsDrilldown)
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
    }

    func testWorkspaceBotsDrilldownRoutePersistsInNavigationState() {
        var state = GaryxMobileNavigationState()

        state.openPanel(.workspaceBots, dreamsAutoScanEnabled: true, source: .replace)
        state.setWorkspaceBotsDrilldown(.workspace("/workspace"))

        XCTAssertEqual(state.workspaceBotsDrilldown, .workspace("/workspace"))
        XCTAssertEqual(state.leadingEdgeAction, .workspaceBotsOverview)

        state.openPanel(.automations, dreamsAutoScanEnabled: true, source: .current)

        XCTAssertNil(state.workspaceBotsDrilldown)
        XCTAssertTrue(state.goBackInMainPanel())
        XCTAssertEqual(state.activePanel, .workspaceBots)
        XCTAssertEqual(state.workspaceBotsDrilldown, .workspace("/workspace"))
    }

    func testExplicitWorkspaceFilesRouteKeepsWorkspacesPanel() {
        var state = GaryxMobileNavigationState()

        state.openRoute(
            GaryxMobilePanelRoute(panel: .workspaces, settingsTab: .manage),
            source: .replace
        )

        XCTAssertEqual(state.activePanel, .workspaces)
        XCTAssertNil(state.workspaceBotsDrilldown)
    }
}
