import Foundation
import XCTest
@testable import GaryxMobile

/// TASK-1802: head refresh owns its filter-keyed pager ticket through the
/// final pre-commit await. Pins the App orchestration and #TASK-1804 archive
/// interleavings with an in-process URL loading stub.
@MainActor
final class GaryxHomeThreadListRefreshCommitTests: XCTestCase {
    func testCommitDoesNotResurrectThreadArchivedDuringBackfillAwait() throws {
        let model = makeModel()
        let pinned = makeThread(id: "thread-pinned", title: "Pinned build")
        let recent = makeThread(id: "thread-recent", title: "Recent chat")
        let incoming = makeThread(id: "thread-new", title: "New arrival")
        model.threads = [pinned, recent]
        model.pinnedThreadIds = [pinned.id]
        primeRecentFeed(model, ids: [pinned.id, recent.id], filter: .all)

        // The refresh ticket is captured before the archive races the
        // pre-await page snapshot.
        let ticket = model.recentThreadFeeds.requestRefresh(filter: .all)!

        // Pre-await captures, exactly like refreshThreads: the page and the
        // pins arrived while `thread-pinned` was still live.
        let page = try makeRecentThreadsPage(threads: [pinned, recent, incoming])

        // Backfill await window: the gateway accepts the archive and the app
        // commits its one local removal while this older page is suspended.
        model.pendingThreadArchives.startArchive(threadId: pinned.id)
        model.pendingThreadArchives.commitArchive(threadId: pinned.id)
        model.removeArchivedThreadLocally(pinned.id)

        // The refresh resumes, but the filter-owned pager rejects every
        // pre-await snapshot before the app-layer commit can run.
        let completion = model.recentThreadFeeds.completeRefresh(
            ticket,
            pageIds: page.threads.map(\.id),
            pageOffset: page.offset,
            pageCount: page.count,
            hasMore: page.hasMore
        )
        XCTAssertEqual(completion, .abandonedLocalMutation)

        XCTAssertFalse(
            model.pinnedThreadIds.contains(pinned.id),
            "a pre-await pins snapshot must not resurrect an archived thread"
        )
        XCTAssertFalse(
            model.allRecentThreadIds.contains(pinned.id),
            "a pre-await page snapshot must not resurrect an archived thread"
        )
        XCTAssertFalse(
            model.threads.contains { $0.id == pinned.id },
            "pre-await fetched summaries must not resurrect an archived thread"
        )
        XCTAssertEqual(model.allRecentThreadIds, [recent.id])
        XCTAssertFalse(model.threads.contains { $0.id == incoming.id })
    }

    func testCommitAppliesPinsPageAndThreadsWithoutPendingArchives() throws {
        let model = makeModel()
        let pinned = makeThread(id: "thread-pinned", title: "Pinned build")
        let recent = makeThread(id: "thread-recent", title: "Recent chat")
        let page = try makeRecentThreadsPage(threads: [pinned, recent])

        let ticket = model.recentThreadFeeds.requestRefresh(filter: .all)!
        let completion = model.recentThreadFeeds.completeRefresh(
            ticket,
            pageIds: page.threads.map(\.id),
            pageOffset: page.offset,
            pageCount: page.count,
            hasMore: page.hasMore
        )
        XCTAssertEqual(completion, .apply(.replaceHead))

        model.commitRefreshedRecentThreadsPage(
            pinsPageThreadIds: [pinned.id],
            fetchedThreads: [pinned, recent],
            previousThreadSummaries: [],
            previouslyRemoteBusyThreadIds: [],
            selectionIdForThisRefresh: nil,
            runtimeGeneration: model.gatewayRuntimeGeneration
        )

        XCTAssertEqual(model.pinnedThreadIds, [pinned.id])
        XCTAssertEqual(model.allRecentThreadIds, [pinned.id, recent.id])
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
        primeRecentFeed(model, ids: [thread.id], filter: .all)
        primeRecentFeed(model, ids: [thread.id], filter: .nonTask)

        let allBase = model.recentThreadFeeds.allFeed.pager.localMutationSequence
        let chatsBase = model.recentThreadFeeds.nonTaskFeed.pager.localMutationSequence
        model.removeArchivedThreadLocally(thread.id)
        XCTAssertGreaterThan(
            model.recentThreadFeeds.allFeed.pager.localMutationSequence,
            allBase,
            "archive/delete local removal must invalidate the All feed"
        )
        XCTAssertGreaterThan(
            model.recentThreadFeeds.nonTaskFeed.pager.localMutationSequence,
            chatsBase,
            "archive/delete local removal must invalidate the Chats feed"
        )

        let allAfterRemove = model.recentThreadFeeds.allFeed.pager.localMutationSequence
        let chatsAfterRemove = model.recentThreadFeeds.nonTaskFeed.pager.localMutationSequence
        model.removePinnedThreadIdLocally(thread.id)
        XCTAssertGreaterThan(
            model.recentThreadFeeds.allFeed.pager.localMutationSequence,
            allAfterRemove,
            "pin removal must invalidate the All feed"
        )
        XCTAssertGreaterThan(
            model.recentThreadFeeds.nonTaskFeed.pager.localMutationSequence,
            chatsAfterRemove,
            "pin removal must invalidate the Chats feed"
        )
    }

