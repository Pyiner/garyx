import XCTest
@testable import GaryxMobileCore

final class GaryxCustomAgentDraftRulesTests: XCTestCase {
    func testDeriveIdGoldenFixtureMatchesMacSemantics() throws {
        let fixture = try loadParityFixture()
        for item in fixture.deriveIdCases {
            XCTAssertEqual(
                GaryxCustomAgentDraftRules.deriveId(from: item.name),
                item.expected,
                "deriveId mismatch for \(String(reflecting: item.name))"
            )
        }
    }

    func testCreateNameChangesPureDerivedId() {
        var draft = GaryxCustomAgentDraft.create()
        draft.displayName = "  Test Agent  "
        XCTAssertEqual(draft.agentId, "test-agent")

        draft.displayName = "Agent__42"
        XCTAssertEqual(draft.agentId, "agent-42")
    }

    func testEditNameChangeLeavesImmutableIdStrictlyUnchanged() {
        var draft = GaryxCustomAgentDraft.edit(authoritative: authoritativeAgent())
        let originalId = draft.agentId

        draft.displayName = "A completely different name"

        XCTAssertEqual(draft.agentId, originalId)
        XCTAssertNotEqual(
            draft.agentId,
            GaryxCustomAgentDraftRules.deriveId(from: draft.displayName),
            "The edit ID is immutable, not merely temporarily out of sync"
        )
        XCTAssertEqual(draft.makeRequest()?.agentId, originalId)
    }

    func testCJKEmojiAndPunctuationNamesExposeDerivedIdValidation() {
        for name in ["研发助手", "🧠🤖", "---___!!!"] {
            var draft = GaryxCustomAgentDraft.create()
            draft.displayName = name
            XCTAssertEqual(draft.agentId, "")
            XCTAssertEqual(
                draft.nameValidationMessage,
                "Name must include at least one English letter or number."
            )
            XCTAssertFalse(draft.canSubmit)
        }
    }

    func testWhitespaceOnlyNameIsRequired() {
        var draft = GaryxCustomAgentDraft.create()
        draft.displayName = "\u{00A0}\u{2007}\u{FEFF}"
        XCTAssertEqual(draft.nameValidationMessage, "Name is required.")
    }

    func testCreateCollisionSurvivesEquivalentNameAndClearsOnlyWhenDerivedIdChanges() {
        var draft = GaryxCustomAgentDraft.create()
        draft.displayName = "Test Agent"
        draft.recordCreateConflict()
        XCTAssertFalse(draft.canSubmit)
        XCTAssertEqual(draft.createCollision?.derivedAgentId, "test-agent")

        draft.displayName = "Test---Agent"
        XCTAssertEqual(draft.agentId, "test-agent")
        XCTAssertNotNil(draft.createCollision)

        draft.displayName = "Test Agent Two"
        XCTAssertEqual(draft.agentId, "test-agent-two")
        XCTAssertNil(draft.createCollision)
        XCTAssertTrue(draft.canSubmit)
    }

    func testCreatePayloadIsStrictCreateWithoutExpectedToken() throws {
        var draft = GaryxCustomAgentDraft.create()
        draft.displayName = "Planning Agent"
        draft.providerType = "codex_app_server"
        draft.model = "test-model"
        draft.modelReasoningEffort = "high"
        draft.defaultWorkspaceDir = " /Users/test/project "
        draft.systemPrompt = " Synthetic instructions. "
        draft.env.addRow()
        draft.env.updateKey(id: try XCTUnwrap(draft.env.rows.last?.id), "TEST_KEY")
        draft.env.updateValue(id: try XCTUnwrap(draft.env.rows.last?.id), "value")

        let request = try XCTUnwrap(draft.makeRequest())
        XCTAssertEqual(request.agentId, "planning-agent")
        XCTAssertEqual(request.displayName, "Planning Agent")
        XCTAssertNil(request.expectedUpdatedAt)
        XCTAssertEqual(request.defaultWorkspaceDir, "/Users/test/project")
        XCTAssertEqual(request.providerEnv, ["TEST_KEY": "value"])
        XCTAssertEqual(request.systemPrompt, "Synthetic instructions.")
    }

