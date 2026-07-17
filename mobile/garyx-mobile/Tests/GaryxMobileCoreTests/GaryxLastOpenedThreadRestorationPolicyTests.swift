import XCTest
@testable import GaryxMobileCore

final class GaryxLastOpenedThreadRestorationPolicyTests: XCTestCase {
    func testPersistenceDecisionRemainsInCoreUntilRetiredGateIsDeleted() {
        XCTAssertTrue(
            GaryxLastOpenedThreadRestorationPolicy.shouldPersistLastOpenedThread(
                excludedFromRecent: false
            )
        )
        XCTAssertFalse(
            GaryxLastOpenedThreadRestorationPolicy.shouldPersistLastOpenedThread(
                excludedFromRecent: true
            )
        )
    }

    func testRestoresPersistedThreadWhenNavigationIsUnclaimed() {
        XCTAssertEqual(
            GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
                persistedLastOpenedThreadId: " thread::restored ",
                persistedLastSessionWasOnThread: true,
                selectedThreadId: nil,
                hasPendingMobileRoute: false,
                hasPendingThreadIntent: false,
                navigationState: GaryxMobileNavigationState(),
                sidebarVisible: false
            ),
            "thread::restored"
        )
    }

    func testDoesNotRestoreWhenAnotherNavigationClaimExists() {
        XCTAssertNil(
            GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
                persistedLastOpenedThreadId: "thread::restored",
                persistedLastSessionWasOnThread: true,
                selectedThreadId: nil,
                hasPendingMobileRoute: true,
                hasPendingThreadIntent: false,
                navigationState: GaryxMobileNavigationState(),
                sidebarVisible: false
            )
        )
    }

    func testCurrentSessionRequiresPresentedConversationAndThread() {
        XCTAssertTrue(
            GaryxLastOpenedThreadRestorationPolicy.isCurrentSessionRestorable(
                navigationState: GaryxMobileNavigationState(
                    activePanel: .chat,
                    presentsContent: true
                ),
                selectedThreadId: "thread::selected"
            )
        )
        XCTAssertFalse(
            GaryxLastOpenedThreadRestorationPolicy.isCurrentSessionRestorable(
                navigationState: GaryxMobileNavigationState(),
                selectedThreadId: "thread::selected"
            )
        )
    }

    func testInitialEmptyLoadingSnapshotDerivesRecentSkeletonRowsInCore() {
        let store = GaryxHomeThreadListStore()
        let input = GaryxHomeThreadListInput(
            sectionsInput: GaryxHomeThreadSectionsInput(
                threads: [],
                agents: [],
                automations: [],
                pinnedThreadIds: [],
                recentThreadIds: [],
                selectedThreadId: nil
            ),
            runningThreadIds: [],
            isLoadingThreads: true,
            isHomeVisible: true
        )

        XCTAssertTrue(store.apply(input))
        XCTAssertEqual(store.snapshot.recentPlaceholder, .loadingSkeleton(rowCount: 6))
    }

    func testCachedRecentRowsSuppressSkeletonDuringRefresh() {
        let fixture = GaryxHomeListFixture.makeInputs(threadCount: 3, pinnedCount: 0, runningCount: 0)
        let store = GaryxHomeThreadListStore()
        let input = GaryxHomeThreadListInput(
            sectionsInput: GaryxHomeThreadSectionsInput(
                threads: fixture.threads,
                agents: fixture.agents,
                automations: fixture.automations,
                pinnedThreadIds: fixture.pinnedThreadIds,
                recentThreadIds: fixture.recentThreadIds,
                selectedThreadId: fixture.selectedThreadId
            ),
            runningThreadIds: [],
            isLoadingThreads: true,
            isHomeVisible: true
        )

        XCTAssertTrue(store.apply(input))
        XCTAssertEqual(store.snapshot.sections.recent.count, 3)
        XCTAssertEqual(store.snapshot.recentPlaceholder, .none)
    }
}
