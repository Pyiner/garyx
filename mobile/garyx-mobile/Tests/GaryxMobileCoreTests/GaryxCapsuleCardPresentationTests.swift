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

    func testCreatorUsesAgentDisplayName() {
        let agents = [makeAgent(id: "agent-1000000001", displayName: "Test Agent")]
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "agent-1000000001",
                providerType: "claude_code",
                agents: agents
            ),
            "Test Agent",
            "agent display name wins over provider"
        )
    }

    func testCreatorFallsBackToAgentIdWhenCatalogMisses() {
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "agent-1000000003",
                providerType: "claude_code",
                agents: []
            ),
            "agent-1000000003",
            "a present agentId is shown raw when the catalog misses it"
        )
    }

    func testCreatorPrettifiesProviderWhenNoAgentId() {
        // Pin the prettified provider fallback contract.
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: nil,
                providerType: "claude_code",
                agents: []
            ),
            "Claude Code"
        )
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "   ",
                providerType: "codex_app_server",
                agents: []
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
                agents: []
            ),
            "Agent"
        )
    }

    func testCreatorSkipsBlankAgentDisplayName() {
        let agents = [makeAgent(id: "agent-1000000004", displayName: "   ")]
        XCTAssertEqual(
            GaryxCapsuleGalleryCardPresentation.creatorName(
                agentId: "agent-1000000004",
                providerType: "claude_code",
                agents: agents
            ),
            "agent-1000000004",
            "a blank agent display name falls through to the raw agent ID"
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
