import XCTest
@testable import GaryxMobileCore

final class GaryxCapsuleCardPresentationTests: XCTestCase {
    // MARK: - Chat-card presentation

    func testChatCardSubtitle() {
        XCTAssertEqual(GaryxCapsuleChatCardPresentation.subtitle(action: .created), "Created")
        XCTAssertEqual(GaryxCapsuleChatCardPresentation.subtitle(action: .updated), "Updated")
    }

    // MARK: - Gallery-card presentation (creator precedence + subline join)

    private func makeAgent(id: String, displayName: String) -> GaryxAgentSummary {
        GaryxAgentSummary(id: id, displayName: displayName, providerType: "claude_code", model: "")
    }

    private func makeTeam(id: String, displayName: String) -> GaryxTeamSummary {
        GaryxTeamSummary(id: id, displayName: displayName, leaderAgentId: "", memberAgentIds: [])
    }

    func testCreatorPrefersAgentDisplayName() {
        let agents = [makeAgent(id: "agent-1000000001", displayName: "Test Agent")]
        let teams = [makeTeam(id: "agent-1000000001", displayName: "Test Team")]
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "agent-1000000001",
                providerType: "claude_code",
                agents: agents,
                teams: teams
            ),
            "Test Agent",
            "agent display name wins over team and provider"
        )
    }

    func testCreatorFallsBackToTeamWhenAgentMisses() {
        // The team tier is the iOS-only fallback that desktop describeCreator lacks.
        let teams = [makeTeam(id: "team-1000000002", displayName: "Test Team")]
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "team-1000000002",
                providerType: "claude_code",
                agents: [],
                teams: teams
            ),
            "Test Team",
            "team display name resolves when no agent matches"
        )
    }

    func testCreatorFallsBackToAgentIdWhenCatalogMisses() {
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "agent-1000000003",
                providerType: "claude_code",
                agents: [],
                teams: []
            ),
            "agent-1000000003",
            "a present agentId is shown raw when neither catalog resolves it"
        )
    }

    func testCreatorPrettifiesProviderWhenNoAgentId() {
        // Pin the prettified provider fallback contract.
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: nil,
                providerType: "claude_code",
                agents: [],
                teams: []
            ),
            "Claude Code"
        )
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "   ",
                providerType: "codex_app_server",
                agents: [],
                teams: []
            ),
            "Codex",
            "blank agentId is treated as absent"
        )
    }

    func testCreatorEmptyEverythingIsAgent() {
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: nil,
                providerType: nil,
                agents: [],
                teams: []
            ),
            "Agent"
        )
    }

    func testCreatorSkipsBlankAgentDisplayName() {
        let agents = [makeAgent(id: "agent-1000000004", displayName: "   ")]
        let teams = [makeTeam(id: "agent-1000000004", displayName: "Test Team")]
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "agent-1000000004",
                providerType: "claude_code",
                agents: agents,
                teams: teams
            ),
            "Test Team",
            "a blank agent display name falls through to the team tier"
        )
    }

    func testSublineJoinsTimeAndCreator() {
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.subline(timeDisplay: "9m", creator: "Test Agent"),
            "9m · Test Agent"
        )
    }

    func testSublineCreatorOnlyWhenTimeMissing() {
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.subline(timeDisplay: nil, creator: "Test Agent"),
            "Test Agent"
        )
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.subline(timeDisplay: "  ", creator: "Test Agent"),
            "Test Agent",
            "whitespace time is treated as missing (no dangling separator)"
        )
    }
}
