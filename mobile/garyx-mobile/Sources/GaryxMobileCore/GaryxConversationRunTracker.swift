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

/// Outcome of reconciling an authoritative transcript snapshot against the
/// tracked thread runtime.
public enum GaryxTranscriptRuntimeReconciliation: Equatable, Sendable {
    /// The transcript reports an active run; the thread stays busy.
    case active
    /// The transcript reports no active run. `clearedLocalRun` is true when a
    /// locally tracked run was released (its streaming UI state should be
    /// cleaned up); false when there was nothing local to release or a chat
    /// start is still in flight (where "no active run yet" is expected).
    case inactive(clearedLocalRun: Bool)
}

/// Conversation run tracking for the iOS app: a thin operations layer over
/// `GaryxConversationMachineState` that owns every run/send lifecycle
/// transition the app model performs. It replaces the legacy scattered flags
/// (`isSending`, `activeRunThreadId`, `remoteBusyThreadIds`,
/// `pendingChatStartThreadIds`, `terminatedActiveRunIdsByThread`) with the
/// cross-platform conversation state contract
/// (docs/agents/conversation-state.md).
///
/// Mapping onto the contract:
/// - a chat-start in flight is the thread runtime in `dispatching_sync`
/// - remote busy is the thread runtime in `running_remote`
/// - queued steer inputs are intents in `awaiting_provider_ack`
/// - terminal stream events the client observed win over racing stale
///   `active_run` snapshots via `lastTerminatedRunIdsByThread`
public struct GaryxConversationRunTracker: Equatable, Sendable {
    public private(set) var machine = GaryxConversationMachineState()
    public private(set) var lastTerminatedRunIdsByThread: [String: String] = [:]
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

