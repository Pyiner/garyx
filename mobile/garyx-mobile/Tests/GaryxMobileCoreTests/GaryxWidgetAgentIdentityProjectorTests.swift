import XCTest
@testable import GaryxMobileCore

final class GaryxWidgetAgentIdentityProjectorTests: XCTestCase {
    func testTeamThreadUsesTeamCatalogIdentity() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(teamId: "team-1", teamName: "Fallback Team"),
            agents: [agent(id: "agent-1")],
            teams: [team(id: "team-1", displayName: "Review Team", avatarDataUrl: "data:image/png;base64,abc")]
        )
        XCTAssertEqual(
            identity,
            GaryxWidgetAgentIdentity(
                id: "team-1",
                name: "Review Team",
                avatarDataUrl: "data:image/png;base64,abc",
                providerType: nil,
                isTeam: true,
                builtIn: false
            )
        )
    }

    func testUnknownTeamFallsBackToThreadTeamName() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(teamId: "team-x", teamName: "Ghost Team"),
            agents: [],
            teams: [team(id: "team-1", displayName: "Review Team")]
        )
        XCTAssertEqual(
            identity,
            GaryxWidgetAgentIdentity(
                id: "team-x",
                name: "Ghost Team",
                avatarDataUrl: nil,
                providerType: nil,
                isTeam: true,
                builtIn: false
            )
        )
    }

    func testAgentThreadUsesAgentCatalogIdentity() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(agentId: "agent-1", providerType: "thread-provider"),
            agents: [
                agent(
                    id: "agent-1",
                    displayName: "Coder",
                    providerType: "claude_code",
                    avatarDataUrl: "data:image/png;base64,def",
                    builtIn: true
                ),
            ],
            teams: []
        )
        XCTAssertEqual(
            identity,
            GaryxWidgetAgentIdentity(
                id: "agent-1",
                name: "Coder",
                avatarDataUrl: "data:image/png;base64,def",
                providerType: "claude_code",
                isTeam: false,
                builtIn: true
            )
        )
    }

    func testUnknownAgentFallsBackToThreadProviderType() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(agentId: "agent-x", providerType: "codex"),
            agents: [agent(id: "agent-1")],
            teams: []
        )
        XCTAssertEqual(
            identity,
            GaryxWidgetAgentIdentity(
                id: "agent-x",
                name: nil,
                avatarDataUrl: nil,
                providerType: "codex",
                isTeam: false,
                builtIn: false
            )
        )
    }

    func testNoTeamOrAgentYieldsProviderOnlyIdentity() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(providerType: "gemini"),
            agents: [agent(id: "agent-1")],
            teams: [team(id: "team-1", displayName: "Review Team")]
        )
        XCTAssertEqual(
            identity,
            GaryxWidgetAgentIdentity(
                id: nil,
                name: nil,
                avatarDataUrl: nil,
                providerType: "gemini",
                isTeam: false,
                builtIn: false
            )
        )
    }

    func testEmptyCatalogAvatarBecomesNil() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(teamId: "team-1"),
            agents: [],
            teams: [team(id: "team-1", displayName: "Review Team", avatarDataUrl: "")]
        )
        XCTAssertNil(identity.avatarDataUrl)
    }

    func testWhitespaceTeamIdFallsThroughToAgent() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(agentId: "agent-1", teamId: "   "),
            agents: [agent(id: "agent-1", displayName: "Coder")],
            teams: []
        )
        XCTAssertEqual(identity.id, "agent-1")
        XCTAssertFalse(identity.isTeam)
    }

    func testDictionaryVariantMatchesArrayVariant() {
        let agents = [agent(id: "agent-1", displayName: "Coder"), agent(id: "agent-2")]
        let teams = [team(id: "team-1", displayName: "Review Team")]
        let agentsById = Dictionary(uniqueKeysWithValues: agents.map { ($0.id, $0) })
        let teamsById = Dictionary(uniqueKeysWithValues: teams.map { ($0.id, $0) })
        for subject in [
            thread(teamId: "team-1"),
            thread(agentId: "agent-1"),
            thread(agentId: "agent-x", providerType: "codex"),
            thread(providerType: "claude_code"),
        ] {
            XCTAssertEqual(
                GaryxWidgetAgentIdentityProjector.identity(for: subject, agents: agents, teams: teams),
                GaryxWidgetAgentIdentityProjector.identity(for: subject, agentsById: agentsById, teamsById: teamsById)
            )
        }
    }

    private func thread(
        agentId: String? = nil,
        teamId: String? = nil,
        teamName: String? = nil,
        providerType: String? = nil
    ) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: "thread-1",
            title: "Thread",
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: agentId,
            teamId: teamId,
            teamName: teamName,
            providerType: providerType,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }

    private func agent(
        id: String,
        displayName: String = "Agent",
        providerType: String = "claude_code",
        avatarDataUrl: String = "",
        builtIn: Bool = false
    ) -> GaryxAgentSummary {
        GaryxAgentSummary(
            id: id,
            displayName: displayName,
            providerType: providerType,
            model: "",
            avatarDataUrl: avatarDataUrl,
            builtIn: builtIn
        )
    }

    private func team(
        id: String,
        displayName: String,
        avatarDataUrl: String = ""
    ) -> GaryxTeamSummary {
        GaryxTeamSummary(
            id: id,
            displayName: displayName,
            leaderAgentId: "",
            memberAgentIds: [],
            avatarDataUrl: avatarDataUrl
        )
    }
}