    func testAllOwnedConsumersAndSidebarSummaryIgnoreTheVisibleChatsFilter() throws {
        let model = makeModel()
        let task = makeThread(id: "thread-task", title: "Task backing thread")
        let chat = makeThread(id: "thread-chat", title: "Chat thread")
        model.threads = [task, chat]
        primeRecentFeed(model, ids: [task.id, chat.id], filter: .all)
        primeRecentFeed(model, ids: [chat.id], filter: .nonTask)
        model.recentThreadFeeds.select(.nonTask)

        XCTAssertEqual(model.visibleRecentThreads.map(\.id), [chat.id])
        XCTAssertEqual(model.allRecentThreads.map(\.id), [task.id, chat.id])
        XCTAssertEqual(try XCTUnwrap(model.sidebarThreadSummary(for: task.id)).id, task.id)
    }

    func testSummaryOnlyTitleUpdateDoesNotRebuildEitherFeedOrder() {
        let model = makeModel()
        let task = makeThread(id: "thread-task", title: "Old task title")
        let chat = makeThread(id: "thread-chat", title: "Chat thread")
        model.threads = [task, chat]
        primeRecentFeed(model, ids: [task.id, chat.id], filter: .all)
        primeRecentFeed(model, ids: [chat.id], filter: .nonTask)

        XCTAssertTrue(model.applyThreadTitleUpdate(threadId: task.id, title: "New task title"))
        XCTAssertEqual(model.allRecentThreadIds, [task.id, chat.id])
        XCTAssertEqual(model.recentThreadFeeds.nonTaskFeed.orderedThreadIds, [chat.id])
        XCTAssertEqual(model.threads.first(where: { $0.id == task.id })?.title, "New task title")
    }

