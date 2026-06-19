import Foundation

// MARK: - Conversation state contract vocabulary
//
// iOS implementation of the cross-platform conversation state contract
// (docs/agents/conversation-state.md). The desktop reference implementation
// is desktop/garyx-desktop/src/renderer/src/message-machine.ts; both sides
// run the shared fixtures in spec/conversation-state. Raw values must match
// spec/conversation-state/states.json exactly.

public enum GaryxComposerPhase: String, CaseIterable, Sendable {
    case empty
    case editing
    case imeComposing = "ime_composing"
    case locked
}

public enum GaryxIntentDispatchMode: String, CaseIterable, Sendable {
    case syncSend = "sync_send"
    case asyncSteer = "async_steer"
}

public enum GaryxIntentState: String, CaseIterable, Sendable {
    case queuedLocal = "queued_local"
    case dispatchRequested = "dispatch_requested"
    case dispatching
    case remoteAccepted = "remote_accepted"
    case awaitingProviderAck = "awaiting_provider_ack"
    case awaitingResponse = "awaiting_response"
    case awaitingHistory = "awaiting_history"
    case completed
    case failed
    case interrupted
    case cancelled
}

public enum GaryxIntentSource: String, CaseIterable, Sendable {
    case composerSend = "composer_send"
    case composerQueue = "composer_queue"
    case queueSend = "queue_send"
    case queueSteer = "queue_steer"
    case retry
}

public enum GaryxThreadRuntimeState: String, CaseIterable, Sendable {
    case idle
    case dispatchingSync = "dispatching_sync"
    case runningRemote = "running_remote"
    case reconcilingHistory = "reconciling_history"
    case interrupting
    case failed
}

public enum GaryxLiveStreamStatus: String, CaseIterable, Sendable {
    case connecting
    case streaming
    case reconciling
    case disconnected
    case failed
    case interrupted
}

public enum GaryxTranscriptEntryState: String, CaseIterable, Sendable {
    case optimistic
    case remotePartial = "remote_partial"
    case remoteFinal = "remote_final"
    case error
    case interrupted
}

// MARK: - Machine records

public struct GaryxMessageIntent: Equatable, Sendable {
    public var intentId: String
    public var threadId: String
    public var text: String
    public var createdAt: String
    public var updatedAt: String
    public var state: GaryxIntentState
    public var source: GaryxIntentSource
    public var dispatchMode: GaryxIntentDispatchMode?
    public var remoteRunId: String?
    public var remoteThreadKey: String?
    public var pendingInputId: String?
    public var responseText: String?
    public var error: String?

    public init(
        intentId: String,
        threadId: String,
        text: String,
        createdAt: String = "",
        updatedAt: String = "",
        state: GaryxIntentState,
        source: GaryxIntentSource,
        dispatchMode: GaryxIntentDispatchMode? = nil,
        remoteRunId: String? = nil,
        remoteThreadKey: String? = nil,
        pendingInputId: String? = nil,
        responseText: String? = nil,
        error: String? = nil
    ) {
        self.intentId = intentId
        self.threadId = threadId
        self.text = text
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.state = state
        self.source = source
        self.dispatchMode = dispatchMode
        self.remoteRunId = remoteRunId
        self.remoteThreadKey = remoteThreadKey
        self.pendingInputId = pendingInputId
        self.responseText = responseText
        self.error = error
    }
}

public struct GaryxThreadRuntime: Equatable, Sendable {
    public var threadId: String
    public var state: GaryxThreadRuntimeState
    public var activeIntentId: String?
    public var remoteRunId: String?
    public var lastError: String?
    public var updatedAt: String

    public init(
        threadId: String,
        state: GaryxThreadRuntimeState,
        activeIntentId: String? = nil,
        remoteRunId: String? = nil,
        lastError: String? = nil,
        updatedAt: String = ""
    ) {
        self.threadId = threadId
        self.state = state
        self.activeIntentId = activeIntentId
        self.remoteRunId = remoteRunId
        self.lastError = lastError
        self.updatedAt = updatedAt
    }
}

public func garyxIsRuntimeBusy(_ state: GaryxThreadRuntimeState?) -> Bool {
    guard let state else { return false }
    return state != .idle && state != .failed
}

public func garyxNextComposerPhase(
    hasText: Bool,
    isComposing: Bool,
    locked: Bool
) -> GaryxComposerPhase {
    if locked { return .locked }
    if isComposing { return .imeComposing }
    return hasText ? .editing : .empty
}

