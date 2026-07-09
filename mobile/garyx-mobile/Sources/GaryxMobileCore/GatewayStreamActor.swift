import Foundation

public enum GatewayStreamFallbackReason: Equatable, Sendable {
    case notFound404
    case persistentFailure
}

public enum GatewayStreamAction: Equatable, Sendable {
    case applyCommittedMessages([GaryxTranscriptMessage])
    case applyRenderSnapshot(GaryxRenderSnapshot)
    /// Server degraded a stale resume to the initial window: cached
    /// committed rows below the window floor are no longer contiguous with
    /// this connection and must be dropped before the window applies.
    case resetCommittedCacheBelow(floorSeq: Int)
    case refetchAfterControlRewrite
    case fallback(GatewayStreamFallbackReason)
}

public enum GatewayStreamActionResult: Equatable, Sendable {
    case none
    case resumeCursor(Int)
}

public enum GatewayStreamReconnect: Equatable, Sendable {
    case gap(resumeAfterSeq: Int)
    case controlRewrite
}

public struct GatewayStreamPayloadResult: Equatable, Sendable {
    public var actions: [GatewayStreamAction]
    public var reconnect: GatewayStreamReconnect?

    public init(actions: [GatewayStreamAction] = [], reconnect: GatewayStreamReconnect? = nil) {
        self.actions = actions
        self.reconnect = reconnect
    }
}

public enum GatewayStreamHTTPDecision: Equatable, Sendable {
    case stream
    case fallback(GatewayStreamFallbackReason)
}

public enum GatewayStreamHTTPStatusPlanner {
    public static func decision(statusCode: Int) throws -> GatewayStreamHTTPDecision {
        if statusCode == 404 {
            return .fallback(.notFound404)
        }
        guard (200..<300).contains(statusCode) else {
            throw GaryxGatewayError.invalidHTTPResponse
        }
        return .stream
    }
}

public struct GatewayStreamEndpoint: Equatable, Sendable {
    public var configuration: GaryxGatewayConfiguration

    public init(configuration: GaryxGatewayConfiguration) {
        self.configuration = configuration
    }

    public func threadStreamRequest(threadId: String, afterSeq: Int) throws -> URLRequest {
        try GaryxGatewayClient(configuration: configuration).threadStreamRequest(
            threadId: threadId,
            afterSeq: afterSeq
        )
    }

    public func threadStreamRequest(threadId: String, request: GatewayThreadStreamRequestState) throws -> URLRequest {
        try GaryxGatewayClient(configuration: configuration).threadStreamRequest(
            threadId: threadId,
            afterSeq: request.afterSeq,
            replayScope: request.replayScope,
            initialUserTurns: request.initialUserTurns,
            renderFloor: request.renderFloor
        )
    }
}

public struct GatewayStreamConnection: Sendable {
    public var statusCode: Int
    public var lines: AsyncThrowingStream<String, Error>

    public init(statusCode: Int, lines: AsyncThrowingStream<String, Error>) {
        self.statusCode = statusCode
        self.lines = lines
    }
}

public struct GatewayStreamTransport: Sendable {
    public var connect: @Sendable (URLRequest) async throws -> GatewayStreamConnection

    public init(connect: @escaping @Sendable (URLRequest) async throws -> GatewayStreamConnection) {
        self.connect = connect
    }
}

public struct GatewayStreamFrameProcessor: Sendable {
    public private(set) var connectionLastSeq: Int = 0
    public private(set) var madeProgressOnConnection: Bool = false
    private var allowsNonContiguousFirstSeq: Bool = false
    /// Delta-reassembly base (#TASK-1956 batch 3): the last full snapshot
    /// this connection accepted, its `rowsHash` chain token included. The
    /// chain only needs to live within one connection — `resetConnection`
    /// clears it, and after a reconnect the first frame is always a full
    /// replay/snapshot frame, which reseeds unconditionally.
    private var heldSnapshot: GaryxRenderSnapshot?

