import XCTest
@testable import GaryxMobileCore

final class GaryxThreadSummaryGatewayContractTests: XCTestCase {
    func testListThreadSummariesCarriesScopeSearchTasksAndCursorExactly() async throws {
        let (client, session) = makeClient()
        defer {
            ThreadSummaryURLProtocolStub.handler = nil
            session.invalidateAndCancel()
        }
        ThreadSummaryURLProtocolStub.handler = { request in
            let components = try XCTUnwrap(
                request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
            )
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(components.percentEncodedPath, "/base/api/thread-summaries")
            let values = Dictionary(
                uniqueKeysWithValues: (components.queryItems ?? []).map { ($0.name, $0.value) }
            )
            XCTAssertEqual(values["workspace_dir"]!, "/workspace/project & notes")
            XCTAssertEqual(values["tasks"]!, "exclude")
            XCTAssertEqual(values["q"]!, "Straße %_\\")
            XCTAssertEqual(values["limit"]!, "42")
            XCTAssertEqual(values["cursor"]!, "cursor+/=")
            return try response(
                request,
                status: 200,
                body: summaryPageJSON(ids: ["thread::one"])
            )
        }

        let page = try await client.listThreadSummaries(
            workspaceDir: "/workspace/project & notes",
            tasks: .exclude,
            query: "Straße %_\\",
            limit: 42,
            cursor: "cursor+/="
        )
        XCTAssertEqual(page.threads.map(\.id), ["thread::one"])
        XCTAssertEqual(page.threads.first?.title, "thread::one")
    }