    func testChatsRefreshStartsOneAuxiliaryAllRequestWithoutExtendingSelectedRefresh() async throws {
        let auxiliaryStarted = expectation(description: "auxiliary All request started")
        let auxiliaryGate = DispatchSemaphore(value: 0)
        let allRequestCount = GaryxLockedCounter()
        let chatsPage = try makeRecentThreadsPageData(rows: [
            (id: "thread-chat", title: "Chat thread"),
        ])
        let allPage = try makeRecentThreadsPageData(rows: [
            (id: "thread-task", title: "Task thread"),
            (id: "thread-chat", title: "Chat thread"),
        ])
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if components.path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: Data(#"{"thread_ids":[]}"#.utf8))
            }
            guard components.path == "/api/recent-threads" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            let tasks = components.queryItems?.first(where: { $0.name == "tasks" })?.value
            if tasks == GaryxRecentThreadFilter.all.tasksQueryValue {
                if allRequestCount.increment() == 1 {
                    auxiliaryStarted.fulfill()
                }
                guard auxiliaryGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, data: allPage)
            }
            XCTAssertEqual(tasks, GaryxRecentThreadFilter.nonTask.tasksQueryValue)
            return try garyxStubResponse(request, data: chatsPage)
        }
        defer {
            auxiliaryGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.recentThreadFeeds.select(.nonTask)
        let selectedRefresh = Task { @MainActor in
            await model.refreshThreads(source: .userPullToRefresh)
        }

        await fulfillment(of: [auxiliaryStarted], timeout: 2)
        let auxiliaryTask = try XCTUnwrap(model.auxiliaryAllRecentThreadsRefreshTask)
        await selectedRefresh.value

        XCTAssertEqual(model.visibleRecentThreadIds, ["thread-chat"])
        XCTAssertFalse(model.recentThreadFeeds.selectedPresentation.isRefreshingHead)
        XCTAssertTrue(
            model.recentThreadFeeds.allFeed.presentation.isRefreshingHead,
            "the selected pull spinner must finish while the independent All request is still in flight"
        )

        // A second selected refresh is allowed, but the filter-owned All gate
        // must coalesce its auxiliary request.
        await model.refreshThreads(source: .userPullToRefresh)
        XCTAssertEqual(allRequestCount.value, 1)

        auxiliaryGate.signal()
        await auxiliaryTask.value

        XCTAssertEqual(model.allRecentThreadIds, ["thread-task", "thread-chat"])
        XCTAssertEqual(
            model.visibleRecentThreadIds,
            ["thread-chat"],
            "the auxiliary result may update All and the shared cache, never the selected Chats membership"
        )
        XCTAssertNil(model.lastError)
    }

    func testChatsAuxiliaryFailureOnlyMarksAllFeed() async throws {
        let auxiliaryStarted = expectation(description: "failing auxiliary All request started")
        let auxiliaryGate = DispatchSemaphore(value: 0)
        let chatsPage = try makeRecentThreadsPageData(rows: [
            (id: "thread-chat", title: "Chat thread"),
        ])
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if components.path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: Data(#"{"thread_ids":[]}"#.utf8))
            }
            guard components.path == "/api/recent-threads" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            let tasks = components.queryItems?.first(where: { $0.name == "tasks" })?.value
            if tasks == GaryxRecentThreadFilter.all.tasksQueryValue {
                auxiliaryStarted.fulfill()
                guard auxiliaryGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 400,
                    data: Data(#"{"error":"synthetic auxiliary failure"}"#.utf8)
                )
            }
            return try garyxStubResponse(request, data: chatsPage)
        }
        defer {
            auxiliaryGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.recentThreadFeeds.select(.nonTask)
        let selectedRefresh = Task { @MainActor in
            await model.refreshThreads(source: .userPullToRefresh)
        }
        await fulfillment(of: [auxiliaryStarted], timeout: 2)
        let auxiliaryTask = try XCTUnwrap(model.auxiliaryAllRecentThreadsRefreshTask)
        await selectedRefresh.value
        let selectedPresentation = model.recentThreadFeeds.selectedPresentation

        auxiliaryGate.signal()
        await auxiliaryTask.value

        XCTAssertNil(model.lastError)
        XCTAssertEqual(model.visibleRecentThreadIds, ["thread-chat"])
        XCTAssertEqual(model.recentThreadFeeds.selectedPresentation, selectedPresentation)
        XCTAssertTrue(model.recentThreadFeeds.allFeed.headFailure)
    }

    func testAuxiliaryFailureFromPreviousGatewayDoesNotToastAfterReset() async throws {
        let auxiliaryStarted = expectation(description: "old gateway auxiliary request started")
        let auxiliaryGate = DispatchSemaphore(value: 0)
        let chatsPage = try makeRecentThreadsPageData(rows: [
            (id: "thread-chat", title: "Chat thread"),
        ])
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if components.path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: Data(#"{"thread_ids":[]}"#.utf8))
            }
            guard components.path == "/api/recent-threads" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            let tasks = components.queryItems?.first(where: { $0.name == "tasks" })?.value
            if tasks == GaryxRecentThreadFilter.all.tasksQueryValue {
                auxiliaryStarted.fulfill()
                guard auxiliaryGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 400,
                    data: Data(#"{"error":"old gateway failed"}"#.utf8)
                )
            }
            return try garyxStubResponse(request, data: chatsPage)
        }
        defer {
            auxiliaryGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.recentThreadFeeds.select(.nonTask)
        let selectedRefresh = Task { @MainActor in
            await model.refreshThreads(source: .userPullToRefresh)
        }
        await fulfillment(of: [auxiliaryStarted], timeout: 2)
        let auxiliaryTask = try XCTUnwrap(model.auxiliaryAllRecentThreadsRefreshTask)
        await selectedRefresh.value

        let oldGeneration = model.gatewayRuntimeGeneration
        model.resetGatewayRuntimeState()
        XCTAssertNotEqual(model.gatewayRuntimeGeneration, oldGeneration)
        XCTAssertEqual(model.recentThreadFeeds.selectedFilter, .nonTask)

        auxiliaryGate.signal()
        await auxiliaryTask.value

        XCTAssertNil(
            model.lastError,
            "an old gateway failure must be dropped while reset preserves the selected filter"
        )
        XCTAssertEqual(model.recentThreadFeeds.selectedPresentation, GaryxRecentThreadFeedPresentation())
    }

    func testRestoredChatsFilterOwnsInitialSnapshotAndFirstVisibleRefresh() async throws {
        let suiteName = "GaryxHomeThreadListRefreshCommitTests.restore.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("http://gateway.example.test", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        defaults.set("nonTask", forKey: GaryxMobileSettingsKeys.recentThreadFilter)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let selectedRequestStarted = expectation(description: "restored Chats request started")
        let chatsPage = try makeRecentThreadsPageData(rows: [
            (id: "thread-restored-chat", title: "Restored chat"),
        ])
        let allPage = try makeRecentThreadsPageData(rows: [
            (id: "thread-restored-task", title: "Restored task"),
            (id: "thread-restored-chat", title: "Restored chat"),
        ])
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if components.path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: Data(#"{"thread_ids":[]}"#.utf8))
            }
            guard components.path == "/api/recent-threads" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            let tasks = components.queryItems?.first(where: { $0.name == "tasks" })?.value
            if tasks == GaryxRecentThreadFilter.nonTask.tasksQueryValue {
                selectedRequestStarted.fulfill()
                return try garyxStubResponse(request, data: chatsPage)
            }
            XCTAssertEqual(tasks, GaryxRecentThreadFilter.all.tasksQueryValue)
            return try garyxStubResponse(request, data: allPage)
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(defaults: defaults, session: session)
        XCTAssertEqual(model.recentThreadFeeds.selectedFilter, .nonTask)
        await model.homeProjectionGateway.waitForIdleForTesting()
        XCTAssertEqual(model.homeThreadListStore.snapshot.selectedRecentFilter, .nonTask)
        XCTAssertEqual(
            model.homeProjectionGateway.snapshotEmitCount,
            1,
            "model init must not publish an intermediate All snapshot"
        )

        let refresh = Task { @MainActor in
            await model.refreshThreads(source: .userPullToRefresh)
        }
        await fulfillment(of: [selectedRequestStarted], timeout: 2)
        await refresh.value
        if let auxiliaryTask = model.auxiliaryAllRecentThreadsRefreshTask {
            await auxiliaryTask.value
        }

        XCTAssertEqual(model.visibleRecentThreadIds, ["thread-restored-chat"])
        XCTAssertEqual(
            model.allRecentThreadIds,
            ["thread-restored-task", "thread-restored-chat"]
        )
    }

    func testGlobalSelectionPersistsAcrossModelAndGatewayReset() throws {
        let suiteName = "GaryxHomeThreadListRefreshCommitTests.persistence.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("http://gateway-a.example.test", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let model = makeModel(defaults: defaults)
        model.gatewayURL = ""
        model.selectRecentThreadFilter(.nonTask)
        XCTAssertEqual(
            defaults.string(forKey: GaryxMobileSettingsKeys.recentThreadFilter),
            "nonTask"
        )
        XCTAssertNil(
            defaults.string(
                forKey: model.scopedSettingsKey(GaryxMobileSettingsKeys.recentThreadFilter)
            )
        )

        primeRecentFeed(model, ids: ["thread-task", "thread-chat"], filter: .all)
        primeRecentFeed(model, ids: ["thread-chat"], filter: .nonTask)
        let staleTicket = try XCTUnwrap(model.recentThreadFeeds.requestRefresh(filter: .nonTask))
        model.resetGatewayRuntimeState()

        XCTAssertEqual(model.recentThreadFeeds.selectedFilter, .nonTask)
        XCTAssertTrue(model.recentThreadFeeds.allFeed.orderedThreadIds.isEmpty)
        XCTAssertTrue(model.recentThreadFeeds.nonTaskFeed.orderedThreadIds.isEmpty)
        XCTAssertEqual(
            model.recentThreadFeeds.completeRefresh(
                staleTicket,
                pageIds: ["stale-thread"],
                pageOffset: 0,
                pageCount: 1,
                hasMore: false
            ),
            .abandonedStaleEpoch
        )

        defaults.set("http://gateway-b.example.test", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        let relaunchedModel = makeModel(defaults: defaults)
        XCTAssertEqual(relaunchedModel.recentThreadFeeds.selectedFilter, .nonTask)
        XCTAssertEqual(
            try XCTUnwrap(relaunchedModel.recentThreadFeeds.requestRefresh()).filter,
            .nonTask
        )
    }

    func testArchiveFailureKeepsListSnapshotStableUntilRemoteCommit() async throws {
        let archiveStarted = expectation(description: "archive request started")
        let archiveGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                archiveStarted.fulfill()
                guard archiveGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 500,
                    data: Data(#"{"error":"archive failed"}"#.utf8)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            archiveGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let archived = makeThread(id: "thread-archived", title: "Archived thread")
        let survivor = makeThread(id: "thread-survivor", title: "Surviving thread")
        model.threads = [archived, survivor]
        model.pinnedThreadIds = [archived.id]
        primeRecentFeed(model, ids: [archived.id, survivor.id], filter: .all)
        primeRecentFeed(model, ids: [archived.id, survivor.id], filter: .nonTask)
        await model.homeProjectionGateway.waitForIdleForTesting()
        let initialSnapshot = model.homeThreadListStore.snapshot

        let archiveTask = Task { @MainActor in
            await model.archiveThreadRecord(threadId: archived.id)
        }
        await fulfillment(of: [archiveStarted], timeout: 2)

        // A refresh may finish while the remote operation is still pending.
        // It must keep the existing row visible until the archive commits.
        let concurrentRefresh = try XCTUnwrap(
            model.recentThreadFeeds.requestRefresh(filter: .all)
        )
        XCTAssertEqual(
            model.recentThreadFeeds.completeRefresh(
                concurrentRefresh,
                pageIds: [archived.id, survivor.id],
                pageOffset: 0,
                pageCount: 2,
                hasMore: false
            ),
            .apply(.replaceHead)
        )
        model.commitRefreshedRecentThreadsPage(
            pinsPageThreadIds: [archived.id],
            fetchedThreads: [archived, survivor],
            previousThreadSummaries: [archived, survivor],
            previouslyRemoteBusyThreadIds: [],
            selectionIdForThisRefresh: nil,
            runtimeGeneration: model.gatewayRuntimeGeneration
        )
        await model.homeProjectionGateway.waitForIdleForTesting()

        XCTAssertTrue(model.pendingThreadArchives.isRequestInFlight(threadId: archived.id))
        XCTAssertEqual(model.pinnedThreadIds, [archived.id])
        XCTAssertEqual(model.allRecentThreadIds, [archived.id, survivor.id])
        XCTAssertEqual(model.threads, [archived, survivor])
        XCTAssertEqual(
            model.homeThreadListStore.snapshot,
            initialSnapshot,
            "a remote archive that has not committed must not delete List rows"
        )
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .archiving)
        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.allRows.map(\.id),
            initialSnapshot.sections.allRows.map(\.id),
            "the optimistic exit keeps the physical List item alive until the remote commit"
        )

        archiveGate.signal()
        await archiveTask.value
        await model.homeProjectionGateway.waitForIdleForTesting()

        XCTAssertFalse(model.pendingThreadArchives.contains(threadId: archived.id))
        XCTAssertEqual(model.pinnedThreadIds, [archived.id])
        XCTAssertEqual(model.allRecentThreadIds, [archived.id, survivor.id])
        XCTAssertEqual(model.threads, [archived, survivor])
        XCTAssertEqual(model.homeThreadListStore.snapshot, initialSnapshot)
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .stable)
    }

    func testArchiveSuccessMakesOneListCommitAndInvalidatesEarlierRefresh() async throws {
        let archiveStarted = expectation(description: "archive request started")
        let archiveGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                archiveStarted.fulfill()
                guard archiveGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: Data(
                        #"{"archived":true,"deleted":true,"thread_id":"thread-archived","detached_endpoint_keys":[]}"#.utf8
                    )
                )
            }
            // Post-archive catalog/list refreshes are irrelevant to this
            // interleaving and fail immediately so the real App flow can exit.
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            archiveGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let archived = makeThread(id: "thread-archived", title: "Archived thread")
        let survivor = makeThread(id: "thread-survivor", title: "Surviving thread")
        model.threads = [archived, survivor]
        primeRecentFeed(model, ids: [archived.id, survivor.id], filter: .all)
        primeRecentFeed(model, ids: [archived.id, survivor.id], filter: .nonTask)
        let allMutationSequence = model.recentThreadFeeds.allFeed.pager.localMutationSequence
        let chatsMutationSequence = model.recentThreadFeeds.nonTaskFeed.pager.localMutationSequence

        let archiveTask = Task { @MainActor in
            await model.archiveThreadRecord(threadId: archived.id)
        }
        await fulfillment(of: [archiveStarted], timeout: 2)
        XCTAssertTrue(model.pendingThreadArchives.isRequestInFlight(threadId: archived.id))
        XCTAssertEqual(model.allRecentThreadIds, [archived.id, survivor.id])
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .archiving)

        // The request is pending but has not changed the List. Its single
        // success commit must invalidate this pre-commit server page.
        let staleTicket = try XCTUnwrap(model.recentThreadFeeds.requestRefresh(filter: .all))
        archiveGate.signal()
        let archiveCommitted = await waitUntil {
            model.pendingThreadArchives.isCommitted(threadId: archived.id)
        }
        XCTAssertTrue(
            archiveCommitted,
            "the real archive success path must commit its stale-response tombstone"
        )
        XCTAssertEqual(
            model.recentThreadFeeds.allFeed.pager.localMutationSequence,
            allMutationSequence + 1
        )
        XCTAssertEqual(
            model.recentThreadFeeds.nonTaskFeed.pager.localMutationSequence,
            chatsMutationSequence + 1
        )

        let completion = model.recentThreadFeeds.completeRefresh(
            staleTicket,
            pageIds: [archived.id, survivor.id],
            pageOffset: 0,
            pageCount: 2,
            hasMore: false
        )
        XCTAssertEqual(completion, .abandonedLocalMutation)
        XCTAssertEqual(model.allRecentThreadIds, [survivor.id])
        XCTAssertFalse(model.threads.contains { $0.id == archived.id })

        await archiveTask.value
        await model.homeProjectionGateway.waitForIdleForTesting()
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .stable)
        XCTAssertFalse(
            model.homeThreadListStore.presentationSnapshot.sections.allRows.contains { $0.id == archived.id }
        )
    }

    func testPinMovesPresentationSynchronouslyThenSettlesOnceAfterRemoteCommit() async throws {
        let pinStarted = expectation(description: "pin request started")
        let pinGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "PUT", components.path.hasSuffix("/api/thread-pins/thread-moved") {
                pinStarted.fulfill()
                guard pinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"thread_ids":["thread-moved"]}"#.utf8)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            pinGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let first = makeThread(id: "thread-first", title: "First thread")
        let moved = makeThread(id: "thread-moved", title: "Moved thread")
        let last = makeThread(id: "thread-last", title: "Last thread")
        model.threads = [first, moved, last]
        primeRecentFeed(model, ids: [first.id, moved.id, last.id], filter: .all)
        await model.homeProjectionGateway.waitForIdleForTesting()
        XCTAssertTrue(model.homeThreadListStore.snapshot.sections.pinned.isEmpty)

        model.togglePinnedThread(moved.id)

        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            [moved.id],
            "the row must move in the same synchronous gesture turn, before the actor or network responds"
        )
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: moved.id), .pinning)
        await fulfillment(of: [pinStarted], timeout: 2)

        pinGate.signal()
        let pinSettled = await waitUntil {
            model.homeThreadListStore.rowMotion(threadId: moved.id) == .stable
                && model.homeThreadListStore.snapshot.sections.pinned.map(\.id) == [moved.id]
        }
        XCTAssertTrue(pinSettled)
        XCTAssertEqual(model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id), [moved.id])
    }

    func testUnpinReturnsToFeedRelativePositionWithoutCountingPinnedRows() async throws {
        let unpinStarted = expectation(description: "unpin request started")
        let unpinGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "DELETE", components.path.hasSuffix("/api/thread-pins/thread-two") {
                unpinStarted.fulfill()
                guard unpinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"thread_ids":["thread-zero","thread-one"]}"#.utf8)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            unpinGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let zero = makeThread(id: "thread-zero", title: "Pinned zero")
        let one = makeThread(id: "thread-one", title: "Pinned one")
        let two = makeThread(id: "thread-two", title: "Pinned two")
        let recentA = makeThread(id: "thread-recent-a", title: "Recent A")
        let recentB = makeThread(id: "thread-recent-b", title: "Recent B")
        model.threads = [zero, one, two, recentA, recentB]
        model.pinnedThreadIds = [zero.id, one.id, two.id]
        primeRecentFeed(
            model,
            ids: [zero.id, one.id, two.id, recentA.id, recentB.id],
            filter: .all
        )
        await model.homeProjectionGateway.waitForIdleForTesting()
        XCTAssertEqual(
            model.homeThreadListStore.snapshot.sections.recent.map(\.id),
            [recentA.id, recentB.id]
        )

        model.unpinThread(two.id)

        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            [zero.id, one.id]
        )
        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.recent.map(\.id),
            [two.id, recentA.id, recentB.id],
            "the raw feed's earlier pinned ids must not push the unpinned row down"
        )
        await fulfillment(of: [unpinStarted], timeout: 2)

        unpinGate.signal()
        let settled = await waitUntil {
            model.homeThreadListStore.rowMotion(threadId: two.id) == .stable
                && model.homeThreadListStore.snapshot.sections.recent.map(\.id)
                    == [two.id, recentA.id, recentB.id]
        }
        XCTAssertTrue(settled)
    }

    func testConcurrentPinFailuresRollBackOnlyTheirOwnThreads() async throws {
        let firstStarted = expectation(description: "first pin request started")
        let secondStarted = expectation(description: "second pin request started")
        let firstGate = DispatchSemaphore(value: 0)
        let secondGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "PUT", components.path.hasSuffix("/api/thread-pins/thread-first") {
                firstStarted.fulfill()
                guard firstGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 500,
                    data: Data(#"{"error":"first pin failed"}"#.utf8)
                )
            }
            if request.httpMethod == "PUT", components.path.hasSuffix("/api/thread-pins/thread-second") {
                secondStarted.fulfill()
                guard secondGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 500,
                    data: Data(#"{"error":"second pin failed"}"#.utf8)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            firstGate.signal()
            secondGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let first = makeThread(id: "thread-first", title: "First pin")
        let second = makeThread(id: "thread-second", title: "Second pin")
        model.threads = [first, second]
        primeRecentFeed(model, ids: [first.id, second.id], filter: .all)
        await model.homeProjectionGateway.waitForIdleForTesting()

        model.togglePinnedThread(first.id)
        model.togglePinnedThread(second.id)

        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            [second.id, first.id]
        )
        await fulfillment(of: [firstStarted, secondStarted], timeout: 2)

        firstGate.signal()
        let firstRolledBack = await waitUntil {
            model.homeThreadListStore.rowMotion(threadId: first.id) == .stable
                && model.homeThreadListStore.rowMotion(threadId: second.id) == .pinning
                && model.pinnedThreadIds == [second.id]
        }
        XCTAssertTrue(firstRolledBack)
        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            [second.id],
            "a failed first request must leave the second optimistic pin in place"
        )

        secondGate.signal()
        let secondRolledBack = await waitUntil {
            model.homeThreadListStore.rowMotion(threadId: second.id) == .stable
                && model.pinnedThreadIds.isEmpty
        }
        XCTAssertTrue(secondRolledBack)
        XCTAssertTrue(model.homeThreadListStore.presentationSnapshot.sections.pinned.isEmpty)
    }

    func testUnpinOutsideSelectedFilterCollapsesThenRestoresInPlaceOnFailure() async throws {
        let unpinStarted = expectation(description: "unpin request started")
        let unpinGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "DELETE", components.path.hasSuffix("/api/thread-pins/thread-pinned") {
                unpinStarted.fulfill()
                guard unpinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 500,
                    data: Data(#"{"error":"unpin failed"}"#.utf8)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            unpinGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let pinned = makeThread(id: "thread-pinned", title: "Pinned task")
        let chat = makeThread(id: "thread-chat", title: "Visible chat")
        model.threads = [pinned, chat]
        model.pinnedThreadIds = [pinned.id]
        primeRecentFeed(model, ids: [pinned.id, chat.id], filter: .all)
        primeRecentFeed(model, ids: [chat.id], filter: .nonTask)
        model.recentThreadFeeds.select(.nonTask)
        await model.homeProjectionGateway.waitForIdleForTesting()

        model.unpinThread(pinned.id)

        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: pinned.id), .leavingFilteredList)
        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            [pinned.id],
            "the source item remains physically present while its content collapses"
        )
        await fulfillment(of: [unpinStarted], timeout: 2)

        unpinGate.signal()
        let unpinRolledBack = await waitUntil {
            model.homeThreadListStore.rowMotion(threadId: pinned.id) == .stable
                && model.pinnedThreadIds == [pinned.id]
        }
        XCTAssertTrue(unpinRolledBack)
        XCTAssertEqual(model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id), [pinned.id])
        XCTAssertEqual(model.homeThreadListStore.presentationSnapshot.sections.recent.map(\.id), [chat.id])
    }

    private func makeModel(
        defaults: UserDefaults? = nil,
        session: URLSession? = nil
    ) -> GaryxMobileModel {
        let resolvedDefaults: UserDefaults
        if let defaults {
            resolvedDefaults = defaults
        } else {
            let suiteName = "GaryxHomeThreadListRefreshCommitTests.\(UUID().uuidString)"
            resolvedDefaults = UserDefaults(suiteName: suiteName)!
            resolvedDefaults.removePersistentDomain(forName: suiteName)
        }
        if resolvedDefaults.string(forKey: GaryxMobileSettingsKeys.gatewayUrl) == nil {
            resolvedDefaults.set(
                "http://gateway.example.test",
                forKey: GaryxMobileSettingsKeys.gatewayUrl
            )
        }
        let clientFactory = session.map { session in
            { (configuration: GaryxGatewayConfiguration) in
                GaryxGatewayClient(
                    configuration: configuration,
                    session: session,
                    retryPolicy: .disabled
                )
            }
        }
        return GaryxMobileModel(defaults: resolvedDefaults, gatewayClientFactory: clientFactory)
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
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }

    private func primeRecentFeed(
        _ model: GaryxMobileModel,
        ids: [String],
        filter: GaryxRecentThreadFilter
    ) {
        var feeds = model.recentThreadFeeds
        let ticket = feeds.requestRefresh(filter: filter)!
        feeds.completeRefresh(
            ticket,
            pageIds: ids,
            pageOffset: 0,
            pageCount: ids.count,
            hasMore: false
        )
        model.recentThreadFeeds = feeds
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

    private func makeRecentThreadsPageData(rows: [(id: String, title: String)]) throws -> Data {
        try JSONSerialization.data(
            withJSONObject: [
                "threads": rows.map { row in
                    [
                        "thread_id": row.id,
                        "title": row.title,
                        "last_active_at": "2026-07-07T02:00:00Z",
                        "last_message_preview": "",
                    ]
                },
                "count": rows.count,
                "limit": 30,
                "offset": 0,
                "total": rows.count,
                "has_more": false,
            ]
        )
    }

    private func makeStubSession(
        handler: @escaping (URLRequest) throws -> (HTTPURLResponse, Data)
    ) -> URLSession {
        GaryxRecentThreadsURLProtocolStub.requestHandler = handler
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxRecentThreadsURLProtocolStub.self]
        return URLSession(configuration: configuration)
    }

    private func waitUntil(
        timeout: TimeInterval = 2,
        condition: () -> Bool
    ) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        return condition()
    }
}

