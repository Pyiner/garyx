import Foundation

// MARK: - Gateway status classifiers

public enum GaryxGatewayStreamStatusClassifier {
    public static func isSuccessfulStreamInput(_ status: String) -> Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "queued"
            || normalized == "accepted"
            || normalized == "ok"
            || normalized == "success"
    }

    public static func shouldFallbackStreamInput(_ status: String) -> Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "no_active_session"
            || normalized == "no active session"
            || normalized == "inactive"
            || normalized == "closed"
            || normalized == "not_found"
    }

    public static func isTransientGatewayErrorMessage(_ message: String) -> Bool {
        let normalized = message.lowercased()
        return normalized.contains("timed out")
            || normalized.contains("timeout")
            || normalized.contains("network connection was lost")
            || normalized.contains("not connected to the internet")
            || normalized.contains("connection reset")
            || normalized.contains("connection closed")
            || normalized.contains("websocket")
            || normalized.contains("socket")
            || normalized.contains("gateway unavailable")
            || normalized.contains("bad gateway")
            || normalized.contains("http 502")
            || normalized.contains("http 503")
            || normalized.contains("http 504")
            || normalized.contains("service unavailable")
    }
}

// MARK: - Run tracker

/// Conversation run tracking for the iOS app: a thin operations layer over
/// `GaryxConversationMachineState` that owns local send, queued-input, and
/// interrupt intent transitions. Server run-state is deliberately not derived
/// here: it is rebuilt from committed transcript control records by
/// `GaryxTranscriptRunStateReducer`.
///
/// Mapping onto the contract:
/// - a chat-start in flight is the thread runtime in `dispatching_sync`
/// - an accepted local dispatch is the thread runtime in `running_remote` with
///   an `activeIntentId`
/// - queued steer inputs are intents in `awaiting_provider_ack`
public struct GaryxConversationRunTracker: Equatable, Sendable {
    public private(set) var machine = GaryxConversationMachineState()
    public private(set) var pendingAckIntentIdsByThread: [String: [String]] = [:]

    public init() {}

    // MARK: Queries

    public func isThreadBusy(_ threadId: String) -> Bool {
        garyxIsRuntimeBusy(machine.threadRuntimeByThread[threadId]?.state)
    }

    /// Busy check that ignores a run owned by `intentId` — used by the send
    /// path to detect "busy with something else" without tripping over the
    /// dispatch it is itself starting.
    public func isThreadBusy(_ threadId: String, excludingIntentId intentId: String) -> Bool {
        guard let runtime = machine.threadRuntimeByThread[threadId],
              garyxIsRuntimeBusy(runtime.state) else {
            return false
        }
        return runtime.activeIntentId != intentId
    }

    public var busyThreadIds: Set<String> {
        Set(
            machine.threadRuntimeByThread.values
                .filter { garyxIsRuntimeBusy($0.state) }
                .map(\.threadId)
        )
    }

    /// A run started by a local dispatch (the runtime still carries the
    /// dispatching intent). Equivalent of the legacy `isSending` +
    /// `activeRunThreadId` pair.
    public var localActiveRunThreadId: String? {
        machine.threadRuntimeByThread.values
            .filter { garyxIsRuntimeBusy($0.state) && $0.activeIntentId != nil }
            .max { $0.updatedAt < $1.updatedAt }?
            .threadId
    }

    public var hasLocalActiveRun: Bool {
        localActiveRunThreadId != nil
    }

    public func isChatStartInFlight(_ threadId: String) -> Bool {
        machine.threadRuntimeByThread[threadId]?.state == .dispatchingSync
    }

    public var locallyTrackedThreadIds: Set<String> {
        Set(machine.intentsById.values.compactMap { intent in
            switch intent.state {
            case .completed, .failed, .interrupted, .cancelled:
                return nil
            case .queuedLocal, .dispatchRequested, .dispatching, .remoteAccepted,
                 .awaitingProviderAck, .awaitingResponse, .awaitingHistory:
                return intent.threadId
            }
        })
    }

    // MARK: Send lifecycle

    /// Claims the thread for a local chat dispatch. Returns false (and leaves
    /// the tracker untouched) when the thread is already busy with another
    /// run, unless the caller is intentionally sending a same-thread follow-up
    /// through the normal chat-start path.
    @discardableResult
    public mutating func beginLocalDispatch(
        threadId: String,
        intentId: String,
        text: String,
        allowWhileBusy: Bool = false
    ) -> Bool {
        let currentRuntime = machine.threadRuntimeByThread[threadId]
        if isThreadBusy(threadId, excludingIntentId: intentId), !allowWhileBusy {
            return false
        }
        if machine.intentsById[intentId] == nil {
            machine.apply(.intentCreated(
                intent: GaryxMessageIntent(
                    intentId: intentId,
                    threadId: threadId,
                    text: text,
                    state: .queuedLocal,
                    source: .composerSend
                ),
                enqueue: false
            ))
        }
        machine.apply(.intentRequestDispatch(
            threadId: threadId,
            intentId: intentId,
            mode: .syncSend,
            source: .composerSend,
            removeFromQueue: false
        ))
        machine.apply(.intentDispatchStarted(intentId: intentId))
        machine.apply(.threadRuntime(
            threadId: threadId,
            state: .dispatchingSync,
            activeIntentId: intentId,
            remoteRunId: allowWhileBusy ? currentRuntime?.remoteRunId : nil,
            error: nil
        ))
        return true
    }