// MARK: - Actions

public enum GaryxConversationAction: Sendable {
    case composerSync(hasText: Bool, isComposing: Bool, locked: Bool)
    case intentCreated(intent: GaryxMessageIntent, enqueue: Bool)
    case intentRequestDispatch(
        threadId: String,
        intentId: String,
        mode: GaryxIntentDispatchMode,
        source: GaryxIntentSource,
        removeFromQueue: Bool
    )
    case intentDispatchStarted(intentId: String)
    case intentRemoteAccepted(
        intentId: String,
        runId: String,
        threadId: String,
        pendingInputId: String?,
        responseText: String?,
        removeFromQueue: Bool,
        awaitProviderAck: Bool
    )
    case intentAwaitingResponse(intentId: String)
    case intentAwaitingHistory(intentId: String, responseText: String?)
    case intentCompleted(intentId: String)
    case intentFailed(intentId: String, error: String)
    case intentInterrupted(intentId: String, error: String?)
    case intentCancelled(threadId: String, intentId: String)
    case intentRequeueFront(
        threadId: String,
        intentId: String,
        error: String?,
        source: GaryxIntentSource?
    )
    case intentReorder(threadId: String, intentId: String, toIndex: Int)
    case threadRuntime(
        threadId: String,
        state: GaryxThreadRuntimeState,
        activeIntentId: String?,
        remoteRunId: String?,
        error: String?
    )
    case threadClear(threadId: String)
    case threadReplaceId(fromThreadId: String, toThreadId: String)
    case threadDelete(threadId: String)
}

// MARK: - Machine state

public struct GaryxConversationMachineState: Equatable, Sendable {
    public var composerPhase: GaryxComposerPhase
    public var intentsById: [String: GaryxMessageIntent]
    public var queueByThread: [String: [String]]
    public var threadRuntimeByThread: [String: GaryxThreadRuntime]

    public init(
        composerPhase: GaryxComposerPhase = .empty,
        intentsById: [String: GaryxMessageIntent] = [:],
        queueByThread: [String: [String]] = [:],
        threadRuntimeByThread: [String: GaryxThreadRuntime] = [:]
    ) {
        self.composerPhase = composerPhase
        self.intentsById = intentsById
        self.queueByThread = queueByThread
        self.threadRuntimeByThread = threadRuntimeByThread
    }

    public func threadRuntime(for threadId: String?) -> GaryxThreadRuntime? {
        guard let threadId else { return nil }
        return threadRuntimeByThread[threadId]
    }

    public func queueIntentIds(for threadId: String?) -> [String] {
        guard let threadId else { return [] }
        return queueByThread[threadId] ?? []
    }

    public var globalActiveThreadId: String? {
        threadRuntimeByThread.values
            .first { garyxIsRuntimeBusy($0.state) }?
            .threadId
    }

    // MARK: Reducer