    public init() {}

    public mutating func resetConnection() {
        resetConnection(afterSeq: 0, replayScope: .resume)
    }

    public mutating func resetConnection(afterSeq: Int, replayScope: GatewayThreadStreamReplayScope?) {
        connectionLastSeq = 0
        madeProgressOnConnection = false
        allowsNonContiguousFirstSeq = replayScope == .initial
        heldSnapshot = nil
        if replayScope != .initial {
            connectionLastSeq = max(afterSeq, 0)
        }
    }

    public mutating func processDataLine(_ line: String, threadId: String) -> GatewayStreamPayloadResult {
        guard line.hasPrefix("data:") else { return GatewayStreamPayloadResult() }
        var value = String(line.dropFirst(5))
        if value.hasPrefix(" ") {
            value.removeFirst()
        }
        return processPayload(value, threadId: threadId)
    }

    public mutating func processPayload(_ payload: String, threadId: String) -> GatewayStreamPayloadResult {
        switch GatewayStreamPayloadDecoder.decode(payload) {
        case .renderFrame(let frame):
            return processRenderFrame(frame, threadId: threadId)
        case .committedMessage, .ping, .ignored:
            return GatewayStreamPayloadResult()
        }
    }

    public mutating func processRenderFrame(
        _ frame: GaryxThreadRenderFrame,
        threadId: String
    ) -> GatewayStreamPayloadResult {
        let frameThreadId = frame.threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard frameThreadId.isEmpty || frameThreadId == threadId else {
            return GatewayStreamPayloadResult()
        }
        madeProgressOnConnection = true
        // Resolve the frame's full snapshot BEFORE any event applies: a
        // delta-chain violation discards the frame atomically (no actions,
        // held snapshot and token untouched) and rides the existing gap
        // exit — the reconnect's replay frame reseeds the chain.
        let renderSnapshot: GaryxRenderSnapshot
        switch resolveRenderSnapshot(frame) {
        case .snapshot(let resolved):
            renderSnapshot = resolved
        case .violation:
            return GatewayStreamPayloadResult(
                reconnect: .gap(resumeAfterSeq: connectionLastSeq)
            )
        case .ignore:
            return GatewayStreamPayloadResult()
        }
        var windowedFloorSeq: Int?
        if frame.replay == "windowed" {
            // Server-degraded stale resume: the frame is a self-identifying
            // window reset, so its first (floor) record is deliberately not
            // contiguous with our cursor. Reset the connection cursor and
            // accept the window; unmarked frames keep the contiguity guard.
            connectionLastSeq = 0
            allowsNonContiguousFirstSeq = true
            windowedFloorSeq = renderSnapshot.window?.floorSeq
                ?? frame.events.compactMap(\.seq).min()
        }

        var actions: [GatewayStreamAction] = []
        if let windowedFloorSeq {
            actions.append(.resetCommittedCacheBelow(floorSeq: windowedFloorSeq))
        }
        var committedMessages: [GaryxTranscriptMessage] = []
        for event in frame.events {
            guard event.type == "committed_message",
                  let seq = event.seq,
                  var message = event.message else {
                continue
            }
            switch GaryxStreamSeqPlanner.decide(
                incomingSeq: seq,
                connectionLastSeq: connectionLastSeq,
                allowsNonContiguousFirstSeq: allowsNonContiguousFirstSeq
            ) {
            case .gapReconnect(let resumeAfterSeq):
                if !committedMessages.isEmpty {
                    actions.append(.applyCommittedMessages(committedMessages))
                }
                return GatewayStreamPayloadResult(
                    actions: actions,
                    reconnect: .gap(resumeAfterSeq: resumeAfterSeq)
                )
            case .stale:
                continue
            case .apply:
                message.index = seq - 1
                message.id = "history:\(seq - 1)"
                if GaryxTranscriptControlRewritePlanner.action(for: message) == .refetchAuthoritativeTranscript {
                    connectionLastSeq = max(connectionLastSeq, seq)
                    madeProgressOnConnection = true
                    if !committedMessages.isEmpty {
                        actions.append(.applyCommittedMessages(committedMessages))
                    }
                    actions.append(.refetchAfterControlRewrite)
                    return GatewayStreamPayloadResult(actions: actions, reconnect: .controlRewrite)
                }
                committedMessages.append(message)
                connectionLastSeq = seq
                madeProgressOnConnection = true
            }
        }

        if !committedMessages.isEmpty {
            actions.append(.applyCommittedMessages(committedMessages))
        }
        actions.append(.applyRenderSnapshot(renderSnapshot))
        return GatewayStreamPayloadResult(actions: actions)
    }

