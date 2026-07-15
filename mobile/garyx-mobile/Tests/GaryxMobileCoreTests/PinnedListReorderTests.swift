import XCTest
@testable import GaryxMobileCore

final class PinnedListReorderTests: XCTestCase {
    func testTranslatesFlatMoveIntoPinnedRelativeOrder() {
        let items = fixtureItems(pinnedIds: ["a", "b", "c"], recentIds: ["d"])

        XCTAssertEqual(
            GaryxPinnedListMoveTranslator.translate(
                items: items,
                sourceOffsets: IndexSet(integer: 1),
                destination: 4
            ),
            GaryxPinnedListMove(order: ["b", "c", "a"], destination: 3)
        )
    }

    func testClampsDestinationsAboveAndBelowPinnedSegment() {
        let items = fixtureItems(pinnedIds: ["a", "b", "c"], recentIds: ["d", "e"])

        XCTAssertEqual(
            GaryxPinnedListMoveTranslator.translate(
                items: items,
                sourceOffsets: IndexSet(integer: 3),
                destination: 0
            ),
            GaryxPinnedListMove(order: ["c", "a", "b"], destination: 0)
        )
        XCTAssertEqual(
            GaryxPinnedListMoveTranslator.translate(
                items: items,
                sourceOffsets: IndexSet(integer: 1),
                destination: items.count
            ),
            GaryxPinnedListMove(order: ["b", "c", "a"], destination: 3)
        )
    }

    func testRejectsMovesFromNonPinnedItems() {
        let items = fixtureItems(pinnedIds: ["a", "b"], recentIds: ["c"])

        XCTAssertNil(
            GaryxPinnedListMoveTranslator.translate(
                items: items,
                sourceOffsets: IndexSet(integer: 0),
                destination: 2
            )
        )
        XCTAssertNil(
            GaryxPinnedListMoveTranslator.translate(
                items: items,
                sourceOffsets: IndexSet(integer: 5),
                destination: 1
            )
        )
    }

    private func fixtureItems(
        pinnedIds: [String],
        recentIds: [String]
    ) -> [GaryxHomeThreadListItem] {
        [.pinnedHeader]
            + pinnedIds.map { .thread(row(id: $0, pinned: true), region: .pinned) }
            + [.pinnedSpacer, .recentHeader]
            + recentIds.map { .thread(row(id: $0, pinned: false), region: .recent) }
    }

    private func row(id: String, pinned: Bool) -> GaryxHomeThreadRow {
        GaryxHomeThreadRow(
            id: id,
            thread: GaryxThreadSummary(
                id: id,
                title: id,
                createdAt: "2026-01-01T00:00:00Z",
                updatedAt: nil,
                lastMessagePreview: "",
                workspacePath: nil,
                messageCount: nil,
                agentId: nil,
                providerType: nil,
                recentRunId: nil,
                activeRunId: nil,
                runState: nil,
                worktreePath: nil
            ),
            presentation: GaryxSidebarThreadRowPresentation(
                title: id,
                subtitle: nil,
                trailingTimestamp: nil,
                isSelected: false,
                isPinned: pinned,
                isRunning: false
            ),
            avatar: GaryxSidebarThreadRowAvatar(
                agentId: "agent",
                avatarDataUrl: "",
                label: "Agent",
                providerType: "codex",
                builtIn: false
            ),
            timestampValue: nil,
            canArchive: true,
            showsDivider: false
        )
    }
}