    /// Applies one action, mirroring the desktop `messageMachineReducer`
    /// semantics exactly (including its no-op edge cases). `now` stamps
    /// `updatedAt` on mutated records; fixtures never assert timestamps.
    public mutating func apply(
        _ action: GaryxConversationAction,
        now: () -> String = { ISO8601DateFormatter().string(from: Date()) }
    ) {
        switch action {
        case let .composerSync(hasText, isComposing, locked):
            composerPhase = garyxNextComposerPhase(
                hasText: hasText,
                isComposing: isComposing,
                locked: locked
            )

        case let .intentCreated(intent, enqueue):
            intentsById[intent.intentId] = intent
            if enqueue {
                queueByThread[intent.threadId, default: []].append(intent.intentId)
            }

        case let .intentRequestDispatch(threadId, intentId, mode, source, removeFromQueue):
            // Unknown intents are a complete no-op: the queue stays untouched.
            guard intentsById[intentId] != nil else { return }
            if removeFromQueue {
                queueByThread[threadId] = (queueByThread[threadId] ?? []).filter { $0 != intentId }
            } else if queueByThread[threadId] == nil {
                queueByThread[threadId] = []
            }
            patchIntent(intentId, now: now) { intent in
                intent.state = .dispatchRequested
                intent.dispatchMode = mode
                intent.source = source
                intent.error = nil
            }

        case let .intentDispatchStarted(intentId):
            patchIntent(intentId, now: now) { intent in
                intent.state = .dispatching
                intent.error = nil
            }

        case let .intentAwaitingResponse(intentId):
            patchIntent(intentId, now: now) { intent in
                intent.state = .awaitingResponse
            }

        case let .intentRemoteAccepted(
            intentId, runId, threadId, pendingInputId, responseText, removeFromQueue, awaitProviderAck
        ):
            let existing = intentsById[intentId]
            var nextState: GaryxIntentState = .remoteAccepted
            if let existingState = existing?.state,
               existingState == .awaitingProviderAck
                || existingState == .awaitingHistory
                || existingState == .completed {
                nextState = existingState
            } else if awaitProviderAck {
                nextState = .awaitingProviderAck
            }
            patchIntent(intentId, now: now) { intent in
                intent.state = nextState
                intent.remoteRunId = runId
                intent.remoteThreadKey = threadId
                intent.pendingInputId = pendingInputId ?? existing?.pendingInputId
                intent.responseText = responseText ?? existing?.responseText
                intent.error = nil
            }
            guard removeFromQueue, let accepted = intentsById[intentId] else { return }
            queueByThread[accepted.threadId] =
                (queueByThread[accepted.threadId] ?? []).filter { $0 != intentId }

        case let .intentAwaitingHistory(intentId, responseText):
            patchIntent(intentId, now: now) { intent in
                intent.state = .awaitingHistory
                intent.responseText = responseText
            }

        case let .intentCompleted(intentId):
            patchIntent(intentId, now: now) { intent in
                intent.state = .completed
                intent.error = nil
            }

        case let .intentFailed(intentId, error):
            patchIntent(intentId, now: now) { intent in
                intent.state = .failed
                intent.error = error
            }

        case let .intentInterrupted(intentId, error):
            patchIntent(intentId, now: now) { intent in
                intent.state = .interrupted
                intent.error = error
            }

        case let .intentCancelled(threadId, intentId):
            patchIntent(intentId, now: now) { intent in
                intent.state = .cancelled
            }
            // The queue entry is removed even when the intent record is unknown.
            queueByThread[threadId] = (queueByThread[threadId] ?? []).filter { $0 != intentId }

        case let .intentRequeueFront(threadId, intentId, error, source):
            patchIntent(intentId, now: now) { intent in
                intent.state = .queuedLocal
                intent.dispatchMode = nil
                intent.remoteRunId = nil
                intent.remoteThreadKey = nil
                intent.pendingInputId = nil
                intent.responseText = nil
                intent.error = error
                intent.source = source ?? .queueSend
            }
            var queue = queueByThread[threadId] ?? []
            if !queue.contains(intentId) {
                queue.insert(intentId, at: 0)
            }
            queueByThread[threadId] = queue

        case let .intentReorder(threadId, intentId, toIndex):
            var queue = queueByThread[threadId] ?? []
            guard let fromIndex = queue.firstIndex(of: intentId) else { return }
            let boundedIndex = max(0, min(toIndex, queue.count - 1))
            guard boundedIndex != fromIndex else { return }
            queue.remove(at: fromIndex)
            queue.insert(intentId, at: boundedIndex)
            queueByThread[threadId] = queue

        case let .threadRuntime(threadId, state, activeIntentId, remoteRunId, error):
            // Omitted fields clear on every application, matching the desktop
            // upsert which always overwrites these three fields.
            threadRuntimeByThread[threadId] = GaryxThreadRuntime(
                threadId: threadId,
                state: state,
                activeIntentId: activeIntentId,
                remoteRunId: remoteRunId,
                lastError: error,
                updatedAt: now()
            )

        case let .threadClear(threadId):
            threadRuntimeByThread[threadId] = nil

        case let .threadReplaceId(fromThreadId, toThreadId):
            guard fromThreadId != toThreadId else { return }
            let timestamp = now()
            for (intentId, intent) in intentsById where intent.threadId == fromThreadId {
                var moved = intent
                moved.threadId = toThreadId
                moved.updatedAt = timestamp
                intentsById[intentId] = moved
            }
            let fromQueue = queueByThread[fromThreadId] ?? []
            let toQueue = queueByThread[toThreadId] ?? []
            queueByThread[fromThreadId] = nil
            if !fromQueue.isEmpty || !toQueue.isEmpty {
                queueByThread[toThreadId] = toQueue + fromQueue.filter { !toQueue.contains($0) }
            }
            if let draftRuntime = threadRuntimeByThread[fromThreadId] {
                threadRuntimeByThread[fromThreadId] = nil
                // An existing runtime on the target id wins field-by-field.
                var merged = threadRuntimeByThread[toThreadId] ?? draftRuntime
                merged.threadId = toThreadId
                merged.updatedAt = timestamp
                threadRuntimeByThread[toThreadId] = merged
            }

        case let .threadDelete(threadId):
            for (intentId, intent) in intentsById where intent.threadId == threadId {
                intentsById[intentId] = nil
            }
            queueByThread[threadId] = nil
            threadRuntimeByThread[threadId] = nil
        }
    }