    private enum RenderFrameResolution {
        /// The frame's full snapshot: the carried `render_state`, or the
        /// held snapshot with an accepted `render_delta` applied. Either
        /// way the emitted action stream only ever carries full snapshots.
        case snapshot(GaryxRenderSnapshot)
        /// Delta-chain protocol violation: discard the frame atomically and
        /// exit through the existing gap path.
        case violation
        /// A frame with neither `render_state` nor `render_delta`:
        /// out-of-contract, nothing to render. Ignoring is safe — if a
        /// delta chain was live, the next delta gaps on `from_seq`.
        case ignore
    }

    /// Delta reassembly + the unified seeding rule (#TASK-1956 batch 3).
    /// Mirrors `apply_render_delta` in garyx-models minus the reassembled
    /// rows-hash tripwire: the server is the only hasher; the client stores
    /// `rowsHash` as an opaque token and validates the chain by equality.
    private mutating func resolveRenderSnapshot(
        _ frame: GaryxThreadRenderFrame
    ) -> RenderFrameResolution {
        if let snapshot = frame.renderState {
            // Unconditional reseed: every frame carrying a full
            // `render_state` — replay, snapshot-only, first live, same-seq
            // reseed — replaces the held snapshot and chain token. There is
            // no other cache-invalidation event. A frame carrying both
            // (the gateway never produces one) resolves the same way: the
            // full snapshot is authoritative and reseeding is always safe,
            // matching the desktop reassembler.
            heldSnapshot = snapshot
            return .snapshot(snapshot)
        }
        guard let delta = frame.renderDelta else {
            return .ignore
        }
        guard let held = heldSnapshot, delta.fromSeq == held.basedOnSeq else {
            return .violation
        }
        // Pure equality on the opaque chain token — the client never
        // hashes. A held snapshot without a token (a full frame that
        // arrived without `rows_hash`) can never anchor a delta.
        guard let heldToken = held.rowsHash, delta.fromRowsHash == heldToken else {
            return .violation
        }
        let orderIds = Set(delta.rowOrder)
        var upsertById = [String: GaryxRenderRow](minimumCapacity: delta.upsertRows.count)
        for row in delta.upsertRows {
            guard let rowId = row.garyxDeltaRowId else {
                return .violation
            }
            guard upsertById.updateValue(row, forKey: rowId) == nil else {
                return .violation
            }
            // Every upsert must be referenced by row_order: a stray upsert
            // is a producer/consumer disagreement, not ignorable padding.
            guard orderIds.contains(rowId) else {
                return .violation
            }
        }
        var heldById = [String: GaryxRenderRow](minimumCapacity: held.rows.count)
        for row in held.rows {
            if let rowId = row.garyxDeltaRowId {
                heldById[rowId] = row
            }
        }
        // Rebuild rows in row_order: upsert body wins, else the held row by
        // id; an id in neither is a protocol violation.
        var rows = [GaryxRenderRow]()
        rows.reserveCapacity(delta.rowOrder.count)
        for rowId in delta.rowOrder {
            guard let row = upsertById[rowId] ?? heldById[rowId] else {
                return .violation
            }
            rows.append(row)
        }
        // Scalar fields are replaced wholesale. `visibleMessageIds` is not
        // carried by deltas (zero consumers, deleted end-to-end in batch
        // 4), so the reassembled snapshot leaves it empty, matching the
        // Rust oracle.
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: delta.basedOnSeq,
            rows: rows,
            tailActivity: delta.tailActivity,
            activeToolGroupId: delta.activeToolGroupId,
            progressLocus: delta.progressLocus,
            visibleMessageIds: [],
            filteredPlaceholders: delta.filteredPlaceholders,
            rateLimit: delta.rateLimit,
            window: delta.window,
            rowsHash: delta.rowsHash
        )
        heldSnapshot = snapshot
        return .snapshot(snapshot)
    }
}

