import XCTest
@testable import GaryxMobileCore

final class GatewayStreamActorTests: XCTestCase {
    func testStaleOverlapSkipsWithoutChangingMessageIds() {
        var processor = GatewayStreamFrameProcessor()

        let first = processor.processPayload(
            framePayload(threadId: "thread-a", basedOnSeq: 2, events: [
                event(seq: 1, role: "user", text: "one"),
                event(seq: 2, role: "assistant", text: "two"),
            ]),
            threadId: "thread-a"
        )
        XCTAssertNil(first.reconnect)
        XCTAssertEqual(committedIds(in: first.actions), [["history:0", "history:1"]])

        let overlap = processor.processPayload(
            framePayload(threadId: "thread-a", basedOnSeq: 3, events: [
                event(seq: 1, role: "user", text: "stale"),
                event(seq: 3, role: "assistant", text: "three"),
            ]),
            threadId: "thread-a"
        )
        XCTAssertNil(overlap.reconnect)
        XCTAssertEqual(committedIds(in: overlap.actions), [["history:2"]])
    }

    func testGapReconnectAppliesPrecedingRowsAndReturnsExactCursor() {
        var processor = GatewayStreamFrameProcessor()

        let result = processor.processPayload(
            framePayload(threadId: "thread-gap", basedOnSeq: 5, events: [
                event(seq: 2, role: "assistant", text: "contiguous"),
                event(seq: 4, role: "assistant", text: "gap"),
            ]),
            threadId: "thread-gap"
        )

        XCTAssertEqual(committedIds(in: result.actions), [["history:1"]])
        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
    }

    func testControlRewriteAppliesPrecedingRowsBeforeRefetch() {
        var processor = GatewayStreamFrameProcessor()

        let result = processor.processPayload(
            framePayload(threadId: "thread-rewrite", basedOnSeq: 12, events: [
                event(seq: 11, role: "assistant", text: "before rewrite"),
                controlEvent(seq: 12, kind: "range_rewrite"),
            ]),
            threadId: "thread-rewrite"
        )

        XCTAssertEqual(result.reconnect, .controlRewrite)
        XCTAssertEqual(result.actions.count, 2)
        guard case .applyCommittedMessages(let messages) = result.actions[0] else {
            return XCTFail("expected committed rows before refetch")
        }
        XCTAssertEqual(messages.map(\.id), ["history:10"])
        XCTAssertEqual(messageTexts(messages), ["before rewrite"])
        XCTAssertEqual(result.actions[1], .refetchAfterControlRewrite)
    }

    func testCapsuleAttachedControlAdvancesCursorWithoutRefetch() {
        var processor = GatewayStreamFrameProcessor()

        let result = processor.processPayload(
            framePayload(threadId: "thread-capsule", basedOnSeq: 13, events: [
                event(seq: 11, role: "assistant", text: "before capsule"),
                controlEvent(seq: 12, kind: "capsule_attached"),
                event(seq: 13, role: "assistant", text: "after capsule"),
            ]),
            threadId: "thread-capsule"
        )

        XCTAssertNil(result.reconnect)
        XCTAssertEqual(committedIds(in: result.actions), [["history:10", "history:11", "history:12"]])
        XCTAssertEqual(committedRoles(in: result.actions), [[.assistant, .system, .assistant]])
        XCTAssertEqual(renderSnapshotSeqs(in: result.actions), [13])
    }

    func testActorAwaitsControlRewriteRefetchBeforeReconnectCursor() async {
        let rewritePayload = framePayload(threadId: "thread-rewrite", basedOnSeq: 12, events: [
            event(seq: 11, role: "assistant", text: "before rewrite"),
            controlEvent(seq: 12, kind: "transcript_reset"),
        ])
        let recorder = GatewayStreamTestRecorder(
            connections: [
                .init(statusCode: 200, lines: [
                    "data: \(rewritePayload)",
                ]),
                .init(statusCode: 404, lines: []),
            ],
            refetchCursor: 42
        )
        let actor = GatewayStreamActor(
            endpoint: endpoint(),
            transport: GatewayStreamTransport { request in
                await recorder.connect(request)
            },
            reconnectDelayNanos: { _ in 0 }
        )

        await actor.run(
            threadId: "thread-rewrite",
            cursorProvider: { 10 },
            shouldContinue: { true },
            actionHandler: { action in
                await recorder.handle(action)
            }
        )

        let requestedCursors = await recorder.requestedCursors()
        let actionNames = await recorder.actionNames()
        let appliedCommittedIds = await recorder.appliedCommittedIds()
        XCTAssertEqual(requestedCursors, [10, 42])
        XCTAssertEqual(actionNames, [
            "applyCommittedMessages",
            "refetchAfterControlRewrite",
            "fallback:notFound404",
        ])
        XCTAssertEqual(appliedCommittedIds, [["history:10"]])
    }