private enum GaryxRefreshStubError: Error {
    case timedOut
    case missingURL
    case invalidResponse
}

private func garyxStubResponse(
    _ request: URLRequest,
    statusCode: Int = 200,
    data: Data
) throws -> (HTTPURLResponse, Data) {
    guard let url = request.url else { throw GaryxRefreshStubError.missingURL }
    guard let response = HTTPURLResponse(
        url: url,
        statusCode: statusCode,
        httpVersion: nil,
        headerFields: ["Content-Type": "application/json"]
    ) else {
        throw GaryxRefreshStubError.invalidResponse
    }
    return (response, data)
}

private final class GaryxRecentThreadsURLProtocolStub: URLProtocol {
    static var requestHandler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let requestHandler = Self.requestHandler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        let request = request
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            do {
                let (response, data) = try requestHandler(request)
                self.client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                self.client?.urlProtocol(self, didLoad: data)
                self.client?.urlProtocolDidFinishLoading(self)
            } catch {
                self.client?.urlProtocol(self, didFailWithError: error)
            }
        }
    }

    override func stopLoading() {}
}

private final class GaryxLockedCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0

    @discardableResult
    func increment() -> Int {
        lock.lock()
        count += 1
        let next = count
        lock.unlock()
        return next
    }

    var value: Int {
        lock.lock()
        defer { lock.unlock() }
        return count
    }
}