    func testCapabilityProbeSendsOnlyLimitAndMapsExactStatusesAndDecodeFailure() async throws {
        let (client, session) = makeClient()
        let counter = LockedCounter()
        defer {
            ThreadSummaryURLProtocolStub.handler = nil
            session.invalidateAndCancel()
        }
        ThreadSummaryURLProtocolStub.handler = { request in
            let call = counter.increment()
            let components = try XCTUnwrap(
                request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
            )
            XCTAssertEqual(components.percentEncodedPath, "/base/api/thread-summaries")
            XCTAssertEqual(components.queryItems, [URLQueryItem(name: "limit", value: "1")])
            switch call {
            case 1: return try response(request, status: 404, body: #"{"error":"missing"}"#)
            case 2: return try response(request, status: 403, body: #"{"error":"denied"}"#)
            case 3: return try response(request, status: 200, body: summaryPageJSON(ids: []))
            default: return try response(request, status: 200, body: #"{"not":"a page"}"#)
            }
        }

        let missing = await client.probeThreadSummariesCapability()
        let forbidden = await client.probeThreadSummariesCapability()
        let supported = await client.probeThreadSummariesCapability()
        let malformed = await client.probeThreadSummariesCapability()
        XCTAssertEqual(missing, .httpStatus(404))
        XCTAssertEqual(forbidden, .httpStatus(403))
        XCTAssertEqual(supported, .httpStatus(200))
        XCTAssertEqual(malformed, .failed)
        XCTAssertEqual(counter.value, 4)
    }

    func testFavoritesIncludeSummariesQueryIsOmittedForLegacyAndPresentForEnhanced() async throws {
        let (client, session) = makeClient()
        let counter = LockedCounter()
        defer {
            ThreadSummaryURLProtocolStub.handler = nil
            session.invalidateAndCancel()
        }
        ThreadSummaryURLProtocolStub.handler = { request in
            let call = counter.increment()
            let components = try XCTUnwrap(
                request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
            )
            XCTAssertEqual(components.percentEncodedPath, "/base/api/thread-favorites/snapshot")
            if call == 1 {
                XCTAssertNil(components.query)
                return try response(request, status: 200, body: favoritesJSON(enhanced: false))
            }
            XCTAssertEqual(
                components.queryItems,
                [URLQueryItem(name: "include_summaries", value: "true")]
            )
            return try response(request, status: 200, body: favoritesJSON(enhanced: true))
        }

        let legacy = try await client.threadFavoritesSnapshot()
        XCTAssertNil(legacy.summaries)
        XCTAssertNil(legacy.summariesTruncated)
        let enhanced = try await client.threadFavoritesSnapshot(includeSummaries: true)
        XCTAssertEqual(enhanced.summaries?.map(\.id), ["thread::favorite"])
        XCTAssertEqual(enhanced.summariesTruncated, false)
        XCTAssertEqual(counter.value, 2)
    }

    func testEnhancedFavoritesEnvelopeRequiresBothFieldsAndMemberSummaries() throws {
        let decoder = JSONDecoder()
        XCTAssertThrowsError(try decoder.decode(
            GaryxThreadFavoritesSnapshot.self,
            from: Data(favoritesJSON(enhanced: true, omitTruncated: true).utf8)
        ))
        XCTAssertThrowsError(try decoder.decode(
            GaryxThreadFavoritesSnapshot.self,
            from: Data(favoritesJSON(enhanced: true, summaryId: "thread::outsider").utf8)
        ))
    }

    func testModeOnlyServerRowFromBothSummaryEnvelopesHasFullFavoriteCapability() throws {
        // The gateway route test seeds a canonical generated-mode-only row and
        // pins these two exact wire fields to false. This Core half proves both
        // envelopes reach the shared capability derivation without reviving
        // the retired gate.
        let decoder = JSONDecoder()
        let page = try decoder.decode(
            GaryxThreadSummariesPage.self,
            from: Data(summaryPageJSON(ids: ["thread::mode-only"]).utf8)
        )
        let favorites = try decoder.decode(
            GaryxThreadFavoritesSnapshot.self,
            from: Data(
                favoritesJSON(
                    enhanced: true,
                    favoriteId: "thread::mode-only",
                    summaryId: "thread::mode-only"
                )
                .utf8
            )
        )
        let rows = try [
            XCTUnwrap(page.threads.first),
            XCTUnwrap(favorites.summaries?.first),
        ]
        for row in rows {
            XCTAssertFalse(row.excludeFromRecent)
            XCTAssertEqual(
                GaryxThreadRowCapabilityDeriver.capabilities(
                    for: row,
                    context: GaryxThreadRowCapabilityContext()
                ).favorite,
                .addAndRemove
            )
        }
    }

    private func makeClient() -> (GaryxGatewayClient, URLSession) {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ThreadSummaryURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        return (
            GaryxGatewayClient(
                configuration: GaryxGatewayConfiguration(
                    baseURL: URL(string: "http://gateway.example.test/base")!
                ),
                session: session,
                retryPolicy: .disabled
            ),
            session
        )
    }
}

private func summaryRowJSON(_ id: String) -> String {
    """
    {"thread_id":"\(id)","title":"\(id)","workspace_dir":"/workspace/project","thread_type":"chat","provider_type":null,"agent_id":null,"created_at":null,"updated_at":null,"message_count":0,"last_user_message":null,"last_assistant_message":null,"last_message_preview":"","recent_run_id":null,"active_run_id":null,"worktree":null,"excluded_from_recent":false}
    """
}

private func summaryPageJSON(ids: [String]) -> String {
    let rows = ids.map(summaryRowJSON).joined(separator: ",")
    return """
    {"threads":[\(rows)],"next_cursor":null,"has_more":false,"store_incarnation_id":"incarnation","server_boot_id":"boot"}
    """
}

private func favoritesJSON(
    enhanced: Bool,
    omitTruncated: Bool = false,
    favoriteId: String = "thread::favorite",
    summaryId: String = "thread::favorite"
) -> String {
    let enhancedFields: String
    if enhanced {
        let truncated = omitTruncated ? "" : ",\"summaries_truncated\":false"
        enhancedFields = ",\"summaries\":[\(summaryRowJSON(summaryId))]\(truncated)"
    } else {
        enhancedFields = ""
    }
    return """
    {"store_incarnation_id":"incarnation","server_boot_id":"boot","revision":1,"thread_ids":["\(favoriteId)"],"favorites":[{"thread_id":"\(favoriteId)","favorited_at":"2026-07-17T00:00:00Z"}],"recent":{"threads":[],"total":0,"truncated":false}\(enhancedFields)}
    """
}

private func response(
    _ request: URLRequest,
    status: Int,
    body: String
) throws -> (HTTPURLResponse, Data) {
    let response = try XCTUnwrap(
        HTTPURLResponse(
            url: try XCTUnwrap(request.url),
            statusCode: status,
            httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )
    )
    return (response, Data(body.utf8))
}

private final class ThreadSummaryURLProtocolStub: URLProtocol {
    static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler else { throw URLError(.badServerResponse) }
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

private final class LockedCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var storage = 0

    func increment() -> Int {
        lock.lock()
        defer { lock.unlock() }
        storage += 1
        return storage
    }

    var value: Int {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }
}
