import XCTest
@testable import GaryxMobileCore

final class GaryxCapsulePreviewLoadPlannerTests: XCTestCase {
    // MARK: - Gallery planner (visibility FIFO)

    func testEmptyPlannerHasNoActiveIds() {
        let planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        XCTAssertEqual(planner.activeIds, [])
        XCTAssertFalse(planner.isActive("a"))
    }

    func testAdmitsFirstNVisibleInAppearanceOrder() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        XCTAssertTrue(planner.markVisible("a"))
        XCTAssertTrue(planner.markVisible("b"))
        XCTAssertTrue(planner.markVisible("c"))
        XCTAssertEqual(planner.activeIds, ["a", "b"])
        XCTAssertTrue(planner.isActive("a"))
        XCTAssertTrue(planner.isActive("b"))
        XCTAssertFalse(planner.isActive("c"))
    }

    func testHidingAnActiveFreesSlotForNextVisible() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        XCTAssertTrue(planner.markHidden("a"))
        XCTAssertEqual(planner.activeIds, ["b", "c"])
        XCTAssertFalse(planner.isActive("a"))
        XCTAssertTrue(planner.isActive("c"))
    }

    func testIPadCapAdmitsAll() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 4)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        XCTAssertEqual(planner.activeIds, ["a", "b", "c"])
    }

    func testMarkVisibleIsIdempotentAndDoesNotReorder() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 1)
        XCTAssertTrue(planner.markVisible("a"))
        XCTAssertTrue(planner.markVisible("b"))
        XCTAssertFalse(planner.markVisible("a"), "re-marking a visible id is a no-op")
        XCTAssertEqual(planner.visibleIds, ["a", "b"], "order is preserved")
        XCTAssertEqual(planner.activeIds, ["a"])
    }

    func testSetMaxActiveRecomputesAdmission() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 1)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        XCTAssertEqual(planner.activeIds, ["a"])
        planner.setMaxActive(3)
        XCTAssertEqual(planner.activeIds, ["a", "b", "c"])
        planner.setMaxActive(0)
        XCTAssertEqual(planner.activeIds, [])
    }

    func testPruneDropsInvalidVisibleIdsKeepingOrder() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        planner.prune(keeping: ["a", "c"])
        XCTAssertEqual(planner.visibleIds, ["a", "c"])
        XCTAssertEqual(planner.activeIds, ["a", "c"])
    }

    // MARK: - Chat-card admission (conversation-level, most-recent N)

    func testChatAdmissionEmpty() {
        XCTAssertEqual(GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: [], maxActive: 2), [])
    }

    func testChatAdmissionKeepsMostRecentN() {
        // Transcript order is oldest-first; the newest cards (tail) are admitted
        // because the transcript opens scrolled to the bottom.
        let keys = ["t1:a", "t2:b", "t3:c"]
        XCTAssertEqual(
            GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: keys, maxActive: 2),
            ["t2:b", "t3:c"]
        )
    }

    func testChatAdmissionAdmitsAllWhenCapExceedsCount() {
        let keys = ["t1:a", "t2:b"]
        XCTAssertEqual(GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: keys, maxActive: 4), keys)
    }

    func testChatAdmissionZeroCapAdmitsNone() {
        XCTAssertEqual(
            GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: ["t1:a"], maxActive: 0),
            []
        )
    }

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
        // The team tier is the iOS-only档 that desktop describeCreator lacks.
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