    /// The gateway accepted a chat start. Moves the runtime to
    /// `running_remote`; when the gateway answered with a different thread id
    /// (draft threads), the tracked state migrates to the accepted id.
    public mutating func confirmChatStartAccepted(
        requestedThreadId: String,
        acceptedThreadId: String,
        intentId: String,
        runId: String
    ) {
        if requestedThreadId != acceptedThreadId {
            machine.apply(.threadReplaceId(
                fromThreadId: requestedThreadId,
                toThreadId: acceptedThreadId
            ))
        }
        machine.apply(.intentRemoteAccepted(
            intentId: intentId,
            runId: runId,
            threadId: acceptedThreadId,
            pendingInputId: nil,
            responseText: nil,
            removeFromQueue: false,
            awaitProviderAck: false
        ))
        machine.apply(.threadRuntime(
            threadId: acceptedThreadId,
            state: .runningRemote,
            activeIntentId: intentId,
            remoteRunId: runId.isEmpty ? nil : runId,
            error: nil
        ))
    }

    /// A local dispatch failed before (or while) reaching the gateway. The
    /// dispatch claim is released; remotely observed activity that arrived in
    /// the meantime keeps the thread busy.
    public mutating func failLocalDispatch(threadId: String, intentId: String, error: String) {
        machine.apply(.intentFailed(intentId: intentId, error: error))
        guard let runtime = machine.threadRuntimeByThread[threadId],
              garyxIsRuntimeBusy(runtime.state),
              runtime.activeIntentId == intentId else {
            return
        }
        let dropsRun = runtime.state == .dispatchingSync
        let preservedRemoteRunId = runtime.remoteRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let restoresRemoteRun = dropsRun && !preservedRemoteRunId.isEmpty
        machine.apply(.threadRuntime(
            threadId: threadId,
            state: dropsRun ? (restoresRemoteRun ? .runningRemote : .idle) : runtime.state,
            activeIntentId: nil,
            remoteRunId: dropsRun ? (restoresRemoteRun ? preservedRemoteRunId : nil) : runtime.remoteRunId,
            error: error
        ))
    }

    /// Releases a locally tracked run without failing its intent (legacy
    /// `clearActiveRunState(for:)`): the dispatch claim goes away, while a
    /// remotely observed run stays busy.
    public mutating func clearLocalRun(threadId: String) {
        guard let runtime = machine.threadRuntimeByThread[threadId],
              runtime.activeIntentId != nil else {
            return
        }
        let preservedRemoteRunId = runtime.remoteRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let restoresRemoteRun = runtime.state == .dispatchingSync && !preservedRemoteRunId.isEmpty
        machine.apply(.threadRuntime(
            threadId: threadId,
            state: runtime.state == .dispatchingSync
                ? (restoresRemoteRun ? .runningRemote : .idle)
                : runtime.state,
            activeIntentId: nil,
            remoteRunId: restoresRemoteRun ? preservedRemoteRunId : runtime.remoteRunId,
            error: nil
        ))
    }

    // MARK: Queued steer lifecycle

    /// Starts tracking a follow-up input sent while the thread is busy.
    public mutating func beginQueuedSteer(threadId: String, intentId: String, text: String) {
        if machine.intentsById[intentId] == nil {
            machine.apply(.intentCreated(
                intent: GaryxMessageIntent(
                    intentId: intentId,
                    threadId: threadId,
                    text: text,
                    state: .queuedLocal,
                    source: .queueSteer
                ),
                enqueue: false
            ))
        }
        machine.apply(.intentRequestDispatch(
            threadId: threadId,
            intentId: intentId,
            mode: .asyncSteer,
            source: .queueSteer,
            removeFromQueue: false
        ))
        machine.apply(.intentDispatchStarted(intentId: intentId))
    }

    /// The gateway queued the input downstream; it now awaits the provider
    /// `user_ack`.
    public mutating func confirmQueuedSteerAccepted(
        threadId: String,
        intentId: String,
        pendingInputId: String?
    ) {
        let currentRunId = machine.threadRuntimeByThread[threadId]?.remoteRunId ?? ""
        machine.apply(.intentRemoteAccepted(
            intentId: intentId,
            runId: currentRunId,
            threadId: threadId,
            pendingInputId: pendingInputId,
            responseText: nil,
            removeFromQueue: false,
            awaitProviderAck: true
        ))
        var pendingAck = pendingAckIntentIdsByThread[threadId] ?? []
        if !pendingAck.contains(intentId) {
            pendingAck.append(intentId)
        }
        pendingAckIntentIdsByThread[threadId] = pendingAck
        if !isThreadBusy(threadId) {
            markRemoteActivity(threadId: threadId, runId: "")
        }
    }