    func testActorFallsBackOnceFor404() async {
        let recorder = GatewayStreamTestRecorder(
            connections: [
                .init(statusCode: 404, lines: []),
            ],
            refetchCursor: 0
        )
        let actor = GatewayStreamActor(
            endpoint: endpoint(),
            transport: GatewayStreamTransport { request in
                await recorder.connect(request)
            },
            reconnectDelayNanos: { _ in 0 }
        )

        await actor.run(
            threadId: "missing-thread",
            cursorProvider: { 7 },
            shouldContinue: { true },
            actionHandler: { action in
                await recorder.handle(action)
            }
        )

        let requestedCursors = await recorder.requestedCursors()
        let actionNames = await recorder.actionNames()
        XCTAssertEqual(requestedCursors, [7])
        XCTAssertEqual(actionNames, ["fallback:notFound404"])
    }

    func testEndpointBuildsInitialWindowRequest() throws {
        let request = try endpoint().threadStreamRequest(
            threadId: "thread::window",
            request: .initial(initialUserTurns: 1)
        )
        let queryItems = queryItems(in: request)

        XCTAssertEqual(queryItems["after_seq"], "0")
        XCTAssertEqual(queryItems["replay_scope"], "initial")
        XCTAssertEqual(queryItems["initial_user_turns"], "1")
        XCTAssertNil(queryItems["render_floor"])
    }

    func testActorGapReconnectResumesWithoutInitialParameters() async {
        let gapPayload = framePayload(threadId: "thread-gap", basedOnSeq: 12, events: [
            event(seq: 10, role: "assistant", text: "first visible row"),
            event(seq: 12, role: "assistant", text: "gap"),
        ])
        let recorder = GatewayStreamTestRecorder(
            connections: [
                .init(statusCode: 200, lines: [
                    "data: \(gapPayload)",
                ]),
                .init(statusCode: 404, lines: []),
            ],
            refetchCursor: 0
        )
        let actor = GatewayStreamActor(
            endpoint: endpoint(),
            transport: GatewayStreamTransport { request in
                await recorder.connect(request)
            },
            reconnectDelayNanos: { _ in 0 }
        )

        await actor.run(
            threadId: "thread-gap",
            requestProvider: {
                GatewayThreadStreamRequestState(
                    afterSeq: 0,
                    replayScope: .initial,
                    initialUserTurns: 1,
                    renderFloor: 7
                )
            },
            shouldContinue: { true },
            actionHandler: { action in
                await recorder.handle(action)
            }
        )

        let requestedCursors = await recorder.requestedCursors()
        let replayScopes = await recorder.requestedQueryValues(name: "replay_scope")
        let initialTurns = await recorder.requestedQueryValues(name: "initial_user_turns")
        let renderFloors = await recorder.requestedQueryValues(name: "render_floor")
        XCTAssertEqual(requestedCursors, [0, 10])
        XCTAssertEqual(replayScopes, ["initial", "resume"])
        XCTAssertEqual(initialTurns, ["1", nil])
        XCTAssertEqual(renderFloors, ["7", "7"])
    }

    private func committedIds(in actions: [GatewayStreamAction]) -> [[String]] {
        actions.compactMap { action in
            guard case .applyCommittedMessages(let messages) = action else { return nil }
            return messages.map(\.id)
        }
    }

    private func messageTexts(_ messages: [GaryxTranscriptMessage]) -> [String] {
        messages.map(\.text)
    }

    private func committedRoles(in actions: [GatewayStreamAction]) -> [[GaryxTranscriptRole]] {
        actions.compactMap { action in
            guard case let .applyCommittedMessages(messages) = action else { return nil }
            return messages.map(\.role)
        }
    }

