import XCTest
@testable import GaryxMobileCore

final class GaryxAgentTargetListPresentationTests: XCTestCase {
    func testOrderedPutsBuiltInAgentsFirstThenCustomThenTeams() {
        let ordered = GaryxAgentTargetListPresentation.ordered([
            team("review-team"),
            agent("gary"),
            agent("claude", builtIn: true),
            agent("quant"),
            agent("codex", builtIn: true),
        ])
        XCTAssertEqual(ordered.map(\.id), ["claude", "codex", "gary", "quant", "review-team"])
    }

    func testPrimaryReturnsAllWhenWithinLimit() {
        let targets = [agent("claude", builtIn: true), agent("gary"), team("review-team")]
        let primary = GaryxAgentTargetListPresentation.primary(targets, selectedId: "gary")
        XCTAssertEqual(primary.map(\.id), ["claude", "gary", "review-team"])
        XCTAssertEqual(GaryxAgentTargetListPresentation.overflowCount(targets), 0)
    }

    func testPrimaryCollapsesToLimitAndKeepsSelectionVisible() {
        let targets = [
            agent("claude", builtIn: true),
            agent("codex", builtIn: true),
            agent("gemini", builtIn: true),
            agent("gary"),
            agent("quant"),
            agent("native"),
            team("review-team"),
        ]

        let primary = GaryxAgentTargetListPresentation.primary(targets, selectedId: "claude")
        XCTAssertEqual(primary.map(\.id), ["claude", "codex", "gemini", "gary", "quant"])

        let withHiddenSelection = GaryxAgentTargetListPresentation.primary(
            targets,
            selectedId: "review-team"
        )
        XCTAssertEqual(
            withHiddenSelection.map(\.id),
            ["claude", "codex", "gemini", "gary", "review-team"]
        )
        XCTAssertEqual(GaryxAgentTargetListPresentation.overflowCount(targets), 2)
    }

    private func agent(_ id: String, builtIn: Bool = false) -> GaryxMobileAgentTarget {
        GaryxMobileAgentTarget(
            id: id,
            title: id,
            subtitle: "",
            kind: .agent,
            avatarDataUrl: "",
            providerType: "claude_code",
            builtIn: builtIn
        )
    }

    private func team(_ id: String) -> GaryxMobileAgentTarget {
        GaryxMobileAgentTarget(
            id: id,
            title: id,
            subtitle: "2 agents",
            kind: .team,
            avatarDataUrl: "",
            providerType: "",
            builtIn: false
        )
    }
}