    private mutating func patchIntent(
        _ intentId: String,
        now: () -> String,
        _ mutate: (inout GaryxMessageIntent) -> Void
    ) {
        guard var intent = intentsById[intentId] else { return }
        mutate(&intent)
        intent.updatedAt = now()
        intentsById[intentId] = intent
    }
}

// MARK: - Provider ack helpers

public func garyxFindPendingAckIntentIndex(
    pendingAckIntentIds: [String],
    acknowledgedPendingInputId: String,
    intentsById: [String: GaryxMessageIntent]
) -> Int {
    if pendingAckIntentIds.isEmpty { return -1 }
    if acknowledgedPendingInputId.isEmpty { return 0 }

    if let exactIndex = pendingAckIntentIds.firstIndex(where: { intentId in
        intentsById[intentId]?.pendingInputId == acknowledgedPendingInputId
    }) {
        return exactIndex
    }

    // Codex can emit user_ack before stream_input returns the pendingInputId.
    let unresolvedIndexes = pendingAckIntentIds.enumerated()
        .filter { _, intentId in
            (intentsById[intentId]?.pendingInputId ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .isEmpty
        }
        .map(\.offset)

    if unresolvedIndexes.count == 1 {
        return unresolvedIndexes[0]
    }
    if unresolvedIndexes.count == pendingAckIntentIds.count {
        return 0
    }
    return -1
}

public func garyxShouldTrackProviderAckAfterStreamInputResponse(
    intentState: GaryxIntentState?
) -> Bool {
    guard let intentState else { return false }
    switch intentState {
    case .awaitingHistory, .completed, .failed, .interrupted, .cancelled:
        return false
    case .queuedLocal, .dispatchRequested, .dispatching, .remoteAccepted,
         .awaitingProviderAck, .awaitingResponse:
        return true
    }
}

// MARK: - Derived activity model

public struct GaryxActivityMessage: Equatable, Sendable {
    public var role: GaryxTranscriptRole
    public var isLoopContinuation: Bool

    public init(
        role: GaryxTranscriptRole,
        isLoopContinuation: Bool = false
    ) {
        self.role = role
        self.isLoopContinuation = isLoopContinuation
    }
}

public struct GaryxThreadActivityModel: Equatable, Sendable {
    public var runActive: Bool
    public var canSteerQueuedPrompt: Bool
    public var showPendingAckLoading: Bool

    public static func latestUserMessageAwaitsAssistant(
        _ messages: [GaryxActivityMessage]
    ) -> Bool {
        var latestUserIndex = -1
        var latestAssistantOrToolIndex = -1
        for (index, message) in messages.enumerated() {
            if message.role == .user, !message.isLoopContinuation {
                latestUserIndex = index
            }
            if message.role == .assistant || message.role == .toolUse || message.role == .toolResult {
                latestAssistantOrToolIndex = index
            }
        }
        return latestUserIndex >= 0 && latestAssistantOrToolIndex < latestUserIndex
    }

    public static func derive(
        messages: [GaryxActivityMessage],
        runtimeBusy: Bool,
        pendingAckIntentCount: Int,
        remoteAwaitingAckInputCount: Int,
        pendingHistoryIntent: Bool
    ) -> GaryxThreadActivityModel {
        let latestUserAwaitsAssistant = latestUserMessageAwaitsAssistant(messages)
        let showPendingAckLoading = pendingAckIntentCount > 0
            || remoteAwaitingAckInputCount > 0
            || (pendingHistoryIntent && latestUserAwaitsAssistant)
        let runActive = runtimeBusy
        return GaryxThreadActivityModel(
            runActive: runActive,
            canSteerQueuedPrompt: showPendingAckLoading || runActive,
            showPendingAckLoading: showPendingAckLoading
        )
    }
}