    private func renderSnapshotSeqs(in actions: [GatewayStreamAction]) -> [Int] {
        actions.compactMap { action in
            guard case let .applyRenderSnapshot(snapshot) = action else { return nil }
            return snapshot.basedOnSeq
        }
    }

    private func endpoint() -> GatewayStreamEndpoint {
        GatewayStreamEndpoint(
            configuration: GaryxGatewayConfiguration(
                baseURL: URL(string: "http://localhost:31337")!,
                authToken: "test-token"
            )
        )
    }

    private func queryItems(in request: URLRequest) -> [String: String] {
        let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
        return Dictionary(uniqueKeysWithValues: items.compactMap { item in
            item.value.map { (item.name, $0) }
        })
    }

    private func framePayload(
        threadId: String,
        basedOnSeq: Int,
        events: [[String: Any]]
    ) -> String {
        jsonString([
            "type": "thread_render_frame",
            "thread_id": threadId,
            "events": events,
            "render_state": [
                "based_on_seq": basedOnSeq,
                "rows": [],
                "tailActivity": "none",
                "progress_locus": "none",
                "visibleMessageIds": [],
                "filtered_placeholders": [],
            ],
        ])
    }

    private func event(seq: Int, role: String, text: String) -> [String: Any] {
        [
            "type": "committed_message",
            "seq": seq,
            "message": [
                "role": role,
                "text": text,
            ],
        ]
    }

    private func controlEvent(seq: Int, kind: String) -> [String: Any] {
        [
            "type": "committed_message",
            "seq": seq,
            "message": [
                "role": "system",
                "kind": "control",
                "text": "",
                "content": [
                    "control": [
                        "kind": kind,
                    ],
                ],
                "message": [
                    "role": "system",
                    "kind": "control",
                    "internal": true,
                    "internal_kind": "control",
                    "control": [
                        "kind": kind,
                    ],
                ],
                "tool_related": false,
                "likely_user_visible": false,
            ],
        ]
    }

    private func jsonString(_ object: [String: Any]) -> String {
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }
}

private struct GatewayStreamTestConnection: Sendable {
    var statusCode: Int
    var lines: [String]
}

private actor GatewayStreamTestRecorder {
    private var connections: [GatewayStreamTestConnection]
    private let refetchCursor: Int
    private var requests: [URLRequest] = []
    private var actions: [GatewayStreamAction] = []

    init(connections: [GatewayStreamTestConnection], refetchCursor: Int) {
        self.connections = connections
        self.refetchCursor = refetchCursor
    }

    func connect(_ request: URLRequest) -> GatewayStreamConnection {
        requests.append(request)
        let connection = connections.isEmpty
            ? GatewayStreamTestConnection(statusCode: 404, lines: [])
            : connections.removeFirst()
        return GatewayStreamConnection(
            statusCode: connection.statusCode,
            lines: AsyncThrowingStream { continuation in
                for line in connection.lines {
                    continuation.yield(line)
                }
                continuation.finish()
            }
        )
    }

    func handle(_ action: GatewayStreamAction) -> GatewayStreamActionResult {
        actions.append(action)
        if action == .refetchAfterControlRewrite {
            return .resumeCursor(refetchCursor)
        }
        return .none
    }

    func requestedCursors() -> [Int] {
        requests.map { request in
            URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?
                .queryItems?
                .first(where: { $0.name == "after_seq" })?
                .value
                .flatMap(Int.init) ?? -1
        }
    }

    func requestedQueryValues(name: String) -> [String?] {
        requests.map { request in
            URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?
                .queryItems?
                .first(where: { $0.name == name })?
                .value
        }
    }

    func actionNames() -> [String] {
        actions.map { action in
            switch action {
            case .applyCommittedMessages:
                return "applyCommittedMessages"
            case .applyRenderSnapshot:
                return "applyRenderSnapshot"
            case .refetchAfterControlRewrite:
                return "refetchAfterControlRewrite"
            case .fallback(.notFound404):
                return "fallback:notFound404"
            case .fallback(.persistentFailure):
                return "fallback:persistentFailure"
            }
        }
    }

    func appliedCommittedIds() -> [[String]] {
        actions.compactMap { action in
            guard case .applyCommittedMessages(let messages) = action else { return nil }
            return messages.map(\.id)
        }
    }
}
