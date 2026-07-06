import XCTest
@testable import GaryxMobileCore

final class GaryxTeamMembershipDraftTests: XCTestCase {
    private func agent(
        _ id: String,
        name: String,
        builtIn: Bool = false,
        standalone: Bool = true
    ) -> GaryxAgentSummary {
        GaryxAgentSummary(
            id: id,
            displayName: name,
            providerType: "",
            model: "",
            builtIn: builtIn,
            standalone: standalone
        )
    }

    // MARK: - memberIds(from:)

    func testMemberIdsParsesEmptyAndSingle() {
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIds(from: ""), [])
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIds(from: "   "), [])
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIds(from: "alpha"), ["alpha"])
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIds(from: "  alpha  "), ["alpha"])
    }

    func testMemberIdsParsesMixedSeparatorsAndDedupes() {
        XCTAssertEqual(
            GaryxTeamMembershipDraft.memberIds(from: "a, b\nc d,,  e"),
            ["a", "b", "c", "d", "e"]
        )
        XCTAssertEqual(
            GaryxTeamMembershipDraft.memberIds(from: "a, b, a, c, b"),
            ["a", "b", "c"]
        )
    }

    // Separators are exactly ",", "\n", and the plain U+0020 space. A tab is
    // NOT a separator: it stays inside the token (interior tabs survive the
    // trim, which only strips the token's ends).
    func testMemberIdsTabIsNotASeparator() {
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIds(from: "a\tb, c"), ["a\tb", "c"])
    }

    func testMemberIdsDedupeIsCaseSensitive() {
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIds(from: "a, A"), ["a", "A"])
    }

    // MARK: - memberIdsString(_:)

    func testMemberIdsStringJoinsTrimsAndFiltersEmpties() {
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIdsString([]), "")
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIdsString(["a", " b ", "", "  "]), "a, b")
    }

    func testMemberIdsStringDoesNotDeduplicate() {
        XCTAssertEqual(GaryxTeamMembershipDraft.memberIdsString(["a", "a"]), "a, a")
    }

    // MARK: - normalizedMemberIds(from:leaderAgentId:)

    func testNormalizedMemberIdsPutsLeaderFirst() {
        XCTAssertEqual(
            GaryxTeamMembershipDraft.normalizedMemberIds(from: "b, c", leaderAgentId: "a"),
            ["a", "b", "c"]
        )
    }

    func testNormalizedMemberIdsDedupesLeaderFromRawValue() {
        XCTAssertEqual(
            GaryxTeamMembershipDraft.normalizedMemberIds(from: "b, a, c, a", leaderAgentId: " a "),
            ["a", "b", "c"]
        )
    }

    func testNormalizedMemberIdsWithEmptyLeader() {
        XCTAssertEqual(
            GaryxTeamMembershipDraft.normalizedMemberIds(from: "b\nc d", leaderAgentId: "  "),
            ["b", "c", "d"]
        )
    }

    func testNormalizedMemberIdsTabIsNotASeparator() {
        XCTAssertEqual(
            GaryxTeamMembershipDraft.normalizedMemberIds(from: "b\tc, d", leaderAgentId: "a"),
            ["a", "b\tc", "d"]
        )
    }

    // MARK: - selectLeader

    func testSelectLeaderInsertsMissingLeaderAtFront() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "", memberAgentIds: "b, c")
        draft.selectLeader("a")
        XCTAssertEqual(draft.leaderAgentId, "a")
        XCTAssertEqual(draft.memberAgentIds, "a, b, c")
    }

    func testSelectLeaderKeepsExistingMemberStringWhenAlreadyMember() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "b", memberAgentIds: "b, c")
        draft.selectLeader("c")
        XCTAssertEqual(draft.leaderAgentId, "c")
        // Rewritten in normalized form; value is unchanged for normalized input.
        XCTAssertEqual(draft.memberAgentIds, "b, c")
    }

    func testSelectLeaderNormalizesMemberString() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "", memberAgentIds: "b,c\nd")
        draft.selectLeader("b")
        XCTAssertEqual(draft.leaderAgentId, "b")
        XCTAssertEqual(draft.memberAgentIds, "b, c, d")
    }

    // MARK: - toggleMember

    func testToggleMemberAddsAndPromotesToLeaderWhenNoneSet() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "", memberAgentIds: "")
        draft.toggleMember("a")
        XCTAssertEqual(draft.leaderAgentId, "a")
        XCTAssertEqual(draft.memberAgentIds, "a")
    }

    func testToggleMemberAddsWithoutTouchingExistingLeader() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "a", memberAgentIds: "a")
        draft.toggleMember("b")
        XCTAssertEqual(draft.leaderAgentId, "a")
        XCTAssertEqual(draft.memberAgentIds, "a, b")
    }

    func testToggleMemberRemovesNonLeader() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "a", memberAgentIds: "a, b, c")
        draft.toggleMember("b")
        XCTAssertEqual(draft.leaderAgentId, "a")
        XCTAssertEqual(draft.memberAgentIds, "a, c")
    }

    func testToggleMemberRemovingLeaderHandsOffToFirstRemaining() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "a", memberAgentIds: "a, b, c")
        draft.toggleMember("a")
        XCTAssertEqual(draft.leaderAgentId, "b")
        XCTAssertEqual(draft.memberAgentIds, "b, c")
    }

    func testToggleMemberRemovingLastMemberClearsLeader() {
        var draft = GaryxTeamMembershipDraft(leaderAgentId: "a", memberAgentIds: "a")
        draft.toggleMember("a")
        XCTAssertEqual(draft.leaderAgentId, "")
        XCTAssertEqual(draft.memberAgentIds, "")
    }

    // MARK: - agentOptions

    func testAgentOptionsFilterStandaloneAndSortBuiltInFirstThenName() {
        let options = GaryxTeamFormPresentation.agentOptions(
            [
                agent("zeta", name: "Zeta"),
                agent("beta", name: "beta"),
                agent("claude", name: "Claude", builtIn: true),
                agent("hidden", name: "Hidden", standalone: false),
                agent("alpha", name: "Alpha"),
            ],
            preserving: []
        )
        XCTAssertEqual(options.map(\.id), ["claude", "alpha", "beta", "zeta"])
    }

    func testAgentOptionsSortIsCaseInsensitive() {
        let options = GaryxTeamFormPresentation.agentOptions(
            [
                agent("b", name: "bravo"),
                agent("a", name: "Alpha"),
                agent("c", name: "Charlie"),
            ],
            preserving: []
        )
        XCTAssertEqual(options.map(\.id), ["a", "b", "c"])
    }

    func testAgentOptionsDeduplicateById() {
        let options = GaryxTeamFormPresentation.agentOptions(
            [
                agent("a", name: "Alpha"),
                agent("a", name: "Alpha Again"),
            ],
            preserving: []
        )
        XCTAssertEqual(options.count, 1)
        XCTAssertEqual(options[0].displayName, "Alpha")
    }

    func testAgentOptionsDoNotSynthesizePlaceholderForKnownPreservedId() {
        let options = GaryxTeamFormPresentation.agentOptions(
            [agent("a", name: "Alpha")],
            preserving: ["a", "  ", ""]
        )
        XCTAssertEqual(options.map(\.id), ["a"])
        XCTAssertEqual(options[0].displayName, "Alpha")
    }

    func testAgentOptionsPrependUnknownPreservedIdsOneByOne() {
        let options = GaryxTeamFormPresentation.agentOptions(
            [agent("a", name: "Alpha")],
            preserving: [" ghost1 ", "ghost2", "ghost1"]
        )
        // Each unknown id is inserted at index 0 in turn, so multiple unknown
        // ids end up in reverse order — pinned existing semantics.
        XCTAssertEqual(options.map(\.id), ["ghost2", "ghost1", "a"])
        XCTAssertEqual(options[0].displayName, "ghost2")
        XCTAssertFalse(options[0].builtIn)
        XCTAssertTrue(options[0].standalone)
        XCTAssertEqual(options[0].providerType, "")
        XCTAssertEqual(options[0].model, "")
    }

    // MARK: - leaderLabel

    func testLeaderLabelEmptyLeaderIsChoosePlaceholder() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.leaderLabel(leaderAgentId: "", options: [agent("a", name: "Alpha")]),
            "Choose leader"
        )
        XCTAssertEqual(
            GaryxTeamFormPresentation.leaderLabel(leaderAgentId: " \n", options: []),
            "Choose leader"
        )
    }

    func testLeaderLabelUsesResolvedDisplayName() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.leaderLabel(leaderAgentId: " a ", options: [agent("a", name: "Alpha")]),
            "Alpha"
        )
    }

    // A resolved option with an empty display name yields an empty label — no
    // id fallback in the editable leader row (existing behavior).
    func testLeaderLabelResolvedEmptyDisplayNameStaysEmpty() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.leaderLabel(leaderAgentId: "a", options: [agent("a", name: "")]),
            ""
        )
    }

    func testLeaderLabelUnresolvedIdFallsBackToId() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.leaderLabel(leaderAgentId: "ghost", options: [agent("a", name: "Alpha")]),
            "ghost"
        )
    }

    // MARK: - membersLabel

    func testMembersLabelEmptyAndUpToTwoMembers() {
        let agents = [agent("a", name: "Alpha"), agent("b", name: "Bravo")]
        XCTAssertEqual(GaryxTeamFormPresentation.membersLabel(memberIds: [], agents: agents), "")
        XCTAssertEqual(GaryxTeamFormPresentation.membersLabel(memberIds: ["a"], agents: agents), "Alpha")
        XCTAssertEqual(
            GaryxTeamFormPresentation.membersLabel(memberIds: ["a", "b"], agents: agents),
            "Alpha, Bravo"
        )
    }

    func testMembersLabelTruncatesBeyondTwoMembers() {
        let agents = [
            agent("a", name: "Alpha"),
            agent("b", name: "Bravo"),
            agent("c", name: "Charlie"),
            agent("d", name: "Delta"),
        ]
        XCTAssertEqual(
            GaryxTeamFormPresentation.membersLabel(memberIds: ["a", "b", "c"], agents: agents),
            "Alpha, Bravo +1"
        )
        XCTAssertEqual(
            GaryxTeamFormPresentation.membersLabel(memberIds: ["a", "b", "c", "d"], agents: agents),
            "Alpha, Bravo +2"
        )
    }

    func testMembersLabelUnknownIdFallsBackToId() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.membersLabel(memberIds: ["ghost", "a"], agents: [agent("a", name: "Alpha")]),
            "ghost, Alpha"
        )
    }

    // A resolved agent with an empty display name contributes an empty segment
    // to the join — no id fallback in the editable members row (existing
    // behavior).
    func testMembersLabelResolvedEmptyDisplayNameStaysEmptySegment() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.membersLabel(
                memberIds: ["a", "b"],
                agents: [agent("a", name: ""), agent("b", name: "Bravo")]
            ),
            ", Bravo"
        )
    }

    // MARK: - memberDetailLabel(s)

    func testMemberDetailLabelFormatsNameAndId() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.memberDetailLabel(agentId: " a ", agents: [agent("a", name: "Alpha")]),
            "Alpha (a)"
        )
    }

    // The read-only detail label is the only one that falls back to the id
    // when the resolved display name is empty (existing behavior).
    func testMemberDetailLabelEmptyDisplayNameFallsBackToId() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.memberDetailLabel(agentId: "a", agents: [agent("a", name: "")]),
            "a"
        )
    }

    func testMemberDetailLabelUnknownAndEmptyIds() {
        XCTAssertEqual(
            GaryxTeamFormPresentation.memberDetailLabel(agentId: "ghost", agents: [agent("a", name: "Alpha")]),
            "ghost"
        )
        XCTAssertEqual(GaryxTeamFormPresentation.memberDetailLabel(agentId: "  ", agents: []), "")
    }

    func testMemberDetailLabelsJoinsWithNewlines() {
        let agents = [agent("a", name: "Alpha"), agent("b", name: "Bravo")]
        XCTAssertEqual(
            GaryxTeamFormPresentation.memberDetailLabels(memberAgentIds: "", agents: agents),
            ""
        )
        XCTAssertEqual(
            GaryxTeamFormPresentation.memberDetailLabels(memberAgentIds: "a, b, ghost", agents: agents),
            "Alpha (a)\nBravo (b)\nghost"
        )
    }
}
