import XCTest
@testable import GaryxMobile

/// TASK-1802: the head-refresh commit is a synchronous MainActor unit that
/// re-filters every pre-await snapshot against the commit-point archive
/// state. Pins these review #TASK-1804 interleavings without a network:
/// the test plays the model exactly as `refreshThreads` does — capture
/// page/pins/fetched threads, suspend (archive happens), then commit.
@MainActor
final class GaryxHomeThreadListRefreshCommitTests: XCTestCase {
    func testCommitDoesNotResurrectThreadArchivedDuringBackfillAwait() throws {
        let model = makeModel()
        let pinned = makeThread(id: "thread-pinned", title: "Pinned build")
        let recent = makeThread(id: "thread-recent", title: "Recent chat")
        let incoming = makeThread(id: "thread-new", title: "New arrival")
        model.threads = [pinned, recent]
        model.pinnedThreadIds = [pinned.id]
        model.recentThreadIds = [pinned.id, recent.id]

        // Pre-await captures, exactly like refreshThreads: the page and the
        // pins arrived while `thread-pinned` was still live.
        let page = try makeRecentThreadsPage(threads: [pinned, recent, incoming])
        let fetchedThreads = [pinned, recent, incoming]

        // Backfill await window: the user archives the pinned thread. The
        // archive flow marks it pending and removes it locally before its
        // gateway call completes.
        model.pendingThreadArchives.startArchive(threadId: pinned.id)
        model.threads.removeAll { $0.id == pinned.id }
        model.pinnedThreadIds.removeAll { $0 == pinned.id }
        model.recentThreadIds.removeAll { $0 == pinned.id }

        // The refresh resumes and commits its pre-await snapshots.
        model.commitRefreshedRecentThreadsPage(
            page: page,
            pinsPageThreadIds: [pinned.id],
            application: .replaceHead,
            fetchedThreads: fetchedThreads,
            previousThreadSummaries: [pinned, recent],
            previouslyRemoteBusyThreadIds: [],
            selectionIdForThisRefresh: nil,
            runtimeGeneration: model.gatewayRuntimeGeneration
        )

        XCTAssertFalse(
            model.pinnedThreadIds.contains(pinned.id),
            "a pre-await pins snapshot must not resurrect an archived thread"
        )
        XCTAssertFalse(
            model.recentThreadIds.contains(pinned.id),
            "a pre-await page snapshot must not resurrect an archived thread"
        )
        XCTAssertFalse(
            model.threads.contains { $0.id == pinned.id },
            "pre-await fetched summaries must not resurrect an archived thread"
        )
        XCTAssertEqual(model.recentThreadIds, [recent.id, incoming.id])
        XCTAssertTrue(model.threads.contains { $0.id == incoming.id })
    }

    func testCommitAppliesPinsPageAndThreadsWithoutPendingArchives() throws {
        let model = makeModel()
        let pinned = makeThread(id: "thread-pinned", title: "Pinned build")
        let recent = makeThread(id: "thread-recent", title: "Recent chat")
        let page = try makeRecentThreadsPage(threads: [pinned, recent])

        model.commitRefreshedRecentThreadsPage(
            page: page,
            pinsPageThreadIds: [pinned.id],
            application: .replaceHead,
            fetchedThreads: [pinned, recent],
            previousThreadSummaries: [],
            previouslyRemoteBusyThreadIds: [],
            selectionIdForThisRefresh: nil,
            runtimeGeneration: model.gatewayRuntimeGeneration
        )

        XCTAssertEqual(model.pinnedThreadIds, [pinned.id])
        XCTAssertEqual(model.recentThreadIds, [pinned.id, recent.id])
        XCTAssertEqual(model.threads.map(\.id).sorted(), [pinned.id, recent.id].sorted())
    }

    /// The archive-resolved interleaving (review #TASK-1804 round 3) is
    /// gated in Core (`abandonedLocalMutation`); what the app must
    /// guarantee is that local list surgery actually marks the pager.
    func testLocalListSurgeryMarksThePagerMutationSequence() {
        let model = makeModel()
        let thread = makeThread(id: "thread-surgery", title: "Doomed")
        model.threads = [thread]
        model.pinnedThreadIds = [thread.id]
        model.recentThreadIds = [thread.id]

        let base = model.threadListPager.localMutationSequence
        model.removeArchivedThreadLocally(thread.id)
        XCTAssertGreaterThan(
            model.threadListPager.localMutationSequence, base,
            "archive/delete local removal must invalidate in-flight refresh commits"
        )

        let afterRemove = model.threadListPager.localMutationSequence
        model.removePinnedThreadIdLocally(thread.id)
        XCTAssertGreaterThan(
            model.threadListPager.localMutationSequence, afterRemove,
            "pin removal must invalidate in-flight refresh commits"
        )
    }

    private func makeModel() -> GaryxMobileModel {
        let suiteName = "GaryxHomeThreadListRefreshCommitTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("http://127.0.0.1:31337", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        return GaryxMobileModel(defaults: defaults)
    }

    private func makeThread(id: String, title: String) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: title,
            createdAt: nil,
            updatedAt: "2026-07-07T02:00:00Z",
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            teamId: nil,
            teamName: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }

    /// Decodes the same wire shape the gateway returns so the commit sees a
    /// real page, not a hand-built lookalike.
    private func makeRecentThreadsPage(threads: [GaryxThreadSummary]) throws -> GaryxRecentThreadsPage {
        let rows = threads.map { thread in
            """
            {"thread_id": "\(thread.id)", "title": "\(thread.title)",
             "last_active_at": "2026-07-07T02:00:00Z", "last_message_preview": ""}
            """
        }
        let json = """
        {
          "threads": [\(rows.joined(separator: ","))],
          "count": \(threads.count), "limit": 30, "offset": 0,
          "total": \(threads.count), "has_more": false
        }
        """
        return try JSONDecoder().decode(GaryxRecentThreadsPage.self, from: Data(json.utf8))
    }
}