private extension GaryxRenderRow {
    /// Row identity for delta reassembly. Unknown row kinds keep their id
    /// so a newer server's rows survive `row_order` carry-forward.
    var garyxDeltaRowId: String? {
        switch self {
        case let .userTurn(row):
            return row.id
        case let .unknown(id):
            return id
        }
    }
}

public actor GatewayStreamActor {
    private let endpoint: GatewayStreamEndpoint
    private let transport: GatewayStreamTransport?
    private let reconnectDelayNanos: @Sendable (Int) -> UInt64
    private var processor = GatewayStreamFrameProcessor()

    public init(
        endpoint: GatewayStreamEndpoint,
        transport: GatewayStreamTransport? = nil,
        reconnectDelayNanos: @escaping @Sendable (Int) -> UInt64 = { consecutiveFailures in
            GatewayStreamActor.defaultReconnectDelayNanos(consecutiveFailures: consecutiveFailures)
        }
    ) {
        self.endpoint = endpoint
        self.transport = transport
        self.reconnectDelayNanos = reconnectDelayNanos
    }

    public func run(
        threadId: String,
        cursorProvider: @escaping @Sendable () async -> Int,
        shouldContinue: @escaping @Sendable () async -> Bool,
        actionHandler: @escaping @Sendable (GatewayStreamAction) async -> GatewayStreamActionResult
    ) async {
        await run(
            threadId: threadId,
            requestProvider: {
                GatewayThreadStreamRequestState(afterSeq: await cursorProvider())
            },
            shouldContinue: shouldContinue,
            actionHandler: actionHandler
        )
    }

    public func run(
        threadId: String,
        requestProvider: @escaping @Sendable () async -> GatewayThreadStreamRequestState,
        shouldContinue: @escaping @Sendable () async -> Bool,
        actionHandler: @escaping @Sendable (GatewayStreamAction) async -> GatewayStreamActionResult
    ) async {
        var consecutiveFailures = 0
        var nextResumeOverride: Int?

        while !Task.isCancelled, await shouldContinue() {
            processor.resetConnection()
            do {
                let streamRequest: GatewayThreadStreamRequestState
                if let resumeOverride = nextResumeOverride {
                    streamRequest = (await requestProvider()).resuming(afterSeq: resumeOverride)
                } else {
                    streamRequest = await requestProvider()
                }
                nextResumeOverride = nil
                processor.resetConnection(afterSeq: streamRequest.afterSeq, replayScope: streamRequest.replayScope)
                let request = try endpoint.threadStreamRequest(threadId: threadId, request: streamRequest)
                let reconnect: GatewayStreamReconnect?
                if let transport {
                    let connection = try await transport.connect(request)
                    guard try await handleStatus(connection.statusCode, actionHandler: actionHandler) else { return }
                    reconnect = try await processLines(
                        connection.lines,
                        threadId: threadId,
                        shouldContinue: shouldContinue,
                        actionHandler: actionHandler
                    )
                } else {
                    let (bytes, response) = try await URLSession.shared.bytes(for: request)
                    guard let http = response as? HTTPURLResponse else {
                        throw GaryxGatewayError.invalidHTTPResponse
                    }
                    guard try await handleStatus(http.statusCode, actionHandler: actionHandler) else { return }
                    reconnect = try await processLines(
                        bytes.lines,
                        threadId: threadId,
                        shouldContinue: shouldContinue,
                        actionHandler: actionHandler
                    )
                }

                switch reconnect {
                case .gap(let resumeAfterSeq):
                    nextResumeOverride = resumeAfterSeq
                case .controlRewrite:
                    break
                case .none:
                    break
                }
            } catch {
                consecutiveFailures += 1
            }

            guard !Task.isCancelled, await shouldContinue() else { break }
            if processor.madeProgressOnConnection {
                consecutiveFailures = 0
            }
            if consecutiveFailures >= 4 {
                _ = await actionHandler(.fallback(.persistentFailure))
                return
            }
            let delay = reconnectDelayNanos(consecutiveFailures)
            if delay > 0 {
                try? await Task.sleep(nanoseconds: delay)
            }
        }
    }

    public nonisolated static func defaultReconnectDelayNanos(consecutiveFailures: Int) -> UInt64 {
        let delay = UInt64(min(consecutiveFailures, 5)) * 1_000_000_000
        return max(delay, 500_000_000)
    }

    private func handleStatus(
        _ statusCode: Int,
        actionHandler: @escaping @Sendable (GatewayStreamAction) async -> GatewayStreamActionResult
    ) async throws -> Bool {
        switch try GatewayStreamHTTPStatusPlanner.decision(statusCode: statusCode) {
        case .stream:
            return true
        case .fallback(let reason):
            _ = await actionHandler(.fallback(reason))
            return false
        }
    }

    private func processLines<Lines: AsyncSequence>(
        _ lines: Lines,
        threadId: String,
        shouldContinue: @escaping @Sendable () async -> Bool,
        actionHandler: @escaping @Sendable (GatewayStreamAction) async -> GatewayStreamActionResult
    ) async throws -> GatewayStreamReconnect? where Lines.Element == String {
        for try await line in lines {
            if Task.isCancelled {
                return nil
            }
            let canContinue = await shouldContinue()
            if !canContinue {
                return nil
            }
            let result = processor.processDataLine(line, threadId: threadId)
            let reconnect = await deliver(result, actionHandler: actionHandler)
            if let reconnect {
                return reconnect
            }
        }
        return nil
    }

    private func deliver(
        _ result: GatewayStreamPayloadResult,
        actionHandler: @escaping @Sendable (GatewayStreamAction) async -> GatewayStreamActionResult
    ) async -> GatewayStreamReconnect? {
        var controlRewriteResumeCursor: Int?
        for action in result.actions {
            let actionResult = await actionHandler(action)
            if action == .refetchAfterControlRewrite,
               case .resumeCursor(let cursor) = actionResult {
                controlRewriteResumeCursor = cursor
            }
        }
        switch result.reconnect {
        case .controlRewrite:
            // The main actor action has already completed cache reset + history reload.
            // Continue from the cursor it returned; if it declines, reconnect from 0
            // instead of reusing a stale pre-refetch cursor.
            return .gap(resumeAfterSeq: controlRewriteResumeCursor ?? 0)
        case .gap, .none:
            return result.reconnect
        }
    }
}

private enum GatewayStreamPayload: Sendable {
    case renderFrame(GaryxThreadRenderFrame)
    case committedMessage
    case ping
    case ignored
}

private enum GatewayStreamPayloadDecoder {
    private struct Envelope: Decodable {
        var type: String?
    }

    static func decode(_ payload: String) -> GatewayStreamPayload {
        let trimmed = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let data = trimmed.data(using: .utf8),
              let envelope = try? JSONDecoder().decode(Envelope.self, from: data),
              let type = envelope.type else {
            return .ignored
        }
        switch type {
        case "thread_render_frame":
            guard let frame = try? JSONDecoder().decode(GaryxThreadRenderFrame.self, from: data) else {
                return .ignored
            }
            return .renderFrame(frame)
        case "committed_message":
            return .committedMessage
        case "ping":
            return .ping
        default:
            return .ignored
        }
    }
}
