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
    func testOpeningPanelFromHomeStartsFreshContentStack() {
        var state = GaryxMobileNavigationState()
        XCTAssertFalse(state.presentsContent)
        XCTAssertEqual(state.rootNavigationPath, [])

        state.openPanel(.automations, source: .current)

        XCTAssertEqual(state.activePanel, .automations)
        XCTAssertTrue(state.presentsContent)
        XCTAssertEqual(state.rootNavigationPath, [.panel(.automations)])
        // The home list is not a back-stack entry; back means pop to home.
        XCTAssertTrue(state.mainPanelBackStack.isEmpty)
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
    }

    func testCurrentPanelNavigationPushesPreviousPresentedRoute() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.skills, source: .current)

        state.openPanel(.automations, source: .current)

        XCTAssertEqual(state.activePanel, .automations)
        XCTAssertEqual(state.mainPanelBackStack, [GaryxMobilePanelRoute(panel: .skills, settingsTab: .manage)])
        XCTAssertEqual(state.leadingEdgeAction, .mainPanelBack)
    }

    func testPopToHomeResetsContentState() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.workspaceBots, source: .current)
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

    func testConversationOpenedFromCurrentPanelReturnsToThatPanel() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.skills, source: .current)

        state.openConversation(source: .current)

        XCTAssertEqual(state.activePanel, .chat)
        XCTAssertEqual(state.rootNavigationPath, [.conversation])
        XCTAssertEqual(state.mainPanelBackStack, [GaryxMobilePanelRoute(panel: .skills, settingsTab: .manage)])
        XCTAssertEqual(state.leadingEdgeAction, .mainPanelBack)
        XCTAssertTrue(state.goBackInMainPanel())
        XCTAssertEqual(state.activePanel, .skills)
        XCTAssertEqual(state.rootNavigationPath, [.panel(.skills)])
    }

    func testSidebarNavigationClearsPreviousRouteStack() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.skills, source: .current)

        state.openPanel(.automations, source: .sidebar)

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

        state.openPanel(.workspaceBots, source: .replace)
        state.setWorkspaceBotsDrilldown(.bot("agent-1"))
        // Drilldowns opened from the drawer have no back stack; back pops
        // straight home instead of surfacing the overview list.
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
        state.showWorkspaceBotsOverview()
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
    }

    func testDrilldownOpenedFromPageGoesBackToThatPage() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.automations, source: .sidebar)

        state.openRoute(
            GaryxMobilePanelRoute(
                panel: .workspaceBots,
                settingsTab: .manage,
                workspaceBotsDrilldown: .automationThreads("auto-1")
            ),
            source: .current
        )

        XCTAssertEqual(state.leadingEdgeAction, .mainPanelBack)
        XCTAssertTrue(state.goBackInMainPanel())
        XCTAssertEqual(state.activePanel, .automations)
    }

    func testDirectPanelMutationClearsStackAndWorkspaceDrilldown() {
        var state = GaryxMobileNavigationState()
        state.openPanel(.workspaceBots, source: .current)
        state.setWorkspaceBotsDrilldown(.workspace("/workspace"))

        state.setActivePanel(.chat)

        XCTAssertEqual(state.activePanel, .chat)
        XCTAssertTrue(state.mainPanelBackStack.isEmpty)
        XCTAssertNil(state.workspaceBotsDrilldown)
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)
    }

    func testWorkspaceBotsDrilldownRoutePersistsInNavigationState() {
        var state = GaryxMobileNavigationState()

        state.openPanel(.workspaceBots, source: .replace)
        state.setWorkspaceBotsDrilldown(.workspace("/workspace"))

        XCTAssertEqual(state.workspaceBotsDrilldown, .workspace("/workspace"))
        XCTAssertEqual(state.leadingEdgeAction, .popToHome)

        state.openPanel(.automations, source: .current)

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