    func testEditPayloadUsesFreshTokenAndPreservesHiddenServiceTier() throws {
        var draft = GaryxCustomAgentDraft.edit(authoritative: authoritativeAgent())
        draft.displayName = "Renamed Agent"
        draft.defaultWorkspaceDir = ""

        let request = try XCTUnwrap(draft.makeRequest())
        XCTAssertEqual(request.agentId, "fixed-agent-id")
        XCTAssertEqual(request.expectedUpdatedAt, "2026-07-13T12:00:00Z")
        XCTAssertEqual(request.modelServiceTier, "priority")
        XCTAssertEqual(request.defaultWorkspaceDir, "", "Empty edit workspace is an explicit clear")
        XCTAssertNil(request.providerEnv, "Untouched authoritative env is preserved by omission")
        XCTAssertNil(request.avatarDataUrl, "Untouched avatar is preserved by omission")
    }

    func testEditAvatarRemovalAndEnvClearAreExplicit() throws {
        var draft = GaryxCustomAgentDraft.edit(authoritative: authoritativeAgent())
        draft.removeAvatar()
        for row in draft.env.rows {
            draft.env.removeRow(id: row.id)
        }

        let request = try XCTUnwrap(draft.makeRequest())
        XCTAssertEqual(request.avatarDataUrl, "")
        XCTAssertEqual(request.providerEnv, [:])
    }

    func testProviderAndEnvironmentValidationGateSubmission() throws {
        var draft = GaryxCustomAgentDraft.create(defaultProviderType: "")
        draft.displayName = "Valid Agent"
        XCTAssertTrue(draft.validationIssues.contains(.providerRequired))

        draft.providerType = "codex_app_server"
        draft.env.addRow()
        draft.env.updateKey(id: try XCTUnwrap(draft.env.rows.last?.id), "1INVALID")
        XCTAssertTrue(draft.validationIssues.contains(.invalidEnvironmentKey))
        XCTAssertNil(draft.makeRequest())
    }

    func testServerFailureMappingSeparatesCreateCollisionDeletedAndOCC() {
        XCTAssertEqual(
            GaryxCustomAgentDraftRules.mutationFailure(
                for: .httpStatus(409, #"{"error":"custom agent already exists"}"#),
                mode: .create
            ),
            .createConflict
        )
        XCTAssertEqual(
            GaryxCustomAgentDraftRules.mutationFailure(
                for: .httpStatus(404, #"{"error":"custom agent not found"}"#),
                mode: .edit(agentId: "fixed-agent-id", expectedUpdatedAt: "old")
            ),
            .deleted
        )
        XCTAssertEqual(
            GaryxCustomAgentDraftRules.mutationFailure(
                for: .httpStatus(
                    409,
                    #"{"error":"custom agent changed","current_updated_at":"2026-07-13T12:01:00Z"}"#
                ),
                mode: .edit(agentId: "fixed-agent-id", expectedUpdatedAt: "old")
            ),
            .editConflict(currentUpdatedAt: "2026-07-13T12:01:00Z")
        )
    }

    private func authoritativeAgent() -> GaryxAgentSummary {
        GaryxAgentSummary(
            id: "fixed-agent-id",
            displayName: "Original Agent",
            providerType: "codex_app_server",
            model: "test-model",
            modelReasoningEffort: "high",
            modelServiceTier: "priority",
            providerEnv: ["KEEP": "value"],
            defaultWorkspaceDir: "/Users/test/project",
            avatarDataUrl: "data:image/png;base64,aGVsbG8=",
            systemPrompt: "Synthetic instructions.",
            updatedAt: "2026-07-13T12:00:00Z"
        )
    }
}

private struct AgentAvatarParityFixture: Decodable {
    struct DeriveIdCase: Decodable {
        let name: String
        let expected: String
    }

    let deriveIdCases: [DeriveIdCase]
}

private func loadParityFixture() throws -> AgentAvatarParityFixture {
    let url = try XCTUnwrap(
        Bundle.module.url(
            forResource: "agent-avatar-parity",
            withExtension: "json",
            subdirectory: "Fixtures"
        )
    )
    return try JSONDecoder().decode(AgentAvatarParityFixture.self, from: Data(contentsOf: url))
}