    /// Whether a thread-list summary's `activeRunId` should count as an
    /// active run: terminal events the client already observed win over a
    /// stale summary projection.
    public func isSummaryRunConsideredActive(threadId: String, activeRunId: String?) -> Bool {
        let normalized = activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !normalized.isEmpty else { return false }
        return lastTerminatedRunIdsByThread[threadId] != normalized
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

    // MARK: Stream events

    /// Applies a gateway stream event. This is the single busy-state
    /// derivation for live and replayed events (the legacy replay path
    /// dropped busy state on transient errors; this one classifies errors
    /// uniformly).
    public mutating func apply(streamEvent event: GaryxChatStreamEvent) {
        let threadId = Self.threadId(from: event)
        guard !threadId.isEmpty else { return }

        switch event {
        case .accepted, .runStart, .userMessage, .assistantDelta, .assistantBoundary, .toolUse, .toolResult:
            markRemoteActivity(threadId: threadId, runId: Self.runId(from: event))

        case .userAck(_, _, let pendingInputId):
            markRemoteActivity(threadId: threadId, runId: Self.runId(from: event))
            acknowledgePendingInput(
                threadId: threadId,
                pendingInputId: pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            )

        case .streamInput(let status, _, _, _):
            if GaryxGatewayStreamStatusClassifier.isSuccessfulStreamInput(status) {
                markRemoteActivity(threadId: threadId, runId: "")
            } else if let runtime = machine.threadRuntimeByThread[threadId],
                      runtime.state == .runningRemote,
                      runtime.activeIntentId == nil {
                // A remotely observed run whose queued input was rejected: the
                // gateway is the only busy signal and it just said no.
                machine.apply(.threadRuntime(
                    threadId: threadId,
                    state: .idle,
                    activeIntentId: nil,
                    remoteRunId: nil,
                    error: nil
                ))
            }

        case .done, .runComplete:
            recordTerminatedRun(threadId: threadId, runId: Self.runId(from: event))
            closeThreadRun(threadId: threadId, intentOutcome: .completed)

        case .runError(_, _, let message):
            recordTerminatedRun(threadId: threadId, runId: Self.runId(from: event))
            closeThreadRun(threadId: threadId, intentOutcome: .failed(message))

        case .interrupt(_, _, let abortedRuns):
            if let aborted = abortedRuns
                .map({ $0.trimmingCharacters(in: .whitespacesAndNewlines) })
                .last(where: { !$0.isEmpty }) {
                lastTerminatedRunIdsByThread[threadId] = aborted
            }
            closeThreadRun(threadId: threadId, intentOutcome: .interrupted)

        case .error(_, _, let message):
            if GaryxGatewayStreamStatusClassifier.isTransientGatewayErrorMessage(message) {
                markRemoteActivity(threadId: threadId, runId: Self.runId(from: event))
            } else {
                recordTerminatedRun(threadId: threadId, runId: Self.runId(from: event))
                closeThreadRun(threadId: threadId, intentOutcome: .failed(message))
            }

        case .ping, .snapshot, .threadTitleUpdated, .unknown:
            break
        }
    }

    /// The user-initiated interrupt was accepted by the gateway.
    public mutating func interruptConfirmed(threadId: String) {
        closeThreadRun(threadId: threadId, intentOutcome: .interrupted)
    }

    // MARK: Reconciliation

    /// Reconciles an authoritative transcript snapshot. Terminal events the
    /// client already observed win over a racing stale `active_run`; a chat
    /// start still in flight keeps its claim even though the transcript does
    /// not know the run yet.
    public mutating func reconcileTranscriptRuntime(
        threadId: String,
        activeRunPresent: Bool,
        activeRunId: String?,
        hasActivePendingInput: Bool
    ) -> GaryxTranscriptRuntimeReconciliation {
        let treatActive = GaryxMobileThreadActivityModel.shouldTreatThreadRuntimeAsActive(
            activeRunPresent: activeRunPresent,
            activeRunId: activeRunId,
            hasActivePendingInput: hasActivePendingInput,
            lastTerminatedRunId: lastTerminatedRunIdsByThread[threadId]
        )
        if treatActive {
            markRemoteActivity(threadId: threadId, runId: activeRunId ?? "")
            return .active
        }
        guard let runtime = machine.threadRuntimeByThread[threadId],
              garyxIsRuntimeBusy(runtime.state) else {
            return .inactive(clearedLocalRun: false)
        }
        if isChatStartClaim(runtime) {
            // The chat start has not produced its HTTP result yet; "no active
            // run" is the expected transcript answer in this window, even when
            // early stream activity already upgraded the runtime. The
            // authoritative transcript did disprove any remote run though, so
            // the runtime downgrades back to the bare dispatch claim — a
            // later dispatch failure then releases the thread instead of
            // leaving a stale remote-run state behind.
            if runtime.state != .dispatchingSync {
                machine.apply(.threadRuntime(
                    threadId: threadId,
                    state: .dispatchingSync,
                    activeIntentId: runtime.activeIntentId,
                    remoteRunId: nil,
                    error: nil
                ))
            }
            return .inactive(clearedLocalRun: false)
        }
        let hadLocalRun = runtime.activeIntentId != nil
        closeThreadRun(threadId: threadId, intentOutcome: .completed)
        return .inactive(clearedLocalRun: hadLocalRun)
    }

    /// Reconciles refreshed thread-list summaries (`activeRunId` per thread).
    /// Unlike the legacy `refreshRemoteBusyIdsForVisibleThreads`, a summary
    /// that still advertises a run the client already saw terminate does not
    /// resurrect the busy state.
    public mutating func syncThreadSummaries(_ summaries: [(threadId: String, activeRunId: String?)]) {
        for summary in summaries {
            let threadId = summary.threadId
            let activeRunId = summary.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let summaryActive = isSummaryRunConsideredActive(
                threadId: threadId,
                activeRunId: activeRunId
            )
            if summaryActive {
                if !isThreadBusy(threadId) {
                    markRemoteActivity(threadId: threadId, runId: activeRunId)
                }
                continue
            }
            guard let runtime = machine.threadRuntimeByThread[threadId],
                  runtime.state == .runningRemote,
                  runtime.activeIntentId == nil else {
                // Local dispatches and chat starts in flight keep their claim
                // regardless of a (possibly stale) summary.
                continue
            }
            machine.apply(.threadRuntime(
                threadId: threadId,
                state: .idle,
                activeIntentId: nil,
                remoteRunId: nil,
                error: nil
            ))
        }
    }

    // MARK: Internals

    private enum IntentOutcome {
        case completed
        case interrupted
        case failed(String)
    }

    /// True while a locally started chat dispatch has not produced its HTTP
    /// result yet — either the runtime still sits in `dispatching_sync`, or
    /// early stream activity upgraded it to `running_remote` while the
    /// dispatching intent is still awaiting `confirmChatStartAccepted` /
    /// `failLocalDispatch`.
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

    private mutating func recordTerminatedRun(threadId: String, runId: String) {
        let normalized = runId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        lastTerminatedRunIdsByThread[threadId] = normalized
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

    private static func threadId(from event: GaryxChatStreamEvent) -> String {
        switch event {
        case .accepted(_, let threadId),
             .runStart(_, let threadId),
             .assistantDelta(_, let threadId, _, _),
             .assistantBoundary(_, let threadId),
             .toolUse(_, let threadId, _),
             .toolResult(_, let threadId, _),
             .userMessage(_, let threadId, _, _),
             .userAck(_, let threadId, _),
             .threadTitleUpdated(_, let threadId, _),
             .done(_, let threadId),
             .runComplete(_, let threadId),
             .runError(_, let threadId, _),
             .streamInput(_, let threadId, _, _),
             .interrupt(_, let threadId, _),
             .snapshot(let threadId, _),
             .error(_, let threadId, _):
            return threadId
        case .ping, .unknown:
            return ""
        }
    }

    private static func runId(from event: GaryxChatStreamEvent) -> String {
        switch event {
        case .accepted(let runId, _),
             .runStart(let runId, _),
             .assistantDelta(let runId, _, _, _),
             .assistantBoundary(let runId, _),
             .toolUse(let runId, _, _),
             .toolResult(let runId, _, _),
             .userMessage(let runId, _, _, _),
             .userAck(let runId, _, _),
             .threadTitleUpdated(let runId, _, _),
             .done(let runId, _),
             .runComplete(let runId, _),
             .runError(let runId, _, _),
             .error(let runId, _, _):
            return runId
        case .streamInput, .interrupt, .snapshot, .ping, .unknown:
            return ""
        }
    }
}
