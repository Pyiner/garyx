import XCTest
@testable import GaryxMobileCore

final class GaryxWidgetAgentIdentityProjectorTests: XCTestCase {
    func testAgentThreadUsesCatalogIdentity() {
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
            ]
        )
        XCTAssertEqual(
            identity,
            GaryxWidgetAgentIdentity(
                id: "agent-1",
                name: "Coder",
                avatarDataUrl: "data:image/png;base64,def",
                providerType: "claude_code",
                builtIn: true
            )
        )
    }

    func testUnknownAgentFallsBackToThreadProviderType() {
        let identity = GaryxWidgetAgentIdentityProjector.identity(
            for: thread(agentId: "agent-x", providerType: "codex"),
            agents: [agent(id: "agent-1")]
        )
        XCTAssertEqual(
            identity,
            GaryxWidgetAgentIdentity(
                id: "agent-x",
                name: nil,
                avatarDataUrl: nil,
                providerType: "codex",
                builtIn: false
            )
        )
    }

    func testDictionaryVariantMatchesArrayVariant() {
        let agents = [agent(id: "agent-1", displayName: "Coder"), agent(id: "agent-2")]
        let agentsById = Dictionary(uniqueKeysWithValues: agents.map { ($0.id, $0) })
        for subject in [
            thread(agentId: "agent-1"),
            thread(agentId: "agent-x", providerType: "codex"),
            thread(providerType: "claude_code"),
        ] {
            XCTAssertEqual(
                GaryxWidgetAgentIdentityProjector.identity(for: subject, agents: agents),
                GaryxWidgetAgentIdentityProjector.identity(for: subject, agentsById: agentsById)
            )
        }
    }

    private func thread(
        agentId: String? = nil,
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
}
