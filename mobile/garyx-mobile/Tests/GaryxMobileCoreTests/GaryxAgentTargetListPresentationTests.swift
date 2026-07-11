import XCTest
@testable import GaryxMobileCore

final class GaryxAgentTargetListPresentationTests: XCTestCase {
    func testOrderedPutsBuiltInAgentsBeforeCustomAgents() {
        let ordered = GaryxAgentTargetListPresentation.ordered([
            agent("gary"),
            agent("claude", builtIn: true),
            agent("quant"),
            agent("codex", builtIn: true),
        ])
        XCTAssertEqual(ordered.map(\.id), ["claude", "codex", "gary", "quant"])
    }

    func testPrimaryCollapsesToLimitAndKeepsSelectionVisible() {
        let targets = [
            agent("claude", builtIn: true),
            agent("codex", builtIn: true),
            agent("antigravity", builtIn: true),
            agent("gary"),
            agent("quant"),
            agent("custom"),
        ]

        let primary = GaryxAgentTargetListPresentation.primary(targets, selectedId: "claude")
        XCTAssertEqual(primary.map(\.id), ["claude", "codex", "antigravity", "gary", "quant"])

        let withHiddenSelection = GaryxAgentTargetListPresentation.primary(
            targets,
            selectedId: "custom"
        )
        XCTAssertEqual(
            withHiddenSelection.map(\.id),
            ["claude", "codex", "antigravity", "gary", "custom"]
        )
        XCTAssertEqual(GaryxAgentTargetListPresentation.overflowCount(targets), 1)
    }

    private func agent(_ id: String, builtIn: Bool = false) -> GaryxMobileAgentTarget {
        GaryxMobileAgentTarget(
            id: id,
            title: id,
            subtitle: "",
            avatarDataUrl: "",
            providerType: "claude_code",
            builtIn: builtIn
        )
    }
}
