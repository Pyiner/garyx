import Foundation
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxCapsuleFocusedPreviewLoadingTests: XCTestCase {
    override func tearDown() {
        GaryxCapsulePreviewURLProtocolStub.requestHandler = nil
        super.tearDown()
    }

    func testCatalogSingleFlightPreservesTrailingTriggerAndCommitsNewestResponse() async throws {
        let firstStarted = expectation(description: "first catalog request started")
        let releaseFirst = DispatchSemaphore(value: 0)
        let counter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            XCTAssertEqual(request.url?.path, "/api/capsules")
            let call = counter.increment()
            if call == 1 {
                firstStarted.fulfill()
                guard releaseFirst.wait(timeout: .now() + 5) == .success else {
                    throw GaryxCapsulePreviewStubError.timedOut
                }
                return try self.capsuleResponse(request, revision: 1, title: "First catalog")
            }
            return try self.capsuleResponse(request, revision: 2, title: "Trailing catalog")
        }
        defer {
            releaseFirst.signal()
            session.invalidateAndCancel()
        }
        let model = makeModel(session: session)

        let first = Task { @MainActor in await model.refreshCapsules(reportFailure: false) }
        await fulfillment(of: [firstStarted], timeout: 2)
        let trailing = Task { @MainActor in await model.refreshCapsules(reportFailure: false) }
        await Task.yield()
        releaseFirst.signal()

        _ = await first.value
        _ = await trailing.value
        XCTAssertEqual(counter.value, 2, "a trigger received in flight must cause a second GET")
        XCTAssertEqual(model.capsules.first?.revision, 2)
        XCTAssertEqual(model.capsules.first?.title, "Trailing catalog")
        XCTAssertEqual(model.capsuleCatalogCommittedTicket, model.capsuleCatalogRequestedTicket)
    }

    func testCatalogRevisionChangeProducesNewKeyAndReloadsHTML() async throws {
        let serveCounter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            switch request.url?.path {
            case "/api/capsules":
                return try self.capsuleResponse(request, revision: 2, title: "Revision two")
            case "/api/capsules/capsule-preview/serve":
                let call = serveCounter.increment()
                return try self.response(
                    request,
                    data: Data((call == 1 ? "<html>rev1</html>" : "<html>rev2</html>").utf8),
                    contentType: "text/html"
                )
            default:
                return try self.response(request, statusCode: 400, data: Data())
            }
        }
        defer { session.invalidateAndCancel() }
        let model = makeModel(session: session)
        let selection = GaryxCapsulePreviewSelection(capsule: capsule(revision: 1))
        model.capsules = [selection.fallback]
        let loader = GaryxCapsuleFocusedPreviewLoader(retryPolicy: .init(delays: []))

        let key1 = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: model.capsules,
            retryGeneration: 0
        )
        await loader.reconcile(key: key1, model: model)
        XCTAssertEqual(loader.renderedContent?.html, "<html>rev1</html>")

        _ = await model.refreshCapsules(reportFailure: false)
        let key2 = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: model.capsules,
            retryGeneration: 0
        )
        XCTAssertEqual(key1.projectedRevision, 1)
        XCTAssertEqual(key2.projectedRevision, 2)
        await loader.reconcile(key: key2, model: model)

        XCTAssertEqual(serveCounter.value, 2)
        XCTAssertEqual(loader.renderedContent, .init(html: "<html>rev2</html>", revision: 2))
        XCTAssertEqual(loader.loadStatus.requestedKey, key2)
    }

    func testRevisionTwoFailureKeepsRevisionOneUntilRetrySucceeds() async throws {
        let retrySleepStarted = expectation(description: "retry sleep started")
        let retryGate = GaryxCapsuleAsyncGate()
        let counter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            let call = counter.increment()
            if call == 1 {
                return try self.response(
                    request,
                    data: Data("<html>rev1</html>".utf8),
                    contentType: "text/html"
                )
            }
            if call == 2 {
                return try self.response(
                    request,
                    statusCode: 503,
                    data: Data(#"{"error":"synthetic unavailable"}"#.utf8)
                )
            }
            return try self.response(
                request,
                data: Data("<html>rev2</html>".utf8),
                contentType: "text/html"
            )
        }
        defer { session.invalidateAndCancel() }
        let model = makeModel(session: session)
        let loader = GaryxCapsuleFocusedPreviewLoader(
            retryPolicy: .init(delays: [2]),
            sleeper: { _ in
                retrySleepStarted.fulfill()
                try await retryGate.wait()
            }
        )
        let key1 = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: 1,
            retryGeneration: 0
        )
        let key2 = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: 2,
            retryGeneration: 0
        )

        await loader.reconcile(key: key1, model: model)
        let rev2Task = Task { @MainActor in await loader.reconcile(key: key2, model: model) }
        await fulfillment(of: [retrySleepStarted], timeout: 2)

        XCTAssertEqual(loader.renderedContent, .init(html: "<html>rev1</html>", revision: 1))
        XCTAssertEqual(loader.loadStatus.requestedKey, key2)
        XCTAssertEqual(loader.loadStatus.phase, .failed)
        XCTAssertTrue(loader.loadStatus.failure?.isRetryable == true)

        await retryGate.release()
        await rev2Task.value
        XCTAssertEqual(counter.value, 3)
        XCTAssertEqual(loader.renderedContent, .init(html: "<html>rev2</html>", revision: 2))
        XCTAssertEqual(loader.loadStatus.phase, .loaded)
    }

    func testMissingProjectionStillVisitsServeAnd404BecomesDeleted() async {
        let counter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            _ = counter.increment()
            return try self.response(
                request,
                statusCode: 404,
                data: Data(#"{"error":"synthetic missing"}"#.utf8)
            )
        }
        defer { session.invalidateAndCancel() }
        let model = makeModel(session: session)
        let loader = GaryxCapsuleFocusedPreviewLoader(retryPolicy: .init(delays: [0, 0, 0]))
        let key = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: nil,
            retryGeneration: 0
        )

        await loader.reconcile(key: key, model: model)

        XCTAssertEqual(counter.value, 1)
        XCTAssertNil(loader.renderedContent)
        XCTAssertEqual(loader.loadStatus.phase, .deleted)
        XCTAssertEqual(loader.loadStatus.failure?.kind, .deleted)
    }

    func testRetryExhaustionStopsAfterFourNetworkAttempts() async {
        let counter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            _ = counter.increment()
            return try self.response(
                request,
                statusCode: 503,
                data: Data(#"{"error":"synthetic unavailable"}"#.utf8)
            )
        }
        defer { session.invalidateAndCancel() }
        let model = makeModel(session: session)
        let loader = GaryxCapsuleFocusedPreviewLoader(
            retryPolicy: .init(delays: [2, 5, 10]),
            sleeper: { _ in }
        )
        let key = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: 1,
            retryGeneration: 0
        )

        await loader.reconcile(key: key, model: model)

        XCTAssertEqual(counter.value, 4)
        XCTAssertEqual(loader.loadStatus.attempt, 4)
        XCTAssertEqual(loader.loadStatus.phase, .failed)
        XCTAssertTrue(loader.loadStatus.retryExhausted)
    }

    func testKeySwitchRejectsLateOldHTMLAndNeverDowngradesRenderedContent() async {
        let firstServeStarted = expectation(description: "slow rev1 serve started")
        let releaseFirstServe = DispatchSemaphore(value: 0)
        let counter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            let call = counter.increment()
            if call == 1 {
                firstServeStarted.fulfill()
                guard releaseFirstServe.wait(timeout: .now() + 5) == .success else {
                    throw GaryxCapsulePreviewStubError.timedOut
                }
                return try self.response(
                    request,
                    data: Data("<html>late-rev1</html>".utf8),
                    contentType: "text/html"
                )
            }
            return try self.response(
                request,
                data: Data("<html>rev2</html>".utf8),
                contentType: "text/html"
            )
        }
        defer {
            releaseFirstServe.signal()
            session.invalidateAndCancel()
        }
        let model = makeModel(session: session)
        let loader = GaryxCapsuleFocusedPreviewLoader(retryPolicy: .init(delays: []))
        let key1 = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: 1,
            retryGeneration: 0
        )
        let key2 = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: 2,
            retryGeneration: 0
        )

        let slow = Task { @MainActor in await loader.reconcile(key: key1, model: model) }
        await fulfillment(of: [firstServeStarted], timeout: 2)
        let newest = Task { @MainActor in await loader.reconcile(key: key2, model: model) }
        await newest.value
        XCTAssertEqual(loader.renderedContent, .init(html: "<html>rev2</html>", revision: 2))

        releaseFirstServe.signal()
        await slow.value
        XCTAssertEqual(loader.renderedContent, .init(html: "<html>rev2</html>", revision: 2))
        XCTAssertEqual(loader.loadStatus.requestedKey, key2)
    }

    func testBackgroundDuringBackoffCancelsSleepAndDoesNotIssueAnotherAttempt() async {
        let sleepStarted = expectation(description: "background cancellation sleep started")
        let gate = GaryxCapsuleAsyncGate()
        let counter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            _ = counter.increment()
            return try self.response(
                request,
                statusCode: 503,
                data: Data(#"{"error":"synthetic unavailable"}"#.utf8)
            )
        }
        defer { session.invalidateAndCancel() }
        let model = makeModel(session: session)
        let loader = GaryxCapsuleFocusedPreviewLoader(
            sleeper: { _ in
                sleepStarted.fulfill()
                try await gate.wait()
            }
        )
        let key = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: 1,
            retryGeneration: 0
        )

        let task = Task { @MainActor in await loader.reconcile(key: key, model: model) }
        await fulfillment(of: [sleepStarted], timeout: 2)
        loader.cancelForScene(model: model, event: .sceneBackground)
        await gate.release()
        await task.value

        XCTAssertEqual(counter.value, 1)
        XCTAssertEqual(loader.loadStatus.phase, .paused)
        XCTAssertTrue(loader.needsForegroundResume(for: key))
    }

    func testDismissDuringBackoffCancelsWithoutConvertingCancellationToFailure() async {
        let sleepStarted = expectation(description: "dismiss cancellation sleep started")
        let gate = GaryxCapsuleAsyncGate()
        let counter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            _ = counter.increment()
            return try self.response(
                request,
                statusCode: 503,
                data: Data(#"{"error":"synthetic unavailable"}"#.utf8)
            )
        }
        defer { session.invalidateAndCancel() }
        let model = makeModel(session: session)
        let loader = GaryxCapsuleFocusedPreviewLoader(
            sleeper: { _ in
                sleepStarted.fulfill()
                try await gate.wait()
            }
        )
        let key = GaryxCapsulePreviewLoadKey(
            id: "capsule-preview",
            projectedRevision: 1,
            retryGeneration: 0
        )

        let task = Task { @MainActor in await loader.reconcile(key: key, model: model) }
        await fulfillment(of: [sleepStarted], timeout: 2)
        let failureBeforeDismiss = loader.loadStatus.failure
        loader.cancelForDismiss(model: model)
        await gate.release()
        await task.value

        XCTAssertEqual(counter.value, 1)
        XCTAssertEqual(loader.loadStatus.failure, failureBeforeDismiss)
        XCTAssertEqual(loader.loadStatus.failure?.kind, .retryable)
    }

    func testForegroundCatalogRevisionChangeLoadsOnlyNewKeyInsteadOfRetryingOldKey() async {
        let sleepStarted = expectation(description: "rev1 retry sleep started")
        let gate = GaryxCapsuleAsyncGate()
        let serveCounter = GaryxCapsuleLockedCounter()
        let session = makeSession { request in
            switch request.url?.path {
            case "/api/capsules":
                return try self.capsuleResponse(request, revision: 2, title: "Revision two")
            case "/api/capsules/capsule-preview/serve":
                let call = serveCounter.increment()
                if call == 1 {
                    return try self.response(
                        request,
                        statusCode: 503,
                        data: Data(#"{"error":"synthetic unavailable"}"#.utf8)
                    )
                }
                return try self.response(
                    request,
                    data: Data("<html>rev2</html>".utf8),
                    contentType: "text/html"
                )
            default:
                return try self.response(request, statusCode: 400, data: Data())
            }
        }
        defer { session.invalidateAndCancel() }
        let model = makeModel(session: session)
        let selection = GaryxCapsulePreviewSelection(capsule: capsule(revision: 1))
        model.capsules = [selection.fallback]
        let loader = GaryxCapsuleFocusedPreviewLoader(
            sleeper: { _ in
                sleepStarted.fulfill()
                try await gate.wait()
            }
        )
        let oldKey = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: model.capsules,
            retryGeneration: 0
        )
        let oldTask = Task { @MainActor in await loader.reconcile(key: oldKey, model: model) }
        await fulfillment(of: [sleepStarted], timeout: 2)
        loader.cancelForScene(model: model, event: .sceneInactive)
        await gate.release()
        await oldTask.value

        _ = await model.refreshCapsules(reportFailure: false)
        let newKey = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: model.capsules,
            retryGeneration: 0
        )
        XCTAssertEqual(newKey.projectedRevision, 2)
        XCTAssertNotEqual(newKey.projectedRevision, oldKey.projectedRevision)
        await loader.reconcile(key: newKey, model: model)

        XCTAssertEqual(serveCounter.value, 2, "rev1 must not start a foreground retry cycle")
        XCTAssertEqual(loader.renderedContent, .init(html: "<html>rev2</html>", revision: 2))
    }

    private func capsule(revision: Int, title: String = "Synthetic Capsule") -> GaryxCapsuleSummary {
        GaryxCapsuleSummary(id: "capsule-preview", title: title, revision: revision)
    }

    private func capsuleResponse(
        _ request: URLRequest,
        revision: Int,
        title: String
    ) throws -> (HTTPURLResponse, Data) {
        let data = try JSONSerialization.data(withJSONObject: [
            "capsules": [[
                "id": "capsule-preview",
                "title": title,
                "revision": revision,
            ]],
        ])
        return try response(request, data: data)
    }

    private func response(
        _ request: URLRequest,
        statusCode: Int = 200,
        data: Data,
        contentType: String = "application/json"
    ) throws -> (HTTPURLResponse, Data) {
        guard let url = request.url else { throw GaryxCapsulePreviewStubError.missingURL }
        guard let response = HTTPURLResponse(
            url: url,
            statusCode: statusCode,
            httpVersion: nil,
            headerFields: ["Content-Type": contentType]
        ) else {
            throw GaryxCapsulePreviewStubError.invalidResponse
        }
        return (response, data)
    }

    private func makeModel(session: URLSession) -> GaryxMobileModel {
        let suiteName = "GaryxCapsuleFocusedPreviewLoadingTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        let model = GaryxMobileModel(
            defaults: defaults,
            gatewayClientFactory: { configuration in
                GaryxGatewayClient(
                    configuration: configuration,
                    session: session,
                    retryPolicy: .disabled
                )
            }
        )
        model.gatewayURL = "http://gateway.example.test"
        return model
    }

    private func makeSession(
        handler: @escaping (URLRequest) throws -> (HTTPURLResponse, Data)
    ) -> URLSession {
        GaryxCapsulePreviewURLProtocolStub.requestHandler = handler
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxCapsulePreviewURLProtocolStub.self]
        return URLSession(configuration: configuration)
    }
}

private enum GaryxCapsulePreviewStubError: Error {
    case timedOut
    case missingURL
    case invalidResponse
}

private actor GaryxCapsuleAsyncGate {
    private var isOpen = false

    func wait() async throws {
        while !isOpen {
            try Task.checkCancellation()
            try await Task.sleep(nanoseconds: 5_000_000)
        }
    }

    func release() {
        isOpen = true
    }
}

private final class GaryxCapsulePreviewURLProtocolStub: URLProtocol {
    static var requestHandler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let handler = Self.requestHandler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        let request = request
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            do {
                let (response, data) = try handler(request)
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

private final class GaryxCapsuleLockedCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0

    @discardableResult
    func increment() -> Int {
        lock.lock()
        count += 1
        let value = count
        lock.unlock()
        return value
    }

    var value: Int {
        lock.lock()
        defer { lock.unlock() }
        return count
    }
}
