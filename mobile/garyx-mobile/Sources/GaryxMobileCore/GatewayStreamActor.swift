import Foundation

public enum GatewayStreamFallbackReason: Equatable, Sendable {
    case notFound404
    case persistentFailure
}

public enum GatewayStreamAction: Equatable, Sendable {
    case applyCommittedMessages([GaryxTranscriptMessage])
    case applyRenderSnapshot(GaryxRenderSnapshot)
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

    public init() {}

    public mutating func resetConnection() {
        resetConnection(afterSeq: 0, replayScope: .resume)
    }

    public mutating func resetConnection(afterSeq: Int, replayScope: GatewayThreadStreamReplayScope?) {
        connectionLastSeq = 0
        madeProgressOnConnection = false
        allowsNonContiguousFirstSeq = replayScope == .initial
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
        if frame.replay == "windowed" {
            // Server-degraded stale resume: the frame is a self-identifying
            // window reset, so its first (floor) record is deliberately not
            // contiguous with our cursor. Reset the connection cursor and
            // accept the window; unmarked frames keep the contiguity guard.
            connectionLastSeq = 0
            allowsNonContiguousFirstSeq = true
        }

        var actions: [GatewayStreamAction] = []
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
        actions.append(.applyRenderSnapshot(frame.renderState))
        return GatewayStreamPayloadResult(actions: actions)
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
