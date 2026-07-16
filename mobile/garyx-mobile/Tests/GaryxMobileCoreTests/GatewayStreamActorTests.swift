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
        processor.resetConnection(afterSeq: 1, replayScope: .resume)

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
        processor.resetConnection(afterSeq: 10, replayScope: .resume)

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
        processor.resetConnection(afterSeq: 10, replayScope: .resume)

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

    func testRenderOnlyFrameDoesNotAdvanceCommittedFrontier() {
        var processor = GatewayStreamFrameProcessor()

        let result = processor.processPayload(
            framePayload(threadId: "thread-render-only", basedOnSeq: 95, events: []),
            threadId: "thread-render-only"
        )

        XCTAssertEqual(committedIds(in: result.actions), [])
        XCTAssertEqual(renderSnapshotSeqs(in: result.actions), [95])
        XCTAssertEqual(processor.connectionLastSeq, 0)
        XCTAssertTrue(processor.madeProgressOnConnection)
    }

    func testResumeFirstHighSeqReconnectsFromHeldCursor() {
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 94, replayScope: .resume)

        let result = processor.processPayload(
            framePayload(threadId: "thread-resume-gap", basedOnSeq: 113, events: [
                event(seq: 113, role: "assistant", text: "later"),
            ]),
            threadId: "thread-resume-gap"
        )

        XCTAssertEqual(committedIds(in: result.actions), [])
        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 94))
        XCTAssertEqual(processor.connectionLastSeq, 94)
    }

    func testInitialWindowAllowsFirstHighSeq() {
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 0, replayScope: .initial)

        let result = processor.processPayload(
            framePayload(threadId: "thread-initial", basedOnSeq: 12, events: [
                event(seq: 11, role: "user", text: "window start"),
                event(seq: 12, role: "assistant", text: "window next"),
            ]),
            threadId: "thread-initial"
        )

        XCTAssertNil(result.reconnect)
        XCTAssertEqual(committedIds(in: result.actions), [["history:10", "history:11"]])
        XCTAssertEqual(processor.connectionLastSeq, 12)
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
            transport: GatewayStreamTransport { request, semantics in
                XCTAssertEqual(semantics, .readRetryable)
                return await recorder.connect(request)
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
            transport: GatewayStreamTransport { request, semantics in
                XCTAssertEqual(semantics, .readRetryable)
                return await recorder.connect(request)
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

    func testActorResumeAfterSeqDoesNotMaskPersistentFailures() async {
        let recorder = GatewayStreamFailureRecorder()
        let actor = GatewayStreamActor(
            endpoint: endpoint(),
            transport: GatewayStreamTransport { request, semantics in
                XCTAssertEqual(semantics, .readRetryable)
                return try await recorder.connect(request)
            },
            reconnectDelayNanos: { _ in 0 }
        )

        await actor.run(
            threadId: "thread-failing",
            requestProvider: {
                .resume(afterSeq: 7)
            },
            shouldContinue: { true },
            actionHandler: { action in
                await recorder.handle(action)
            }
        )

        let requestedCursors = await recorder.requestedCursors()
        let actionNames = await recorder.actionNames()
        XCTAssertEqual(requestedCursors, [7, 7, 7, 7])
        XCTAssertEqual(actionNames, ["fallback:persistentFailure"])
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
            transport: GatewayStreamTransport { request, semantics in
                XCTAssertEqual(semantics, .readRetryable)
                return await recorder.connect(request)
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

    private func actionKinds(in actions: [GatewayStreamAction]) -> [String] {
        actions.map { action in
            switch action {
            case .applyCommittedMessages:
                return "rows"
            case .applyRenderSnapshot:
                return "snapshot"
            case .resetCommittedCacheBelow:
                return "reset"
            case .refetchAfterControlRewrite:
                return "refetch"
            case .fallback:
                return "fallback"
            }
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
    func testWindowedReplayFrameResetsCursorOnResumeConnection() throws {
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 12, replayScope: .resume)

        let payload = """
        {"type":"thread_render_frame","thread_id":"thread::w","replay":"windowed","events":[        {"type":"committed_message","seq":4801,"message":{"role":"assistant","text":"win head"}},        {"type":"committed_message","seq":4802,"message":{"role":"assistant","text":"win tail"}}        ],"render_state":{"based_on_seq":4802,"rows":[],"window":{"floor_seq":4801,"has_more_above":true}}}
        """
        let result = processor.processPayload(payload, threadId: "thread::w")

        XCTAssertNil(result.reconnect, "windowed frame must not trigger a gap reconnect")
        guard case .resetCommittedCacheBelow(let floorSeq) = result.actions.first else {
            return XCTFail("windowed frame must lead with the cache reset, got \(result.actions)")
        }
        XCTAssertEqual(floorSeq, 4801)
        let applied = result.actions.compactMap { action -> [GaryxTranscriptMessage]? in
            if case .applyCommittedMessages(let messages) = action { return messages }
            return nil
        }.flatMap { $0 }
        XCTAssertEqual(applied.count, 2, "window records must be applied")
        XCTAssertEqual(processor.connectionLastSeq, 4802)

        // An UNMARKED non-contiguous frame on the same connection still trips
        // the contiguity guard.
        var strict = GatewayStreamFrameProcessor()
        strict.resetConnection(afterSeq: 12, replayScope: .resume)
        let unmarked = """
        {"type":"thread_render_frame","thread_id":"thread::w","events":[        {"type":"committed_message","seq":4801,"message":{"role":"assistant","text":"gap"}}        ],"render_state":{"based_on_seq":4801,"rows":[]}}
        """
        let gapResult = strict.processPayload(unmarked, threadId: "thread::w")
        if case .gap = gapResult.reconnect {
            // expected
        } else {
            XCTFail("unmarked non-contiguous frame must gap-reconnect, got \(String(describing: gapResult.reconnect))")
        }
    }

    // TASK-1716: the flush gate is driven only from the frame-final render
    // snapshot apply. These pins guarantee "snapshot is the frame's last
    // action" for every frame shape that renders, and "no snapshot" exactly
    // on the early-return frames whose re-render arrives via the reconnect
    // replay or the history refetch instead.
    func testFrameActionsAlwaysEndWithRenderSnapshot() {
        var processor = GatewayStreamFrameProcessor()
        let rowsFrame = processor.processPayload(
            framePayload(threadId: "thread-order", basedOnSeq: 2, events: [
                event(seq: 1, role: "user", text: "one"),
                event(seq: 2, role: "assistant", text: "two"),
            ]),
            threadId: "thread-order"
        )
        XCTAssertEqual(actionKinds(in: rowsFrame.actions), ["rows", "snapshot"])

        let renderOnly = processor.processPayload(
            framePayload(threadId: "thread-order", basedOnSeq: 2, events: []),
            threadId: "thread-order"
        )
        XCTAssertEqual(actionKinds(in: renderOnly.actions), ["snapshot"])

        var windowed = GatewayStreamFrameProcessor()
        windowed.resetConnection(afterSeq: 12, replayScope: .resume)
        let windowedPayload = """
        {"type":"thread_render_frame","thread_id":"thread-order","replay":"windowed","events":[{"type":"committed_message","seq":4801,"message":{"role":"assistant","text":"head"}}],"render_state":{"based_on_seq":4801,"rows":[],"window":{"floor_seq":4801,"has_more_above":true}}}
        """
        let windowedFrame = windowed.processPayload(windowedPayload, threadId: "thread-order")
        XCTAssertEqual(actionKinds(in: windowedFrame.actions), ["reset", "rows", "snapshot"])

        var gapped = GatewayStreamFrameProcessor()
        gapped.resetConnection(afterSeq: 2, replayScope: .resume)
        let gapFrame = gapped.processPayload(
            framePayload(threadId: "thread-order", basedOnSeq: 5, events: [
                event(seq: 3, role: "assistant", text: "contiguous"),
                event(seq: 5, role: "assistant", text: "gap"),
            ]),
            threadId: "thread-order"
        )
        XCTAssertEqual(actionKinds(in: gapFrame.actions), ["rows"])
        XCTAssertEqual(gapFrame.reconnect, .gap(resumeAfterSeq: 3))

        var rewrite = GatewayStreamFrameProcessor()
        rewrite.resetConnection(afterSeq: 10, replayScope: .resume)
        let rewriteFrame = rewrite.processPayload(
            framePayload(threadId: "thread-order", basedOnSeq: 12, events: [
                event(seq: 11, role: "assistant", text: "before rewrite"),
                controlEvent(seq: 12, kind: "range_rewrite"),
            ]),
            threadId: "thread-order"
        )
        XCTAssertEqual(actionKinds(in: rewriteFrame.actions), ["rows", "refetch"])
        XCTAssertEqual(rewriteFrame.reconnect, .controlRewrite)
    }

    // TASK-1716: mirror the app wiring rule over real processor output —
    // rows and floor resets mark a pending visible change, and only the
    // frame-final `.applyRenderSnapshot` settles the gate, comparing against
    // the applied snapshot exactly like the app does. Covers the #TASK-1719
    // counterexample (rows must not flush mid-frame), the #TASK-1721
    // counterexample (a caught-up no-op frame must not open the window), and
    // pending rows driving a settle whose snapshot is unchanged.
    func testFrameLevelGateDriveRendersWholeFrameOnQuietEdge() {
        var processor = GatewayStreamFrameProcessor()
        var gate = GaryxStreamFlushGate()
        var applied: GaryxRenderSnapshot?

        func drive(_ result: GatewayStreamPayloadResult) -> [GaryxStreamFlushGate.FrameAction] {
            var settles: [GaryxStreamFlushGate.FrameAction] = []
            for action in result.actions {
                switch action {
                case .applyCommittedMessages, .resetCommittedCacheBelow:
                    XCTAssertTrue(
                        settles.isEmpty,
                        "rows/floor changes apply before the frame's snapshot and must not settle the gate"
                    )
                    gate.recordVisibleChange()
                case .applyRenderSnapshot(let snapshot):
                    let changed = GaryxStreamFlushGate.snapshotChanged(snapshot, appliedBefore: applied)
                    applied = snapshot
                    settles.append(gate.settleFrame(snapshotChanged: changed))
                default:
                    break
                }
            }
            return settles
        }

        // Cold open: the first frame (rows already merged) flushes at once.
        let cold = processor.processPayload(
            framePayload(threadId: "thread-drive", basedOnSeq: 1, events: [
                event(seq: 1, role: "assistant", text: "first"),
            ]),
            threadId: "thread-drive"
        )
        XCTAssertEqual(drive(cold), [.flushNowAndArmWindow])
        XCTAssertEqual(gate.windowElapsed(), .closeWindow)

        // #TASK-1721 regression: a reconnect's caught-up snapshot-only frame
        // (identical snapshot) is inert — no flush, no window...
        let caughtUp = processor.processPayload(
            framePayload(threadId: "thread-drive", basedOnSeq: 1, events: []),
            threadId: "thread-drive"
        )
        XCTAssertEqual(drive(caughtUp), [.skip])
        XCTAssertEqual(gate.state, .idle)

        // ...so the next real frame is still a zero-delay leading edge.
        let reply = processor.processPayload(
            framePayload(threadId: "thread-drive", basedOnSeq: 2, events: [
                event(seq: 2, role: "assistant", text: "reply"),
            ]),
            threadId: "thread-drive"
        )
        XCTAssertEqual(drive(reply), [.flushNowAndArmWindow])
        XCTAssertEqual(gate.windowElapsed(), .closeWindow)

        // A seq-gap frame (contiguous prefix row, then a hole) carries no
        // snapshot: its rows stay pending, nothing settles.
        let gap = processor.processPayload(
            framePayload(threadId: "thread-drive", basedOnSeq: 9, events: [
                event(seq: 3, role: "assistant", text: "contiguous"),
                event(seq: 9, role: "assistant", text: "hole"),
            ]),
            threadId: "thread-drive"
        )
        XCTAssertEqual(gap.reconnect, .gap(resumeAfterSeq: 3))
        XCTAssertEqual(drive(gap), [])

        // The reconnect replay settles the pending rows (leading edge), and a
        // follow-up replay frame whose tail snapshot equals the applied one
        // still drives via its own rows (body upgrades must render).
        processor.resetConnection(afterSeq: 3, replayScope: .resume)
        let replayHead = processor.processPayload(
            framePayload(threadId: "thread-drive", basedOnSeq: 9, events: [
                event(seq: 4, role: "assistant", text: "replay head"),
            ]),
            threadId: "thread-drive"
        )
        XCTAssertEqual(drive(replayHead), [.flushNowAndArmWindow])

        let replayTail = processor.processPayload(
            framePayload(threadId: "thread-drive", basedOnSeq: 9, events: [
                event(seq: 5, role: "assistant", text: "replay tail"),
            ]),
            threadId: "thread-drive"
        )
        XCTAssertEqual(drive(replayTail), [.absorb])
        XCTAssertEqual(gate.state, .window(hasBacklog: true))
        XCTAssertEqual(gate.windowElapsed(), .flushBacklogAndRearmWindow)
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
            case .resetCommittedCacheBelow(let floorSeq):
                return "resetCommittedCacheBelow(\(floorSeq))"
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

private actor GatewayStreamFailureRecorder {
    private var requests: [URLRequest] = []
    private var actions: [GatewayStreamAction] = []

    func connect(_ request: URLRequest) throws -> GatewayStreamConnection {
        requests.append(request)
        throw URLError(.notConnectedToInternet)
    }

    func handle(_ action: GatewayStreamAction) -> GatewayStreamActionResult {
        actions.append(action)
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

    func actionNames() -> [String] {
        actions.map { action in
            switch action {
            case .applyCommittedMessages:
                return "applyCommittedMessages"
            case .resetCommittedCacheBelow(let floorSeq):
                return "resetCommittedCacheBelow(\(floorSeq))"
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
}
