import Foundation
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxGatewayRequestTokenTests: XCTestCase {
    func testQueuedInputFallbackDoesNotCrossGatewayRuntime() async throws {
        let streamInputStarted = expectation(description: "Gateway A queued input request started")
        let streamInputGate = DispatchSemaphore(value: 0)
        let replacementChatStarts = GaryxLockedCounter()
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            switch (url.host, url.path) {
            case ("gateway-a.example.test", "/api/chat/stream-input"):
                streamInputStarted.fulfill()
                guard streamInputGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"status":"inactive"}"#.utf8)
                )
            case ("gateway-b.example.test", "/api/chat/start"):
                replacementChatStarts.increment()
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"status":"accepted","run_id":"run-on-b","thread_id":"shared-thread"}"#.utf8)
                )
            default:
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
        }
        defer {
            streamInputGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.gatewayURL = "http://gateway-a.example.test"
        let sharedThread = makeThread(id: "shared-thread", title: "Shared thread ID")
        model.seedThreadSummariesForTesting([sharedThread])
        model.selectedThread = sharedThread
        let queuedInputTask = Task { @MainActor in
            await model.queueRemoteInput("Queue on Gateway A", attachments: [], in: sharedThread)
        }
        await fulfillment(of: [streamInputStarted], timeout: 2)

        model.resetGatewayRuntimeState()
        model.gatewayURL = "http://gateway-b.example.test"
        model.seedThreadSummariesForTesting([sharedThread])
        model.selectedThread = sharedThread
        streamInputGate.signal()
        await queuedInputTask.value

        XCTAssertEqual(
            replacementChatStarts.value,
            0,
            "Gateway A's fallback input must not start a chat on Gateway B"
        )
        XCTAssertTrue(model.pendingQueuedInputsByIntentId.isEmpty)
    }

    func testSameURLHeaderChangeRotatesGatewayRequestToken() async throws {
        let replacementConnectStarted = expectation(description: "replacement header connect request started")
        let replacementConnectGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            guard url.path == "/api/status",
                  request.value(forHTTPHeaderField: "X-Environment") == "B" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            replacementConnectStarted.fulfill()
            guard replacementConnectGate.wait(timeout: .now() + 5) == .success else {
                throw GaryxRefreshStubError.timedOut
            }
            return try garyxStubResponse(
                request,
                statusCode: 503,
                data: Data(#"{"error":"replacement gateway unavailable"}"#.utf8)
            )
        }
        defer {
            replacementConnectGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.gatewayHeaders = "X-Environment=A"
        model.loadGatewayScopedUserState(fallbackToLegacy: false)
        let originalGeneration = model.gatewayRequestToken

        model.gatewayHeaders = "X-Environment=B"
        let connectTask = Task { @MainActor in
            await model.connectAndRefresh()
        }
        await fulfillment(of: [replacementConnectStarted], timeout: 2)

        XCTAssertNotEqual(model.gatewayRequestToken, originalGeneration)

        replacementConnectGate.signal()
        await connectTask.value
    }

    func testSameURLCredentialChangeInvalidatesInFlightDraftCreation() async throws {
        let createStarted = expectation(description: "credential A create request started")
        let replacementConnectStarted = expectation(description: "credential B connect request started")
        let createGate = DispatchSemaphore(value: 0)
        let replacementConnectGate = DispatchSemaphore(value: 0)
        let replacementChatStarts = GaryxLockedCounter()
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            let authorization = request.value(forHTTPHeaderField: "Authorization")
            let environment = request.value(forHTTPHeaderField: "X-Environment")
            switch (url.path, authorization, environment) {
            case ("/api/threads", "Bearer token-a", "stable"):
                createStarted.fulfill()
                guard createGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"thread_id":"thread-from-a","title":"Credential A thread"}"#.utf8)
                )
            case ("/api/status", "Bearer token-b", "stable"):
                replacementConnectStarted.fulfill()
                guard replacementConnectGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 503,
                    data: Data(#"{"error":"replacement gateway unavailable"}"#.utf8)
                )
            case ("/api/chat/start", "Bearer token-b", "stable"):
                replacementChatStarts.increment()
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"status":"accepted","run_id":"run-on-b","thread_id":"thread-from-a"}"#.utf8)
                )
            default:
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
        }
        defer {
            createGate.signal()
            replacementConnectGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.gatewayURL = "http://gateway.example.test"
        model.gatewayAuthToken = "token-a"
        model.gatewayHeaders = "X-Environment=stable"
        model.loadGatewayScopedUserState(fallbackToLegacy: false)
        let sendTask = Task { @MainActor in
            await model.send("Hello from credential A")
        }
        await fulfillment(of: [createStarted], timeout: 2)

        model.gatewayAuthToken = "token-b"
        let connectTask = Task { @MainActor in
            await model.connectAndRefresh()
        }
        await fulfillment(of: [replacementConnectStarted], timeout: 2)
        createGate.signal()
        await sendTask.value

        XCTAssertEqual(
            replacementChatStarts.value,
            0,
            "a same-URL credential change must invalidate the old create request"
        )
        XCTAssertNil(model.selectedThread)
        XCTAssertEqual(model.threadSummaryCache.count, 0)

        replacementConnectGate.signal()
        await connectTask.value
    }

    func testChatStartResponseDoesNotCommitAfterGatewaySwitch() async throws {
        let chatStartStarted = expectation(description: "Gateway A chat start request started")
        let chatStartGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            guard url.host == "gateway-a.example.test", url.path == "/api/chat/start" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            chatStartStarted.fulfill()
            guard chatStartGate.wait(timeout: .now() + 5) == .success else {
                throw GaryxRefreshStubError.timedOut
            }
            return try garyxStubResponse(
                request,
                data: Data(#"{"status":"accepted","run_id":"run-on-a","thread_id":"thread-on-a"}"#.utf8)
            )
        }
        defer {
            chatStartGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.gatewayURL = "http://gateway-a.example.test"
        let thread = makeThread(id: "thread-on-a", title: "Gateway A thread")
        model.seedThreadSummariesForTesting([thread])
        model.selectedThread = thread
        let sendTask = Task { @MainActor in
            await model.send("Hello on Gateway A")
        }
        await fulfillment(of: [chatStartStarted], timeout: 2)

        model.resetGatewayRuntimeState()
        model.gatewayURL = "http://gateway-b.example.test"
        chatStartGate.signal()
        await sendTask.value

        XCTAssertTrue(model.runTracker.busyThreadIds.isEmpty)
        XCTAssertNil(model.selectedThread)
        XCTAssertEqual(model.threadSummaryCache.count, 0)
        XCTAssertNil(model.lastError)
    }

    func testDraftThreadCreationDoesNotStartChatOnReplacementGateway() async throws {
        let createStarted = expectation(description: "Gateway A create request started")
        let createGate = DispatchSemaphore(value: 0)
        let replacementChatStarts = GaryxLockedCounter()
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            switch (url.host, url.path) {
            case ("gateway-a.example.test", "/api/threads"):
                createStarted.fulfill()
                guard createGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"thread_id":"thread-from-a","title":"Gateway A thread"}"#.utf8)
                )
            case ("gateway-b.example.test", "/api/chat/start"):
                replacementChatStarts.increment()
                return try garyxStubResponse(
                    request,
                    data: Data(#"{"status":"started","run_id":"run-on-b","thread_id":"thread-from-a"}"#.utf8)
                )
            default:
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
        }
        defer {
            createGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.gatewayURL = "http://gateway-a.example.test"
        let sendTask = Task { @MainActor in
            await model.send("Hello from the draft")
        }
        await fulfillment(of: [createStarted], timeout: 2)

        model.resetGatewayRuntimeState()
        model.gatewayURL = "http://gateway-b.example.test"
        createGate.signal()
        await sendTask.value

        XCTAssertEqual(
            replacementChatStarts.value,
            0,
            "a thread created by Gateway A must never be sent to Gateway B"
        )
        XCTAssertNil(model.selectedThread)
        XCTAssertEqual(model.threadSummaryCache.count, 0)
        XCTAssertNil(model.lastError)
    }

    func testDirectThreadCreationDoesNotCommitResponseFromSupersededGateway() async throws {
        let createStarted = expectation(description: "Gateway A direct create request started")
        let createGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            guard url.host == "gateway-a.example.test", url.path == "/api/threads" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            createStarted.fulfill()
            guard createGate.wait(timeout: .now() + 5) == .success else {
                throw GaryxRefreshStubError.timedOut
            }
            return try garyxStubResponse(
                request,
                data: Data(#"{"thread_id":"thread-from-a","title":"Gateway A thread"}"#.utf8)
            )
        }
        defer {
            createGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.gatewayURL = "http://gateway-a.example.test"
        let createTask = Task { @MainActor in
            await model.createThread(workspaceOverride: nil)
        }
        await fulfillment(of: [createStarted], timeout: 2)

        model.resetGatewayRuntimeState()
        model.gatewayURL = "http://gateway-b.example.test"
        createGate.signal()
        await createTask.value

        XCTAssertNil(model.selectedThread)
        XCTAssertEqual(model.threadSummaryCache.count, 0)
        XCTAssertNil(model.lastError)
    }

    func testOptimisticTitleResponseDoesNotCrossGatewayRuntime() async throws {
        let updateStarted = expectation(description: "Gateway A title update started")
        let updateGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            guard url.host == "gateway-a.example.test",
                  url.path == "/api/threads/shared-thread",
                  request.httpMethod == "PATCH" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            updateStarted.fulfill()
            guard updateGate.wait(timeout: .now() + 5) == .success else {
                throw GaryxRefreshStubError.timedOut
            }
            return try garyxStubResponse(
                request,
                data: Data(#"{"thread_id":"shared-thread","title":"Gateway A title"}"#.utf8)
            )
        }
        defer {
            updateGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.gatewayURL = "http://gateway-a.example.test"
        let original = makeThread(id: "shared-thread", title: "Original")
        model.seedThreadSummariesForTesting([original])
        model.selectedThread = original
        let updateTask = Task { @MainActor in
            await model.renameSelectedThread(to: "Optimistic A title")
        }
        await fulfillment(of: [updateStarted], timeout: 2)

        model.resetGatewayRuntimeState()
        model.gatewayURL = "http://gateway-b.example.test"
        let replacement = makeThread(id: "shared-thread", title: "Gateway B title")
        model.seedThreadSummariesForTesting([replacement])
        model.selectedThread = replacement
        updateGate.signal()
        await updateTask.value

        XCTAssertEqual(model.cachedThreadSummary(for: replacement.id)?.title, replacement.title)
        XCTAssertEqual(model.selectedThread?.title, replacement.title)
        XCTAssertNil(model.lastError)
    }

    private func makeModel(session: URLSession) -> GaryxMobileModel {
        let suiteName = "GaryxGatewayRequestTokenTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(
            "http://gateway.example.test",
            forKey: GaryxMobileSettingsKeys.gatewayUrl
        )
        return GaryxMobileModel(
            defaults: defaults,
            gatewayClientFactory: { configuration in
                GaryxGatewayClient(
                    configuration: configuration,
                    session: session,
                    retryPolicy: .disabled
                )
            }
        )
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

    private func makeStubSession(
        handler: @escaping (URLRequest) throws -> (HTTPURLResponse, Data)
    ) -> URLSession {
        GaryxRecentThreadsURLProtocolStub.requestHandler = handler
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxRecentThreadsURLProtocolStub.self]
        return URLSession(configuration: configuration)
    }
}

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
        model.seedThreadSummariesForTesting([pinned, recent])
        model.applyPinnedThreadIds([pinned.id])
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
            bundle: makeGaryxTestRecentRefreshBundle(
                threadIds: page.threads.map(\.id),
                storeIncarnationId: page.storeIncarnationId,
                serverBootId: page.serverBootId,
                hasMore: page.hasMore,
                nextCursor: page.nextCursor
            )
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
            model.residentRecentThreadSummaries.contains { $0.id == pinned.id },
            "pre-await fetched summaries must not resurrect an archived thread"
        )
        XCTAssertEqual(model.allRecentThreadIds, [recent.id])
        XCTAssertNil(model.cachedThreadSummary(for: incoming.id))
    }

    func testCommitAppliesPinsPageAndThreadsWithoutPendingArchives() throws {
        let model = makeModel()
        let pinned = makeThread(id: "thread-pinned", title: "Pinned build")
        let recent = makeThread(id: "thread-recent", title: "Recent chat")
        let page = try makeRecentThreadsPage(threads: [pinned, recent])

        let ticket = model.recentThreadFeeds.requestRefresh(filter: .all)!
        let completion = model.recentThreadFeeds.completeRefresh(
            ticket,
            bundle: makeGaryxTestRecentRefreshBundle(
                threadIds: page.threads.map(\.id),
                storeIncarnationId: page.storeIncarnationId,
                serverBootId: page.serverBootId,
                hasMore: page.hasMore,
                nextCursor: page.nextCursor
            )
        )
        XCTAssertEqual(completion, .applied)

        model.commitRefreshedRecentThreadsPage(
            pinsPageThreadIds: [pinned.id],
            fetchedThreads: [pinned, recent],
            previousThreadSummaries: [],
            previouslyRemoteBusyThreadIds: [],
            selectionIdForThisRefresh: nil,
            runtimeGeneration: model.gatewayRequestToken
        )

        XCTAssertEqual(model.pinnedThreadIds, [pinned.id])
        XCTAssertEqual(model.allRecentThreadIds, [pinned.id, recent.id])
        XCTAssertEqual(
            model.residentRecentThreadSummaries.map(\.id).sorted(),
            [pinned.id, recent.id].sorted()
        )
    }

    /// The archive-resolved interleaving (review #TASK-1804 round 3) is
    /// gated in Core (`abandonedLocalMutation`); what the app must
    /// guarantee is that local list surgery actually marks the pager.
    func testLocalListSurgeryMarksThePagerMutationSequence() {
        let model = makeModel()
        let thread = makeThread(id: "thread-surgery", title: "Doomed")
        model.seedThreadSummariesForTesting([thread])
        model.applyPinnedThreadIds([thread.id])
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
        model.seedThreadSummariesForTesting([task, chat])
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
        model.seedThreadSummariesForTesting([task, chat])
        primeRecentFeed(model, ids: [task.id, chat.id], filter: .all)
        primeRecentFeed(model, ids: [chat.id], filter: .nonTask)

        XCTAssertTrue(model.applyThreadTitleUpdate(threadId: task.id, title: "New task title"))
        XCTAssertEqual(model.allRecentThreadIds, [task.id, chat.id])
        XCTAssertEqual(model.recentThreadFeeds.nonTaskFeed.orderedThreadIds, [chat.id])
        XCTAssertEqual(model.cachedThreadSummary(for: task.id)?.title, "New task title")
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
                let requestIndex = allRequestCount.increment()
                if requestIndex == 1 {
                    auxiliaryStarted.fulfill()
                    guard auxiliaryGate.wait(timeout: .now() + 5) == .success else {
                        throw GaryxRefreshStubError.timedOut
                    }
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

        let oldGeneration = model.gatewayRequestToken
        model.resetGatewayRuntimeState()
        XCTAssertNotEqual(model.gatewayRequestToken, oldGeneration)
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
        let selectedRequestCount = GaryxLockedCounter()
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
                if selectedRequestCount.increment() == 1 {
                    selectedRequestStarted.fulfill()
                }
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

    func testSelectingFavoritesUsesSnapshotAndAuxiliaryAllWithoutFavoritesRecentRequest() async throws {
        let suiteName = "GaryxHomeThreadListRefreshCommitTests.favorites.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("http://gateway.example.test", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let favoritesSnapshotStarted = expectation(description: "favorites snapshot started")
        let allRecentStarted = expectation(description: "auxiliary All requests started")
        allRecentStarted.expectedFulfillmentCount = 2
        let pinsStarted = expectation(description: "pins request started")
        let favoritesSnapshots = GaryxLockedCounter()
        let allRecentRequests = GaryxLockedCounter()
        let unexpectedRecentRequests = GaryxLockedCounter()
        let allPage = try makeRecentThreadsPageData(rows: [
            (id: "thread-favorite", title: "Favorite thread"),
            (id: "thread-other", title: "Other thread"),
        ])
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "GET", components.path == "/api/thread-favorites/snapshot" {
                if favoritesSnapshots.increment() == 1 {
                    favoritesSnapshotStarted.fulfill()
                }
                return try garyxStubResponse(
                    request,
                    data: try garyxFavoritesSnapshotData(ids: ["thread-favorite"])
                )
            }
            if request.httpMethod == "GET", components.path == "/api/thread-pins" {
                pinsStarted.fulfill()
                return try garyxStubResponse(
                    request,
                    data: try garyxPinsPageData(ids: ["thread-favorite"], revision: 3)
                )
            }
            guard components.path == "/api/recent-threads" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            let tasks = components.queryItems?.first(where: { $0.name == "tasks" })?.value
            guard tasks == GaryxRecentThreadFilter.all.tasksQueryValue else {
                unexpectedRecentRequests.increment()
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            if allRecentRequests.increment() <= 2 {
                allRecentStarted.fulfill()
            }
            return try garyxStubResponse(request, data: allPage)
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(defaults: defaults, session: session)
        model.selectRecentThreadFilter(.favorites)

        await fulfillment(
            of: [favoritesSnapshotStarted, allRecentStarted, pinsStarted],
            timeout: 2
        )
        if let auxiliaryTask = model.auxiliaryAllRecentThreadsRefreshTask {
            await auxiliaryTask.value
        }
        let favoritesSettled = await waitUntil {
            model.threadFavoritesState.rawThreadIds == ["thread-favorite"]
        }
        XCTAssertTrue(favoritesSettled)

        XCTAssertEqual(model.recentThreadFeeds.selectedFilter, .favorites)
        XCTAssertEqual(
            defaults.string(forKey: GaryxMobileSettingsKeys.recentThreadFilter),
            "favorites"
        )
        XCTAssertEqual(unexpectedRecentRequests.value, 0)
        XCTAssertGreaterThanOrEqual(favoritesSnapshots.value, 1)
        XCTAssertGreaterThanOrEqual(allRecentRequests.value, 2)
        XCTAssertEqual(model.visibleRecentThreadIds, ["thread-favorite"])
        XCTAssertEqual(model.allRecentThreadIds, ["thread-favorite", "thread-other"])
        XCTAssertEqual(model.pinnedThreadIds, ["thread-favorite"])
    }

    func testFavoritesSnapshotFailureSurfacesUnavailableAndManualRetryRecovers() async throws {
        let requests = GaryxLockedCounter()
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            guard request.httpMethod == "GET",
                  url.path == "/api/thread-favorites/snapshot" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            if requests.increment() == 1 {
                return try garyxStubResponse(
                    request,
                    statusCode: 500,
                    data: Data(#"{"error":"snapshot unavailable"}"#.utf8)
                )
            }
            return try garyxStubResponse(
                request,
                data: try garyxFavoritesSnapshotData(ids: [])
            )
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.recentThreadFeeds.select(.favorites)
        model.refreshThreadFavoritesSnapshot()
        let failed = await waitUntil {
            model.threadFavoritesState.snapshotFailed
        }
        XCTAssertTrue(failed)
        XCTAssertFalse(model.selectedRecentFeedPresentation.isPrimed)
        XCTAssertTrue(model.selectedRecentFeedPresentation.headFailure)

        model.refreshThreadFavoritesSnapshot()
        let recovered = await waitUntil {
            model.threadFavoritesState.rawRevision == 1
                && !model.threadFavoritesState.snapshotFailed
        }
        XCTAssertTrue(recovered)
        XCTAssertTrue(model.selectedRecentFeedPresentation.isPrimed)
        XCTAssertFalse(model.selectedRecentFeedPresentation.headFailure)
        XCTAssertEqual(requests.value, 2)
    }

    func testAwaitableFavoritesSnapshotWaitsForNetworkSettlement() async throws {
        let snapshotStarted = expectation(description: "favorites snapshot started")
        let snapshotGate = DispatchSemaphore(value: 0)
        let settled = GaryxLockedCounter()
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            if request.httpMethod == "GET", url.path == "/api/thread-summaries" {
                return try garyxStubResponse(request, statusCode: 404, data: Data())
            }
            if request.httpMethod == "GET", url.path == "/api/thread-favorites/snapshot" {
                snapshotStarted.fulfill()
                guard snapshotGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: try garyxFavoritesSnapshotData(ids: ["thread-favorite"])
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            snapshotGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let refresh = Task { @MainActor in
            await model.refreshThreadFavoritesSnapshotAndWait()
            settled.increment()
        }
        await fulfillment(of: [snapshotStarted], timeout: 2)

        XCTAssertEqual(settled.value, 0)
        snapshotGate.signal()
        await refresh.value

        XCTAssertEqual(settled.value, 1)
        XCTAssertEqual(model.threadFavoritesState.rawThreadIds, ["thread-favorite"])
    }

    func testFavoritesIncarnationChangeResetsAnInFlightRecentLane() async throws {
        let firstIncarnation = "11111111-1111-4111-8111-111111111111"
        let secondIncarnation = "33333333-3333-4333-8333-333333333333"
        let snapshots = GaryxLockedCounter()
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            guard request.httpMethod == "GET",
                  url.path == "/api/thread-favorites/snapshot" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            let incarnation = snapshots.increment() == 1
                ? firstIncarnation
                : secondIncarnation
            return try garyxStubResponse(
                request,
                data: try garyxFavoritesSnapshotData(
                    ids: [],
                    storeIncarnationId: incarnation
                )
            )
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.refreshThreadFavoritesSnapshot()
        let firstSnapshotSettled = await waitUntil {
            model.threadFavoritesState.storeIncarnationId == firstIncarnation
        }
        XCTAssertTrue(firstSnapshotSettled)
        primeRecentFeed(model, ids: ["old-thread"], filter: .all)
        let oldEpoch = model.threadFavoritesState.runtimeEpoch
        let interrupted = try XCTUnwrap(model.recentThreadFeeds.requestRefresh(
            filter: .all,
            gatewayScope: model.threadFavoritesState.gatewayScope,
            runtimeEpoch: oldEpoch
        ))
        XCTAssertTrue(model.recentThreadFeeds.allFeed.pager.isRefreshingHead)

        model.refreshThreadFavoritesSnapshot()
        let replacementSnapshotSettled = await waitUntil {
            model.threadFavoritesState.runtimeEpoch == oldEpoch + 1
                && model.threadFavoritesState.storeIncarnationId == secondIncarnation
        }
        XCTAssertTrue(replacementSnapshotSettled)

        XCTAssertFalse(model.recentThreadFeeds.allFeed.pager.isRefreshingHead)
        XCTAssertTrue(model.recentThreadFeeds.allFeed.orderedThreadIds.isEmpty)
        XCTAssertEqual(
            model.recentThreadFeeds.completeRefresh(
                interrupted,
                bundle: makeGaryxTestRecentRefreshBundle(threadIds: ["stale-thread"])
            ),
            .abandonedStaleEpoch
        )
        XCTAssertNotNil(model.recentThreadFeeds.requestRefresh(
            filter: .all,
            gatewayScope: model.threadFavoritesState.gatewayScope,
            runtimeEpoch: model.threadFavoritesState.runtimeEpoch
        ))
    }

    func testCommittedArchiveFiltersALateFavoritesSnapshotEverywhere() async throws {
        let archivedId = "thread-archived-favorite"
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            guard request.httpMethod == "GET",
                  url.path == "/api/thread-favorites/snapshot" else {
                return try garyxStubResponse(request, statusCode: 400, data: Data())
            }
            return try garyxStubResponse(
                request,
                data: try garyxFavoritesSnapshotData(
                    ids: [archivedId],
                    rows: [(id: archivedId, title: "Archived favorite")]
                )
            )
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.pendingThreadArchives.commitArchive(threadId: archivedId)
        model.recentThreadFeeds.select(.favorites)
        model.refreshThreadFavoritesSnapshot()
        let snapshotSettled = await waitUntil {
            model.threadFavoritesState.rawThreadIds == [archivedId]
        }
        XCTAssertTrue(snapshotSettled)

        XCTAssertTrue(model.favoriteThreads.isEmpty)
        XCTAssertFalse(model.visibleRecentThreadIds.contains(archivedId))
        XCTAssertFalse(
            model.residentRecentThreadSummaries.contains { $0.id == archivedId }
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
                bundle: makeGaryxTestRecentRefreshBundle(threadIds: ["stale-thread"])
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
        let archiveAttempts = GaryxLockedCounter()
        let favoritesSnapshots = GaryxLockedCounter()
        let allReplacements = GaryxLockedCounter()
        let chatsReplacements = GaryxLockedCounter()
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                archiveAttempts.increment()
                if archiveAttempts.value == 1 {
                    archiveStarted.fulfill()
                    guard archiveGate.wait(timeout: .now() + 5) == .success else {
                        throw GaryxRefreshStubError.timedOut
                    }
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 500,
                    data: Data(#"{"error":"archive failed"}"#.utf8)
                )
            }
            if request.httpMethod == "GET", components.path == "/api/thread-favorites/snapshot" {
                favoritesSnapshots.increment()
            }
            if request.httpMethod == "GET", components.path == "/api/recent-threads" {
                switch components.queryItems?.first(where: { $0.name == "tasks" })?.value {
                case "include": allReplacements.increment()
                case "exclude": chatsReplacements.increment()
                default: break
                }
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            archiveGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 0
        let archived = makeThread(id: "thread-archived", title: "Archived thread")
        let survivor = makeThread(id: "thread-survivor", title: "Surviving thread")
        model.seedThreadSummariesForTesting([archived, survivor])
        model.applyPinnedThreadIds([archived.id])
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
                bundle: makeGaryxTestRecentRefreshBundle(
                    threadIds: [archived.id, survivor.id]
                )
            ),
            .applied
        )
        model.commitRefreshedRecentThreadsPage(
            pinsPageThreadIds: [archived.id],
            fetchedThreads: [archived, survivor],
            previousThreadSummaries: [archived, survivor],
            previouslyRemoteBusyThreadIds: [],
            selectionIdForThisRefresh: nil,
            runtimeGeneration: model.gatewayRequestToken
        )
        await model.homeProjectionGateway.waitForIdleForTesting()

        XCTAssertTrue(model.pendingThreadArchives.isRequestInFlight(threadId: archived.id))
        XCTAssertEqual(model.pinnedThreadIds, [archived.id])
        XCTAssertEqual(model.allRecentThreadIds, [archived.id, survivor.id])
        XCTAssertEqual(
            model.threadSummaryCache.summaries(for: [archived.id, survivor.id]),
            [archived, survivor]
        )
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
        XCTAssertEqual(
            model.threadSummaryCache.summaries(for: [archived.id, survivor.id]),
            [archived, survivor]
        )
        XCTAssertEqual(model.homeThreadListStore.snapshot.sections, initialSnapshot.sections)
        XCTAssertTrue(
            model.recentThreadFeeds.allFeed.headFailure,
            "an ambiguous archive must force a replacement even when reconstruction fails"
        )
        XCTAssertTrue(
            model.recentThreadFeeds.nonTaskFeed.headFailure,
            "the non-selected feed must be reconstructed too"
        )
        let reconstructionIssued = await waitUntil {
            favoritesSnapshots.value == 1
                && allReplacements.value == 1
                && chatsReplacements.value == 1
        }
        XCTAssertTrue(reconstructionIssued)
        XCTAssertEqual(favoritesSnapshots.value, 1)
        XCTAssertEqual(allReplacements.value, 1)
        XCTAssertEqual(chatsReplacements.value, 1)
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .stable)
        XCTAssertEqual(archiveAttempts.value, 6)
    }

    func testAmbiguousDeleteReconstructsFavoritesAndBothFeedsWithoutLocalDeletion() async throws {
        let deleteAttempts = GaryxLockedCounter()
        let favoritesSnapshots = GaryxLockedCounter()
        let allRequests = GaryxLockedCounter()
        let chatsRequests = GaryxLockedCounter()
        let visibleIds = ["thread-delete", "thread-survivor"]
        let recentData = try garyxRecentThreadsData(ids: visibleIds)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "DELETE",
               components.path.hasSuffix("/api/threads/thread-delete") {
                deleteAttempts.increment()
                throw URLError(.networkConnectionLost)
            }
            if request.httpMethod == "GET", components.path == "/api/thread-favorites/snapshot" {
                favoritesSnapshots.increment()
                return try garyxStubResponse(
                    request,
                    data: try garyxFavoritesSnapshotData(ids: [])
                )
            }
            if request.httpMethod == "GET", components.path == "/api/recent-threads" {
                switch components.queryItems?.first(where: { $0.name == "tasks" })?.value {
                case "include": allRequests.increment()
                case "exclude": chatsRequests.increment()
                default: break
                }
                return try garyxStubResponse(request, data: recentData)
            }
            if request.httpMethod == "GET", components.path == "/api/thread-pins" {
                return try garyxStubResponse(
                    request,
                    data: try garyxPinsPageData(ids: [], revision: 1)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 0
        let deleted = makeThread(id: visibleIds[0], title: "Delete candidate")
        let survivor = makeThread(id: visibleIds[1], title: "Survivor")
        model.seedThreadSummariesForTesting([deleted, survivor])
        model.selectedThread = deleted
        primeRecentFeed(model, ids: visibleIds, filter: .all)
        primeRecentFeed(model, ids: visibleIds, filter: .nonTask)

        await model.deleteThread(deleted)
        let reconstructed = await waitUntil {
            favoritesSnapshots.value == 1
                && allRequests.value == 2
                && chatsRequests.value == 2
        }

        XCTAssertTrue(reconstructed)
        XCTAssertEqual(deleteAttempts.value, 6)
        XCTAssertEqual(favoritesSnapshots.value, 1)
        XCTAssertEqual(allRequests.value, 2, "primary + head verification")
        XCTAssertEqual(chatsRequests.value, 2, "primary + head verification")
        XCTAssertEqual(model.selectedThread?.id, deleted.id)
        XCTAssertNotNil(model.cachedThreadSummary(for: deleted.id))
        XCTAssertEqual(model.allRecentThreadIds, visibleIds)
        XCTAssertEqual(model.recentThreadFeeds.nonTaskFeed.orderedThreadIds, visibleIds)
        XCTAssertFalse(model.recentThreadFeeds.allFeed.forceReplacementPending)
        XCTAssertFalse(model.recentThreadFeeds.nonTaskFeed.forceReplacementPending)
    }

    func testArchiveInProgressResendsSameIdentityThenCommitsUIOnce() async throws {
        let recorder = GaryxLockedLifecycleRequestRecorder()
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                let attempt = try recorder.record(request)
                if attempt == 1 {
                    return try garyxStubResponse(
                        request,
                        statusCode: 409,
                        data: Data(
                            """
                            {
                              "kind": "garyx_api_error",
                              "operation": "thread_archive",
                              "code": "operation_in_progress",
                              "message": "still working"
                            }
                            """.utf8
                        )
                    )
                }
                return try garyxStubResponse(
                    request,
                    data: Data(
                        """
                        {
                          "operation_id": "\(recorder.values.last!.operationId)",
                          "outcome": "applied_changed",
                          "changed": true,
                          "archived": true,
                          "deleted": true,
                          "thread_id": "thread-archived",
                          "detached_endpoint_keys": []
                        }
                        """.utf8
                    )
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 0
        let archived = makeThread(id: "thread-archived", title: "Archived thread")
        let survivor = makeThread(id: "thread-survivor", title: "Surviving thread")
        model.seedThreadSummariesForTesting([archived, survivor])
        primeRecentFeed(model, ids: [archived.id, survivor.id], filter: .all)
        primeRecentFeed(model, ids: [archived.id, survivor.id], filter: .nonTask)

        await model.archiveThreadRecord(threadId: archived.id)

        XCTAssertEqual(recorder.values.count, 2)
        XCTAssertEqual(Set(recorder.values.map(\.operationId)).count, 1)
        XCTAssertEqual(
            Set(recorder.values.map(\.expectedStoreIncarnation)),
            ["11111111-1111-4111-8111-111111111111"]
        )
        XCTAssertEqual(model.allRecentThreadIds, [survivor.id])
        XCTAssertNil(model.cachedThreadSummary(for: archived.id))
        XCTAssertTrue(model.pendingThreadArchives.isCommitted(threadId: archived.id))
    }

    func testArchiveRejectedRestoresRowWithoutAmbiguousReconstruction() async throws {
        let attempts = GaryxLockedCounter()
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                attempts.increment()
                return try garyxStubResponse(
                    request,
                    statusCode: 409,
                    data: Data(
                        """
                        {
                          "kind": "garyx_api_error",
                          "operation": "thread_archive",
                          "code": "rejected_conflict",
                          "message": "thread is active"
                        }
                        """.utf8
                    )
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 0
        let archived = makeThread(id: "thread-archived", title: "Archived thread")
        model.seedThreadSummariesForTesting([archived])
        primeRecentFeed(model, ids: [archived.id], filter: .all)
        primeRecentFeed(model, ids: [archived.id], filter: .nonTask)

        await model.archiveThreadRecord(threadId: archived.id)

        XCTAssertEqual(attempts.value, 1)
        XCTAssertEqual(model.allRecentThreadIds, [archived.id])
        XCTAssertEqual(model.cachedThreadSummary(for: archived.id), archived)
        XCTAssertFalse(model.pendingThreadArchives.contains(threadId: archived.id))
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .stable)
        XCTAssertEqual(model.lastError, "thread is active")
        XCTAssertFalse(model.recentThreadFeeds.allFeed.forceReplacementPending)
        XCTAssertFalse(model.recentThreadFeeds.nonTaskFeed.forceReplacementPending)
        let transaction = model.threadMutationHubStore.value.transactions.values.first {
            $0.kind == .archive(threadId: archived.id)
        }
        XCTAssertEqual(transaction?.phase, .rolledBack(message: nil))
        XCTAssertTrue(
            model.threadMutationHubStore.value.residents.values.allSatisfy {
                $0.pending.isEmpty && $0.barrier == nil
            }
        )
    }

    func testArchiveWrongIncarnationReconstructsTheUnifiedThreadListDomain() async throws {
        let archiveAttempts = GaryxLockedCounter()
        let favoritesSnapshots = GaryxLockedCounter()
        let allRequests = GaryxLockedCounter()
        let chatsRequests = GaryxLockedCounter()
        let replacementIncarnation = "33333333-3333-4333-8333-333333333333"
        let visibleIds = ["thread-archive", "thread-survivor"]
        let recentData = try garyxRecentThreadsData(
            ids: visibleIds,
            storeIncarnationId: replacementIncarnation
        )
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                archiveAttempts.increment()
                return try garyxStubResponse(
                    request,
                    statusCode: 409,
                    data: Data(
                        """
                        {
                          "kind": "garyx_api_error",
                          "operation": "thread_archive",
                          "code": "wrong_incarnation",
                          "message": "thread store identity changed",
                          "current_store_incarnation": "\(replacementIncarnation)"
                        }
                        """.utf8
                    )
                )
            }
            if request.httpMethod == "GET", components.path == "/api/thread-favorites/snapshot" {
                favoritesSnapshots.increment()
                return try garyxStubResponse(
                    request,
                    data: try garyxFavoritesSnapshotData(
                        ids: [],
                        storeIncarnationId: replacementIncarnation
                    )
                )
            }
            if request.httpMethod == "GET", components.path == "/api/recent-threads" {
                switch components.queryItems?.first(where: { $0.name == "tasks" })?.value {
                case "include": allRequests.increment()
                case "exclude": chatsRequests.increment()
                default: break
                }
                return try garyxStubResponse(request, data: recentData)
            }
            if request.httpMethod == "GET", components.path == "/api/thread-pins" {
                return try garyxStubResponse(
                    request,
                    data: try garyxPinsPageData(ids: [], revision: 1)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 0
        let archived = makeThread(id: visibleIds[0], title: "Archive candidate")
        let survivor = makeThread(id: visibleIds[1], title: "Survivor")
        model.seedThreadSummariesForTesting([archived, survivor])
        primeRecentFeed(model, ids: visibleIds, filter: .all)
        primeRecentFeed(model, ids: visibleIds, filter: .nonTask)

        await model.archiveThreadRecord(threadId: archived.id)
        let reconstructed = await waitUntil {
            favoritesSnapshots.value == 1
                && allRequests.value >= 1
                && chatsRequests.value >= 1
        }

        XCTAssertTrue(reconstructed)
        XCTAssertEqual(archiveAttempts.value, 1)
        XCTAssertFalse(model.pendingThreadArchives.contains(threadId: archived.id))
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .stable)
        XCTAssertEqual(model.lastError, "thread store identity changed")
        XCTAssertEqual(model.recentThreadFeeds.allFeed.storeIncarnationId, replacementIncarnation)
        XCTAssertEqual(
            model.recentThreadFeeds.nonTaskFeed.storeIncarnationId,
            replacementIncarnation
        )
        XCTAssertFalse(model.recentThreadFeeds.allFeed.forceReplacementPending)
        XCTAssertFalse(model.recentThreadFeeds.nonTaskFeed.forceReplacementPending)
    }

    func testArchiveCancellationRollsBackTheUnifiedMutationWithoutReconstruction() async throws {
        let archiveStarted = expectation(description: "archive retry lane started")
        let attempts = GaryxLockedCounter()
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                attempts.increment()
                archiveStarted.fulfill()
                return try garyxStubResponse(
                    request,
                    statusCode: 409,
                    data: Data(
                        """
                        {
                          "kind": "garyx_api_error",
                          "operation": "thread_archive",
                          "code": "operation_in_progress",
                          "message": "still working"
                        }
                        """.utf8
                    )
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 5_000_000_000
        let archived = makeThread(id: "thread-cancelled", title: "Cancelled archive")
        model.seedThreadSummariesForTesting([archived])
        primeRecentFeed(model, ids: [archived.id], filter: .all)
        primeRecentFeed(model, ids: [archived.id], filter: .nonTask)

        let archiveTask = Task { @MainActor in
            await model.archiveThreadRecord(threadId: archived.id)
        }
        await fulfillment(of: [archiveStarted], timeout: 2)
        archiveTask.cancel()
        await archiveTask.value
        await model.homeProjectionGateway.waitForIdleForTesting()

        XCTAssertEqual(attempts.value, 1)
        XCTAssertFalse(model.pendingThreadArchives.contains(threadId: archived.id))
        XCTAssertEqual(model.homeThreadListStore.rowMotion(threadId: archived.id), .stable)
        XCTAssertFalse(model.recentThreadFeeds.allFeed.forceReplacementPending)
        XCTAssertFalse(model.recentThreadFeeds.nonTaskFeed.forceReplacementPending)
        let transaction = model.threadMutationHubStore.value.transactions.values.first {
            $0.kind == .archive(threadId: archived.id)
        }
        XCTAssertEqual(transaction?.phase, .rolledBack(message: nil))
        XCTAssertNil(model.lastError)
    }

    func testDeleteRejectedRollsBackTheUnifiedMutationHub() async throws {
        let attempts = GaryxLockedCounter()
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "DELETE", components.path.hasSuffix("/api/threads/thread-delete") {
                attempts.increment()
                return try garyxStubResponse(
                    request,
                    statusCode: 409,
                    data: Data(
                        """
                        {
                          "kind": "garyx_api_error",
                          "operation": "thread_delete",
                          "code": "rejected_conflict",
                          "message": "thread is still bound"
                        }
                        """.utf8
                    )
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 0
        let deleted = makeThread(id: "thread-delete", title: "Delete candidate")
        model.seedThreadSummariesForTesting([deleted])
        model.selectedThread = deleted
        primeRecentFeed(model, ids: [deleted.id], filter: .all)
        primeRecentFeed(model, ids: [deleted.id], filter: .nonTask)

        await model.deleteThread(deleted)

        XCTAssertEqual(attempts.value, 1)
        XCTAssertEqual(model.selectedThread?.id, deleted.id)
        XCTAssertNotNil(model.cachedThreadSummary(for: deleted.id))
        XCTAssertEqual(model.lastError, "thread is still bound")
        let transaction = model.threadMutationHubStore.value.transactions.values.first {
            $0.kind == .archive(threadId: deleted.id)
        }
        XCTAssertEqual(transaction?.phase, .rolledBack(message: "thread is still bound"))
        XCTAssertTrue(
            model.threadMutationHubStore.value.residents.values.allSatisfy {
                $0.pending.isEmpty && $0.barrier == nil
            }
        )
    }

    func testDeleteOperationIdConflictReconstructsBeforeClearingTheBarrier() async throws {
        let deleteAttempts = GaryxLockedCounter()
        let favoritesSnapshots = GaryxLockedCounter()
        let allRequests = GaryxLockedCounter()
        let chatsRequests = GaryxLockedCounter()
        let visibleIds = ["thread-delete", "thread-survivor"]
        let recentData = try garyxRecentThreadsData(ids: visibleIds)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "DELETE", components.path.hasSuffix("/api/threads/thread-delete") {
                deleteAttempts.increment()
                return try garyxStubResponse(
                    request,
                    statusCode: 409,
                    data: Data(
                        """
                        {
                          "kind": "garyx_api_error",
                          "operation": "thread_delete",
                          "code": "operation_id_conflict",
                          "message": "operation id was reused"
                        }
                        """.utf8
                    )
                )
            }
            if request.httpMethod == "GET", components.path == "/api/thread-favorites/snapshot" {
                favoritesSnapshots.increment()
                return try garyxStubResponse(
                    request,
                    data: try garyxFavoritesSnapshotData(ids: [])
                )
            }
            if request.httpMethod == "GET", components.path == "/api/recent-threads" {
                switch components.queryItems?.first(where: { $0.name == "tasks" })?.value {
                case "include": allRequests.increment()
                case "exclude": chatsRequests.increment()
                default: break
                }
                return try garyxStubResponse(request, data: recentData)
            }
            if request.httpMethod == "GET", components.path == "/api/thread-pins" {
                return try garyxStubResponse(
                    request,
                    data: try garyxPinsPageData(ids: [], revision: 1)
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        model.lifecycleRetryDelayOverrideNanoseconds = 0
        let deleted = makeThread(id: visibleIds[0], title: "Delete candidate")
        let survivor = makeThread(id: visibleIds[1], title: "Survivor")
        model.seedThreadSummariesForTesting([deleted, survivor])
        model.selectedThread = deleted
        primeRecentFeed(model, ids: visibleIds, filter: .all)
        primeRecentFeed(model, ids: visibleIds, filter: .nonTask)

        await model.deleteThread(deleted)
        let reconstructed = await waitUntil {
            favoritesSnapshots.value == 1
                && allRequests.value == 2
                && chatsRequests.value == 2
        }

        XCTAssertTrue(reconstructed)
        XCTAssertEqual(deleteAttempts.value, 1)
        XCTAssertEqual(model.selectedThread?.id, deleted.id)
        XCTAssertNotNil(model.cachedThreadSummary(for: deleted.id))
        XCTAssertEqual(model.lastError, "operation id was reused")
        let transaction = model.threadMutationHubStore.value.transactions.values.first {
            $0.kind == .archive(threadId: deleted.id)
        }
        XCTAssertEqual(transaction?.phase, .ambiguous)
        XCTAssertTrue(
            model.threadMutationHubStore.value.residents.values.allSatisfy {
                $0.pending.isEmpty && $0.barrier == nil
            }
        )
        XCTAssertFalse(model.recentThreadFeeds.allFeed.forceReplacementPending)
        XCTAssertFalse(model.recentThreadFeeds.nonTaskFeed.forceReplacementPending)
    }

    func testArchiveSuccessMakesOneListCommitAndInvalidatesEarlierRefresh() async throws {
        let archiveStarted = expectation(description: "archive request started")
        let archiveGate = DispatchSemaphore(value: 0)
        let recorder = GaryxLockedLifecycleRequestRecorder()
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "POST", components.path.hasSuffix("/archive") {
                _ = try recorder.record(request)
                archiveStarted.fulfill()
                guard archiveGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    data: Data(
                        """
                        {
                          "operation_id": "\(recorder.values.last!.operationId)",
                          "outcome": "applied_changed",
                          "changed": true,
                          "archived": true,
                          "deleted": true,
                          "thread_id": "thread-archived",
                          "detached_endpoint_keys": []
                        }
                        """.utf8
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
        model.seedThreadSummariesForTesting([archived, survivor])
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
            bundle: makeGaryxTestRecentRefreshBundle(
                threadIds: [archived.id, survivor.id]
            )
        )
        XCTAssertEqual(completion, .abandonedLocalMutation)
        XCTAssertEqual(model.allRecentThreadIds, [survivor.id])
        XCTAssertNil(model.cachedThreadSummary(for: archived.id))

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
        model.seedThreadSummariesForTesting([first, moved, last])
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
        model.seedThreadSummariesForTesting([zero, one, two, recentA, recentB])
        model.applyPinnedThreadIds([zero.id, one.id, two.id])
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
        model.seedThreadSummariesForTesting([first, second])
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

    func testConcurrentPinAndUnpinFailuresRestoreOriginalPinnedOrder() async throws {
        let pinStarted = expectation(description: "pin request started")
        let unpinStarted = expectation(description: "unpin request started")
        let pinGate = DispatchSemaphore(value: 0)
        let unpinGate = DispatchSemaphore(value: 0)
        let session = makeStubSession { request in
            let components = try XCTUnwrap(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            )
            if request.httpMethod == "PUT", components.path.hasSuffix("/api/thread-pins/thread-new") {
                pinStarted.fulfill()
                guard pinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, statusCode: 500, data: Data())
            }
            if request.httpMethod == "DELETE", components.path.hasSuffix("/api/thread-pins/thread-one") {
                unpinStarted.fulfill()
                guard unpinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, statusCode: 500, data: Data())
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            pinGate.signal()
            unpinGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let zero = makeThread(id: "thread-zero", title: "Pinned zero")
        let one = makeThread(id: "thread-one", title: "Pinned one")
        let two = makeThread(id: "thread-two", title: "Pinned two")
        let new = makeThread(id: "thread-new", title: "New pin")
        model.seedThreadSummariesForTesting([zero, one, two, new])
        model.applyPinnedThreadIds([zero.id, one.id, two.id])
        primeRecentFeed(model, ids: [zero.id, one.id, two.id, new.id], filter: .all)
        await model.homeProjectionGateway.waitForIdleForTesting()

        model.togglePinnedThread(new.id)
        model.unpinThread(one.id)

        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            [new.id, zero.id, two.id]
        )
        await fulfillment(of: [pinStarted, unpinStarted], timeout: 2)

        pinGate.signal()
        let pinRolledBack = await waitUntil {
            model.pinnedThreadIds == [zero.id, two.id]
                && model.homeThreadListStore.rowMotion(threadId: one.id) == .pinning
        }
        XCTAssertTrue(pinRolledBack)

        unpinGate.signal()
        let unpinRolledBack = await waitUntil {
            model.pinnedThreadIds == [zero.id, one.id, two.id]
                && model.homeThreadListStore.rowMotion(threadId: one.id) == .stable
        }
        XCTAssertTrue(unpinRolledBack)
        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            [zero.id, one.id, two.id],
            "The failed unpin must restore between its stable neighbors after the earlier pin fails."
        )
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
        model.seedThreadSummariesForTesting([pinned, chat])
        model.applyPinnedThreadIds([pinned.id])
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

    func testPinnedReorderLow200AfterHighGetResendsWithAcceptedFloor() async throws {
        try await assertPinnedReorderBelowFloorCompletion(statusCode: 200)
    }

    func testPinnedReorderLow409AfterHighGetResendsWithAcceptedFloor() async throws {
        try await assertPinnedReorderBelowFloorCompletion(statusCode: 409)
    }

    func testPinnedReorderPlainConflictMergesMembershipAndResendsOnce() async throws {
        let puts = GaryxLockedPinsPutRecorder()
        let conflict = try garyxPinsPageData(
            ids: ["thread-c", "thread-a", "thread-b"],
            revision: 11
        )
        let settledPage = try garyxPinsPageData(
            ids: ["thread-c", "thread-b", "thread-a"],
            revision: 12
        )
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                let index = try puts.record(request)
                return try garyxStubResponse(
                    request,
                    statusCode: index == 1 ? 409 : 200,
                    data: index == 1 ? conflict : settledPage
                )
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 10)
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()

        let settled = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
        }
        XCTAssertTrue(settled)
        XCTAssertEqual(
            puts.values,
            [
                GaryxRecordedPinsPut(
                    threadIds: ["thread-b", "thread-a"],
                    expectedRevision: 10
                ),
                GaryxRecordedPinsPut(
                    threadIds: ["thread-c", "thread-b", "thread-a"],
                    expectedRevision: 11
                ),
            ]
        )
        XCTAssertEqual(model.pinnedThreadIds, ["thread-c", "thread-b", "thread-a"])
    }

    func testPinsGetIssuedAfterDropCannotRevertAfterHigherAck() async throws {
        let putStarted = expectation(description: "reorder started")
        let staleGetStarted = expectation(description: "stale pins get started")
        let putGate = DispatchSemaphore(value: 0)
        let getGate = DispatchSemaphore(value: 0)
        let puts = GaryxLockedPinsPutRecorder()
        let recent = try garyxRecentThreadsData(ids: ["thread-a", "thread-b"])
        let ack = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 12)
        let stale = try garyxPinsPageData(ids: ["thread-a", "thread-b"], revision: 11)
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                _ = try puts.record(request)
                putStarted.fulfill()
                guard putGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, data: ack)
            }
            if request.httpMethod == "GET", path == "/api/recent-threads" {
                return try garyxStubResponse(request, data: recent)
            }
            if request.httpMethod == "GET", path == "/api/thread-pins" {
                staleGetStarted.fulfill()
                guard getGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, data: stale)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            putGate.signal()
            getGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 10)
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()
        await fulfillment(of: [putStarted], timeout: 2)

        let refresh = Task { @MainActor in
            await model.refreshThreads(source: .backgroundLoop)
        }
        await fulfillment(of: [staleGetStarted], timeout: 2)
        putGate.signal()
        let ackSettled = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
                && model.homeThreadListStore.pinnedOrderState.highestObservedRevision == 12
        }
        XCTAssertTrue(ackSettled)
        getGate.signal()
        await refresh.value

        XCTAssertEqual(model.pinnedThreadIds, ["thread-b", "thread-a"])
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.highestObservedRevision, 12)
        XCTAssertEqual(puts.values.count, 1)
    }

    private func assertPinnedReorderBelowFloorCompletion(
        statusCode: Int
    ) async throws {
        let firstPutStarted = expectation(description: "first reorder started")
        let secondPutStarted = expectation(description: "floor-token reorder started")
        let firstPutGate = DispatchSemaphore(value: 0)
        let puts = GaryxLockedPinsPutRecorder()
        let recentData = try garyxRecentThreadsData(ids: ["thread-a", "thread-b"])
        let highPage = try garyxPinsPageData(ids: ["thread-a", "thread-b"], revision: 12)
        let lowPageIds = statusCode == 200
            ? ["thread-b", "thread-a"]
            : ["thread-a", "thread-b"]
        let lowPage = try garyxPinsPageData(ids: lowPageIds, revision: 11)
        let settledPage = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 13)
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "GET", path == "/api/recent-threads" {
                return try garyxStubResponse(request, data: recentData)
            }
            if request.httpMethod == "GET", path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: highPage)
            }
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                let index = try puts.record(request)
                if index == 1 {
                    firstPutStarted.fulfill()
                    guard firstPutGate.wait(timeout: .now() + 5) == .success else {
                        throw GaryxRefreshStubError.timedOut
                    }
                    return try garyxStubResponse(
                        request,
                        statusCode: statusCode,
                        data: lowPage
                    )
                }
                secondPutStarted.fulfill()
                return try garyxStubResponse(request, data: settledPage)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            firstPutGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 10)
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()
        await fulfillment(of: [firstPutStarted], timeout: 2)

        await model.refreshThreads(source: .backgroundLoop)
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.highestObservedRevision, 12)
        XCTAssertEqual(puts.values.count, 1, "the high page cannot dispatch beside the old flight")

        firstPutGate.signal()
        await fulfillment(of: [secondPutStarted], timeout: 2)
        let settled = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
        }
        XCTAssertTrue(settled)
        XCTAssertEqual(
            puts.values,
            [
                GaryxRecordedPinsPut(threadIds: ["thread-b", "thread-a"], expectedRevision: 10),
                GaryxRecordedPinsPut(threadIds: ["thread-b", "thread-a"], expectedRevision: 12),
            ]
        )
    }

    func testPinnedOrderGatewaySwitchDropsLateOldResponseAndAcceptsRevisionZero() async throws {
        let oldPutStarted = expectation(description: "old gateway reorder started")
        let oldResponseReleased = expectation(description: "old gateway response released")
        let oldPutGate = DispatchSemaphore(value: 0)
        let puts = GaryxLockedPinsPutRecorder()
        let newRecent = try garyxRecentThreadsData(ids: ["thread-new"])
        let newPins = try garyxPinsPageData(ids: ["thread-new"], revision: 0)
        let oldAck = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 101)
        let session = makeStubSession { request in
            let url = try XCTUnwrap(request.url)
            if request.httpMethod == "PUT", url.path == "/api/thread-pins" {
                _ = try puts.record(request)
                oldPutStarted.fulfill()
                guard oldPutGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                oldResponseReleased.fulfill()
                return try garyxStubResponse(request, data: oldAck)
            }
            if url.host == "new-gateway.example.test",
               request.httpMethod == "GET",
               url.path == "/api/recent-threads" {
                return try garyxStubResponse(request, data: newRecent)
            }
            if url.host == "new-gateway.example.test",
               request.httpMethod == "GET",
               url.path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: newPins)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            oldPutGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 100)
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()
        await fulfillment(of: [oldPutStarted], timeout: 2)

        model.resetGatewayRuntimeState()
        model.gatewayURL = "http://new-gateway.example.test"
        model.loadGatewayScopedUserState(fallbackToLegacy: false)
        await model.refreshThreads(source: .backgroundLoop)

        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.highestObservedRevision, 0)
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.presentedOrder, ["thread-new"])
        XCTAssertNil(model.homeThreadListStore.pinnedOrderState.outbox)

        oldPutGate.signal()
        await fulfillment(of: [oldResponseReleased], timeout: 2)
        try await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertEqual(puts.values.count, 1, "a late old-identity page must not retry")
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.highestObservedRevision, 0)
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.presentedOrder, ["thread-new"])
    }

    func testHighRevisionRemotePinIsShownAfterLowRevisionLocalUnpinAck() async throws {
        let unpinStarted = expectation(description: "local unpin started")
        let unpinGate = DispatchSemaphore(value: 0)
        let collectionPuts = GaryxLockedCounter()
        let recent = try garyxRecentThreadsData(ids: ["thread-a", "thread-b"])
        let highRemotePin = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 12)
        let lowLocalAck = try garyxPinsPageData(ids: ["thread-a"], revision: 11)
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "DELETE", path == "/api/thread-pins/thread-b" {
                unpinStarted.fulfill()
                guard unpinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, data: lowLocalAck)
            }
            if request.httpMethod == "GET", path == "/api/recent-threads" {
                return try garyxStubResponse(request, data: recent)
            }
            if request.httpMethod == "GET", path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: highRemotePin)
            }
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                collectionPuts.increment()
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            unpinGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 10)
        await model.homeProjectionGateway.waitForIdleForTesting()
        model.unpinThread("thread-b")
        await fulfillment(of: [unpinStarted], timeout: 2)

        await model.refreshThreads(source: .backgroundLoop)
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.presentedOrder, ["thread-a"])
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderState.highestObservedRevision, 12)

        unpinGate.signal()
        let resolved = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.liveMembershipIntentCount == 0
                && model.pinnedThreadIds == ["thread-b", "thread-a"]
        }
        XCTAssertTrue(resolved)
        await model.homeProjectionGateway.waitForIdleForTesting()
        XCTAssertEqual(
            model.homeThreadListStore.presentationSnapshot.sections.pinned.map(\.id),
            ["thread-b", "thread-a"]
        )
        XCTAssertEqual(collectionPuts.value, 0)
    }

    func testReorderWaitsForRealUnpinThenSendsOneReducedFreshFloorPut() async throws {
        let firstPutStarted = expectation(description: "initial reorder started")
        let unpinStarted = expectation(description: "unpin started")
        let followupPutStarted = expectation(description: "reduced reorder started")
        let firstPutGate = DispatchSemaphore(value: 0)
        let unpinGate = DispatchSemaphore(value: 0)
        let puts = GaryxLockedPinsPutRecorder()
        let conflict = try garyxPinsPageData(
            ids: ["thread-a", "thread-b", "thread-c"],
            revision: 11
        )
        let unpinAck = try garyxPinsPageData(ids: ["thread-b", "thread-c"], revision: 12)
        let settle = try garyxPinsPageData(ids: ["thread-c", "thread-b"], revision: 13)
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                let index = try puts.record(request)
                if index == 1 {
                    firstPutStarted.fulfill()
                    guard firstPutGate.wait(timeout: .now() + 5) == .success else {
                        throw GaryxRefreshStubError.timedOut
                    }
                    return try garyxStubResponse(request, statusCode: 409, data: conflict)
                }
                followupPutStarted.fulfill()
                return try garyxStubResponse(request, data: settle)
            }
            if request.httpMethod == "DELETE", path == "/api/thread-pins/thread-a" {
                unpinStarted.fulfill()
                guard unpinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, data: unpinAck)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            firstPutGate.signal()
            unpinGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(
            model,
            ids: ["thread-a", "thread-b", "thread-c"],
            revision: 10
        )
        await model.homeProjectionGateway.waitForIdleForTesting()
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-c", "thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()
        await fulfillment(of: [firstPutStarted], timeout: 2)
        model.unpinThread("thread-a")
        await fulfillment(of: [unpinStarted], timeout: 2)

        firstPutGate.signal()
        let parked = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.pendingSync == .waitingForMembership
        }
        XCTAssertTrue(parked)
        XCTAssertEqual(puts.values.count, 1)

        unpinGate.signal()
        await fulfillment(of: [followupPutStarted], timeout: 2)
        let settled = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
        }
        XCTAssertTrue(settled)
        XCTAssertEqual(
            puts.values,
            [
                GaryxRecordedPinsPut(
                    threadIds: ["thread-c", "thread-b", "thread-a"],
                    expectedRevision: 10
                ),
                GaryxRecordedPinsPut(
                    threadIds: ["thread-c", "thread-b"],
                    expectedRevision: 12
                ),
            ]
        )
    }

    func testFullUnpinClearsRealOutboxWithoutSendingEmptyCollectionPut() async throws {
        let firstPutStarted = expectation(description: "initial reorder started")
        let unpinsStarted = expectation(description: "both unpins started")
        unpinsStarted.expectedFulfillmentCount = 2
        let firstPutGate = DispatchSemaphore(value: 0)
        let unpinAGate = DispatchSemaphore(value: 0)
        let unpinBGate = DispatchSemaphore(value: 0)
        let puts = GaryxLockedPinsPutRecorder()
        let conflict = try garyxPinsPageData(ids: ["thread-a", "thread-b"], revision: 11)
        let unpinA = try garyxPinsPageData(ids: ["thread-b"], revision: 12)
        let unpinB = try garyxPinsPageData(ids: [], revision: 13)
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                _ = try puts.record(request)
                firstPutStarted.fulfill()
                guard firstPutGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, statusCode: 409, data: conflict)
            }
            if request.httpMethod == "DELETE", path == "/api/thread-pins/thread-a" {
                unpinsStarted.fulfill()
                guard unpinAGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, data: unpinA)
            }
            if request.httpMethod == "DELETE", path == "/api/thread-pins/thread-b" {
                unpinsStarted.fulfill()
                guard unpinBGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(request, data: unpinB)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            firstPutGate.signal()
            unpinAGate.signal()
            unpinBGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 10)
        await model.homeProjectionGateway.waitForIdleForTesting()
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()
        await fulfillment(of: [firstPutStarted], timeout: 2)
        model.unpinThread("thread-a")
        model.unpinThread("thread-b")
        await fulfillment(of: [unpinsStarted], timeout: 2)

        firstPutGate.signal()
        let parked = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.pendingSync == .waitingForMembership
                && model.homeThreadListStore.pinnedOrderState.desiredOrder.isEmpty
        }
        XCTAssertTrue(parked)

        unpinAGate.signal()
        let oneIntent = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.liveMembershipIntentCount == 1
        }
        XCTAssertTrue(oneIntent)
        unpinBGate.signal()
        let settled = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
                && model.homeThreadListStore.pinnedOrderState.presentedOrder.isEmpty
        }
        XCTAssertTrue(settled)
        XCTAssertEqual(puts.values.count, 1)
        XCTAssertTrue(puts.values.allSatisfy { !$0.threadIds.isEmpty })
    }

    func testConflictFullUnpinFailureRollbackDispatchesOneRecoveryPutAndDoesNotFlip() async throws {
        let firstPutStarted = expectation(description: "initial reorder started")
        let unpinsStarted = expectation(description: "both failing unpins started")
        unpinsStarted.expectedFulfillmentCount = 2
        let recoveryPutStarted = expectation(description: "rollback recovery reorder started")
        let firstPutGate = DispatchSemaphore(value: 0)
        let unpinGate = DispatchSemaphore(value: 0)
        let puts = GaryxLockedPinsPutRecorder()
        let conflict = try garyxPinsPageData(ids: ["thread-a", "thread-b"], revision: 11)
        let recovered = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 12)
        let recent = try garyxRecentThreadsData(ids: ["thread-a", "thread-b"])
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                let index = try puts.record(request)
                if index == 1 {
                    firstPutStarted.fulfill()
                    guard firstPutGate.wait(timeout: .now() + 5) == .success else {
                        throw GaryxRefreshStubError.timedOut
                    }
                    return try garyxStubResponse(request, statusCode: 409, data: conflict)
                }
                recoveryPutStarted.fulfill()
                return try garyxStubResponse(request, data: recovered)
            }
            if request.httpMethod == "DELETE",
               path == "/api/thread-pins/thread-a" || path == "/api/thread-pins/thread-b" {
                unpinsStarted.fulfill()
                guard unpinGate.wait(timeout: .now() + 5) == .success else {
                    throw GaryxRefreshStubError.timedOut
                }
                return try garyxStubResponse(
                    request,
                    statusCode: 500,
                    data: Data(#"{"error":"synthetic unpin failure"}"#.utf8)
                )
            }
            if request.httpMethod == "GET", path == "/api/recent-threads" {
                return try garyxStubResponse(request, data: recent)
            }
            if request.httpMethod == "GET", path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: recovered)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            firstPutGate.signal()
            unpinGate.signal()
            unpinGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 10)
        await model.homeProjectionGateway.waitForIdleForTesting()
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()
        await fulfillment(of: [firstPutStarted], timeout: 2)
        model.unpinThread("thread-a")
        model.unpinThread("thread-b")
        await fulfillment(of: [unpinsStarted], timeout: 2)

        firstPutGate.signal()
        let parked = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.pendingSync == .waitingForMembership
                && model.homeThreadListStore.pinnedOrderState.desiredOrder.isEmpty
        }
        XCTAssertTrue(parked)
        unpinGate.signal()
        unpinGate.signal()

        await fulfillment(of: [recoveryPutStarted], timeout: 2)
        let settled = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
                && model.pinnedThreadIds == ["thread-b", "thread-a"]
        }
        XCTAssertTrue(settled)
        XCTAssertEqual(puts.values.count, 2)
        XCTAssertEqual(
            puts.values.last,
            GaryxRecordedPinsPut(
                threadIds: ["thread-b", "thread-a"],
                expectedRevision: 11
            )
        )

        await model.refreshThreads(source: .backgroundLoop)
        XCTAssertEqual(model.pinnedThreadIds, ["thread-b", "thread-a"])
        XCTAssertEqual(puts.values.count, 2)
    }

    func testPinResponseBeforeOldReorderCoalescesUntilFlightThenFollowsUpOnce() async throws {
        let firstPutStarted = expectation(description: "old reorder started")
        let pinCompleted = expectation(description: "pin response returned")
        let followupPutStarted = expectation(description: "coalesced reorder started")
        let firstPutGate = DispatchSemaphore(value: 0)
        let puts = GaryxLockedPinsPutRecorder()
        let pinPage = try garyxPinsPageData(
            ids: ["thread-c", "thread-a", "thread-b"],
            revision: 12
        )
        let lowOldAck = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 11)
        let settled = try garyxPinsPageData(
            ids: ["thread-c", "thread-b", "thread-a"],
            revision: 13
        )
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                let index = try puts.record(request)
                if index == 1 {
                    firstPutStarted.fulfill()
                    guard firstPutGate.wait(timeout: .now() + 5) == .success else {
                        throw GaryxRefreshStubError.timedOut
                    }
                    return try garyxStubResponse(request, data: lowOldAck)
                }
                followupPutStarted.fulfill()
                return try garyxStubResponse(request, data: settled)
            }
            if request.httpMethod == "PUT", path == "/api/thread-pins/thread-c" {
                pinCompleted.fulfill()
                return try garyxStubResponse(request, data: pinPage)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            firstPutGate.signal()
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        let ids = ["thread-a", "thread-b", "thread-c"]
        model.seedThreadSummariesForTesting(ids.map { makeThread(id: $0, title: $0) })
        model.applyPinnedThreadIds(["thread-a", "thread-b"], revision: 10)
        primeRecentFeed(model, ids: ids, filter: .all)
        await model.homeProjectionGateway.waitForIdleForTesting()
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()
        await fulfillment(of: [firstPutStarted], timeout: 2)

        model.togglePinnedThread("thread-c")
        await fulfillment(of: [pinCompleted], timeout: 2)
        let coalesced = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.liveMembershipIntentCount == 0
                && model.homeThreadListStore.pinnedOrderState.pendingSync == .coalescedBehindFlight
        }
        XCTAssertTrue(coalesced)
        XCTAssertEqual(puts.values.count, 1)

        firstPutGate.signal()
        await fulfillment(of: [followupPutStarted], timeout: 2)
        let didSettle = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
        }
        XCTAssertTrue(didSettle)
        XCTAssertEqual(
            puts.values,
            [
                GaryxRecordedPinsPut(
                    threadIds: ["thread-b", "thread-a"],
                    expectedRevision: 10
                ),
                GaryxRecordedPinsPut(
                    threadIds: ["thread-c", "thread-b", "thread-a"],
                    expectedRevision: 12
                ),
            ]
        )
    }

    func testPermanentReorderFailurePausesUntilExplicitRefreshWithoutRollback() async throws {
        let puts = GaryxLockedPinsPutRecorder()
        let recent = try garyxRecentThreadsData(ids: ["thread-a", "thread-b"])
        let oldPage = try garyxPinsPageData(ids: ["thread-a", "thread-b"], revision: 10)
        let settledPage = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 11)
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                let index = try puts.record(request)
                if index == 1 {
                    return try garyxStubResponse(
                        request,
                        statusCode: 405,
                        data: Data(#"{"error":"synthetic unsupported route"}"#.utf8)
                    )
                }
                return try garyxStubResponse(request, data: settledPage)
            }
            if request.httpMethod == "GET", path == "/api/recent-threads" {
                return try garyxStubResponse(request, data: recent)
            }
            if request.httpMethod == "GET", path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: oldPage)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let model = makeModel(session: session)
        primePinnedModel(model, ids: ["thread-a", "thread-b"], revision: 10)
        model.beginPinnedOrderDrag()
        model.previewPinnedOrderDrag(["thread-b", "thread-a"])
        model.acceptPinnedOrderDrop()

        let paused = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.pendingSync
                == .pausedPermanent(statusCode: 405)
        }
        XCTAssertTrue(paused)
        XCTAssertEqual(model.pinnedThreadIds, ["thread-b", "thread-a"])
        XCTAssertEqual(model.homeThreadListStore.pinnedOrderSyncStatusLabel, "Sync pending")

        await model.refreshThreads(source: .backgroundLoop)
        XCTAssertEqual(puts.values.count, 1)
        XCTAssertEqual(model.pinnedThreadIds, ["thread-b", "thread-a"])
        XCTAssertNotNil(model.homeThreadListStore.pinnedOrderState.outbox)

        await model.refreshThreads(source: .userPullToRefresh)
        let settled = await waitUntil {
            model.homeThreadListStore.pinnedOrderState.outbox == nil
        }
        XCTAssertTrue(settled)
        XCTAssertEqual(puts.values.count, 2)
        XCTAssertEqual(model.pinnedThreadIds, ["thread-b", "thread-a"])
        XCTAssertNil(model.homeThreadListStore.pinnedOrderSyncStatusLabel)
    }

    func testDurablePinnedOrderOutboxRestoresAcrossModelRestartAndDrains() async throws {
        let suiteName = "GaryxPinnedOrderOutboxTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("http://gateway.example.test", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let puts = GaryxLockedPinsPutRecorder()
        let recent = try garyxRecentThreadsData(ids: ["thread-a", "thread-b"])
        let oldPage = try garyxPinsPageData(ids: ["thread-a", "thread-b"], revision: 10)
        let settledPage = try garyxPinsPageData(ids: ["thread-b", "thread-a"], revision: 11)
        let session = makeStubSession { request in
            let path = try XCTUnwrap(request.url?.path)
            if request.httpMethod == "PUT", path == "/api/thread-pins" {
                let index = try puts.record(request)
                if index == 1 {
                    return try garyxStubResponse(
                        request,
                        statusCode: 405,
                        data: Data(#"{"error":"synthetic old gateway"}"#.utf8)
                    )
                }
                return try garyxStubResponse(request, data: settledPage)
            }
            if request.httpMethod == "GET", path == "/api/recent-threads" {
                return try garyxStubResponse(request, data: recent)
            }
            if request.httpMethod == "GET", path == "/api/thread-pins" {
                return try garyxStubResponse(request, data: oldPage)
            }
            return try garyxStubResponse(request, statusCode: 400, data: Data())
        }
        defer {
            GaryxRecentThreadsURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let firstModel = makeModel(defaults: defaults, session: session)
        primePinnedModel(firstModel, ids: ["thread-a", "thread-b"], revision: 10)
        firstModel.beginPinnedOrderDrag()
        firstModel.previewPinnedOrderDrag(["thread-b", "thread-a"])
        firstModel.acceptPinnedOrderDrop()
        let paused = await waitUntil {
            firstModel.homeThreadListStore.pinnedOrderState.pendingSync
                == .pausedPermanent(statusCode: 405)
        }
        XCTAssertTrue(paused)
        XCTAssertNotNil(
            firstModel.pinnedOrderOutboxStore.loadPinnedOrderOutbox(
                gatewayIdentity: firstModel.currentGatewayScopeId
            )
        )

        let restoredModel = makeModel(defaults: defaults, session: session)
        XCTAssertEqual(restoredModel.pinnedThreadIds, ["thread-b", "thread-a"])
        XCTAssertEqual(restoredModel.homeThreadListStore.pinnedOrderState.pendingSync, .ready)
        await restoredModel.refreshThreads(source: .backgroundLoop)
        let settled = await waitUntil {
            restoredModel.homeThreadListStore.pinnedOrderState.outbox == nil
        }
        XCTAssertTrue(settled)
        XCTAssertEqual(puts.values.count, 2)
        XCTAssertNil(
            restoredModel.pinnedOrderOutboxStore.loadPinnedOrderOutbox(
                gatewayIdentity: restoredModel.currentGatewayScopeId
            )
        )
    }

    private func primePinnedModel(
        _ model: GaryxMobileModel,
        ids: [String],
        revision: Int64
    ) {
        model.seedThreadSummariesForTesting(ids.map { makeThread(id: $0, title: $0) })
        model.applyPinnedThreadIds(ids, revision: revision)
        primeRecentFeed(model, ids: ids, filter: .all)
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
            bundle: makeGaryxTestRecentRefreshBundle(threadIds: ids)
        )
        model.recentThreadFeeds = feeds
    }

    /// Decodes the same wire shape the gateway returns so the commit sees a
    /// real page, not a hand-built lookalike.
    private func makeRecentThreadsPage(threads: [GaryxThreadSummary]) throws -> GaryxRecentThreadsPage {
        let rows = threads.enumerated().map { index, thread in
            """
            {"thread_id": "\(thread.id)", "title": "\(thread.title)",
             "last_active_at": "2026-07-07T02:00:00Z", "last_message_preview": "",
             "activity_seq": \(threads.count - index)}
            """
        }
        let json = """
        {
          "threads": [\(rows.joined(separator: ","))],
          "count": \(threads.count), "limit": 30,
          "total": \(threads.count), "has_more": false, "next_cursor": null,
          "store_incarnation_id": "11111111-1111-4111-8111-111111111111",
          "server_boot_id": "22222222-2222-4222-8222-222222222222"
        }
        """
        return try JSONDecoder().decode(GaryxRecentThreadsPage.self, from: Data(json.utf8))
    }

    private func makeRecentThreadsPageData(rows: [(id: String, title: String)]) throws -> Data {
        try JSONSerialization.data(
            withJSONObject: [
                "threads": rows.enumerated().map { index, row in
                    [
                        "thread_id": row.id,
                        "title": row.title,
                        "last_active_at": "2026-07-07T02:00:00Z",
                        "last_message_preview": "",
                        "activity_seq": rows.count - index,
                    ]
                },
                "count": rows.count,
                "limit": 30,
                "total": rows.count,
                "has_more": false,
                "next_cursor": NSNull(),
                "store_incarnation_id": "11111111-1111-4111-8111-111111111111",
                "server_boot_id": "22222222-2222-4222-8222-222222222222",
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

private func garyxPinsPageData(ids: [String], revision: Int64) throws -> Data {
    try JSONSerialization.data(
        withJSONObject: [
            "thread_ids": ids,
            "revision": revision,
        ]
    )
}

private func garyxRecentThreadsData(
    ids: [String],
    storeIncarnationId: String = "11111111-1111-4111-8111-111111111111"
) throws -> Data {
    try JSONSerialization.data(
        withJSONObject: [
            "threads": ids.enumerated().map { index, id in
                [
                    "thread_id": id,
                    "title": id,
                    "last_active_at": "2026-07-07T02:00:00Z",
                    "last_message_preview": "",
                    "activity_seq": ids.count - index,
                ]
            },
            "count": ids.count,
            "limit": 30,
            "total": ids.count,
            "has_more": false,
            "next_cursor": NSNull(),
            "store_incarnation_id": storeIncarnationId,
            "server_boot_id": "22222222-2222-4222-8222-222222222222",
        ]
    )
}

private func garyxFavoritesSnapshotData(
    ids: [String],
    rows: [(id: String, title: String)] = [],
    storeIncarnationId: String = "11111111-1111-4111-8111-111111111111"
) throws -> Data {
    try JSONSerialization.data(
        withJSONObject: [
            "store_incarnation_id": storeIncarnationId,
            "server_boot_id": "22222222-2222-4222-8222-222222222222",
            "revision": 1,
            "thread_ids": ids,
            "favorites": ids.map { id in
                [
                    "thread_id": id,
                    "favorited_at": "2026-07-16T08:00:00Z",
                ]
            },
            "recent": [
                "threads": rows.enumerated().map { index, row in
                    [
                        "thread_id": row.id,
                        "title": row.title,
                        "last_active_at": "2026-07-16T08:00:00Z",
                        "last_message_preview": "",
                        "activity_seq": rows.count - index,
                    ]
                },
                "total": rows.count,
                "truncated": false,
            ],
        ]
    )
}

private func garyxRequestBodyData(from request: URLRequest) -> Data? {
    if let body = request.httpBody { return body }
    guard let stream = request.httpBodyStream else { return nil }
    stream.open()
    defer { stream.close() }
    var data = Data()
    var buffer = [UInt8](repeating: 0, count: 4096)
    while stream.hasBytesAvailable {
        let count = stream.read(&buffer, maxLength: buffer.count)
        if count > 0 {
            data.append(buffer, count: count)
        } else {
            break
        }
    }
    return data
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

private struct GaryxRecordedLifecycleRequest: Equatable {
    var operationId: String
    var expectedStoreIncarnation: String
}

private final class GaryxLockedLifecycleRequestRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [GaryxRecordedLifecycleRequest] = []

    @discardableResult
    func record(_ request: URLRequest) throws -> Int {
        let data = try XCTUnwrap(garyxRequestBodyData(from: request))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )
        let value = GaryxRecordedLifecycleRequest(
            operationId: try XCTUnwrap(object["operationId"] as? String),
            expectedStoreIncarnation: try XCTUnwrap(
                object["expectedStoreIncarnation"] as? String
            )
        )
        lock.lock()
        recorded.append(value)
        let count = recorded.count
        lock.unlock()
        return count
    }

    var values: [GaryxRecordedLifecycleRequest] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }
}

private struct GaryxRecordedPinsPut: Equatable {
    var threadIds: [String]
    var expectedRevision: Int64
}

private final class GaryxLockedPinsPutRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [GaryxRecordedPinsPut] = []

    @discardableResult
    func record(_ request: URLRequest) throws -> Int {
        let data = try XCTUnwrap(garyxRequestBodyData(from: request))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )
        let value = GaryxRecordedPinsPut(
            threadIds: try XCTUnwrap(object["thread_ids"] as? [String]),
            expectedRevision: try XCTUnwrap((object["expected_revision"] as? NSNumber)?.int64Value)
        )
        lock.lock()
        recorded.append(value)
        let count = recorded.count
        lock.unlock()
        return count
    }

    var values: [GaryxRecordedPinsPut] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }
}