    /// The queued input was rejected (or its request failed). The thread's
    /// busy state is untouched — only the input itself failed.
    public mutating func failQueuedSteer(threadId: String, intentId: String, error: String) {
        machine.apply(.intentFailed(intentId: intentId, error: error))
        removePendingAckIntent(threadId: threadId, intentId: intentId)
    }

    /// Releases a queued steer for the fallback path that re-dispatches it as
    /// a fresh chat start.
    public mutating func releaseQueuedSteer(threadId: String, intentId: String) {
        removePendingAckIntent(threadId: threadId, intentId: intentId)
        guard let runtime = machine.threadRuntimeByThread[threadId],
              runtime.state == .runningRemote,
              runtime.activeIntentId == nil else {
            return
        }
        machine.apply(.threadRuntime(
            threadId: threadId,
            state: .idle,
            activeIntentId: nil,
            remoteRunId: nil,
            error: nil
        ))
    }

    // MARK: Committed control outcomes

    public mutating func acknowledgeProviderInput(threadId: String, pendingInputId: String?) {
        acknowledgePendingInput(
            threadId: threadId,
            pendingInputId: pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        )
    }

    public mutating func completeCommittedRun(threadId: String) {
        closeThreadRun(threadId: threadId, intentOutcome: .completed)
    }

    public mutating func failCommittedRun(threadId: String, error: String) {
        closeThreadRun(threadId: threadId, intentOutcome: .failed(error))
    }

    /// The user-initiated interrupt was accepted by the gateway or a committed
    /// control record reported an interrupted run.
    public mutating func interruptConfirmed(threadId: String) {
        closeThreadRun(threadId: threadId, intentOutcome: .interrupted)
    }

    // MARK: Internals

    private enum IntentOutcome {
        case completed
        case interrupted
        case failed(String)
    }

    /// True while a locally started chat dispatch has not produced its HTTP
    /// result yet.
    private func isChatStartClaim(_ runtime: GaryxThreadRuntime) -> Bool {
        if runtime.state == .dispatchingSync {
            return true
        }
        guard let activeIntentId = runtime.activeIntentId,
              let intent = machine.intentsById[activeIntentId] else {
            return false
        }
        return intent.state == .dispatchRequested || intent.state == .dispatching
    }

    private mutating func markRemoteActivity(threadId: String, runId: String) {
        let current = machine.threadRuntimeByThread[threadId]
        let normalizedRunId = runId.trimmingCharacters(in: .whitespacesAndNewlines)
        machine.apply(.threadRuntime(
            threadId: threadId,
            state: .runningRemote,
            activeIntentId: current?.activeIntentId,
            remoteRunId: normalizedRunId.isEmpty ? current?.remoteRunId : normalizedRunId,
            error: nil
        ))
    }

    private mutating func closeThreadRun(threadId: String, intentOutcome: IntentOutcome) {
        for intent in machine.intentsById.values where intent.threadId == threadId {
            switch intent.state {
            case .completed, .failed, .interrupted, .cancelled:
                continue
            case .queuedLocal, .dispatchRequested, .dispatching, .remoteAccepted,
                 .awaitingProviderAck, .awaitingResponse, .awaitingHistory:
                switch intentOutcome {
                case .completed:
                    machine.apply(.intentCompleted(intentId: intent.intentId))
                case .interrupted:
                    machine.apply(.intentInterrupted(intentId: intent.intentId, error: nil))
                case .failed(let message):
                    machine.apply(.intentFailed(intentId: intent.intentId, error: message))
                }
            }
        }
        pendingAckIntentIdsByThread[threadId] = nil
        guard machine.threadRuntimeByThread[threadId] != nil else { return }
        machine.apply(.threadRuntime(
            threadId: threadId,
            state: .idle,
            activeIntentId: nil,
            remoteRunId: nil,
            error: nil
        ))
    }

    private mutating func acknowledgePendingInput(threadId: String, pendingInputId: String) {
        let pendingAck = pendingAckIntentIdsByThread[threadId] ?? []
        let index = garyxFindPendingAckIntentIndex(
            pendingAckIntentIds: pendingAck,
            acknowledgedPendingInputId: pendingInputId,
            intentsById: machine.intentsById
        )
        guard index >= 0 else { return }
        var nextPendingAck = pendingAck
        // The acked intent leaves the pending-ack set; the runtime keeps its
        // existing dispatch claim — `activeIntentId` marks chat-start
        // ownership, not streaming attribution (the app tracks streaming
        // targets separately).
        nextPendingAck.remove(at: index)
        pendingAckIntentIdsByThread[threadId] = nextPendingAck.isEmpty ? nil : nextPendingAck
    }

    private mutating func removePendingAckIntent(threadId: String, intentId: String) {
        guard var pendingAck = pendingAckIntentIdsByThread[threadId] else { return }
        pendingAck.removeAll { $0 == intentId }
        pendingAckIntentIdsByThread[threadId] = pendingAck.isEmpty ? nil : pendingAck
    }

}
