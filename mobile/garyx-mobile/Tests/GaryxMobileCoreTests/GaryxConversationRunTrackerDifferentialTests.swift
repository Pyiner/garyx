import XCTest
@testable import GaryxMobileCore

/// Differential suite for the run-state migration: replays the legacy
/// scattered-flags semantics (`isSending` / `activeRunThreadId` /
/// `remoteBusyThreadIds` / `pendingChatStartThreadIds` /
/// `terminatedActiveRunIdsByThread`, reproduced verbatim in
/// `LegacyRunFlagsOracle`) against `GaryxConversationRunTracker` over the
/// same operation sequences and asserts the user-visible busy signal stays
/// identical — except for the documented legacy bugs the tracker fixes,
/// which get explicit divergence tests asserting the new behavior.
final class GaryxConversationRunTrackerDifferentialTests: XCTestCase {
    // MARK: Legacy oracle

    /// Verbatim port of the pre-migration flag bookkeeping in
    /// GaryxMobileModel (+Composer / +Streaming / +Threads).
    ///
    /// With `appliesTrackerFixes` the oracle additionally applies the two
    /// intentional behavior corrections the tracker ships (a transient
    /// gateway error and a rejected queued input no longer cancel a local
    /// run claim), so randomized sequences can assert strict equality:
    /// new behavior == legacy behavior + exactly these documented deltas.
    private struct LegacyRunFlagsOracle {
        var appliesTrackerFixes = false
        var isSending = false
        var activeRunThreadId: String?
        var remoteBusyThreadIds: Set<String> = []
        var pendingChatStartThreadIds: Set<String> = []
        var terminatedActiveRunIdsByThread: [String: String] = [:]

        func isThreadBusy(_ threadId: String) -> Bool {
            activeRunThreadId == threadId || remoteBusyThreadIds.contains(threadId)
        }

        mutating func clearActiveRunState(for threadId: String) {
            if activeRunThreadId == threadId {
                activeRunThreadId = nil
            }
            isSending = activeRunThreadId != nil
        }

        // send() optimistic phase
        mutating func sendOptimistic(threadId: String) {
            isSending = true
            activeRunThreadId = threadId
            pendingChatStartThreadIds.insert(threadId)
        }

        // startChatRunViaGateway success
        mutating func chatStartAccepted(requestedThreadId: String, acceptedThreadId: String) {
            pendingChatStartThreadIds.remove(requestedThreadId)
            pendingChatStartThreadIds.remove(acceptedThreadId)
            activeRunThreadId = acceptedThreadId
            remoteBusyThreadIds.insert(acceptedThreadId)
        }

        // send() catch path
        mutating func chatStartFailed(threadId: String) {
            pendingChatStartThreadIds.remove(threadId)
            clearActiveRunState(for: threadId)
        }

        // submitQueuedInputViaGateway result handling
        mutating func queuedInputAccepted(threadId: String) {
            remoteBusyThreadIds.insert(threadId)
        }

        mutating func queuedInputRejected(threadId: String) {
            // Legacy only marks the message; busy flags stay untouched.
        }

        // updateRemoteBusyState + the busy-relevant parts of handle()
        mutating func applyLiveEvent(_ event: GaryxChatStreamEvent, transientError: Bool) {
            let threadId = Self.threadId(from: event)
            guard !threadId.isEmpty else { return }
            applyReplayEvent(event, transientError: transientError)
            switch event {
            case .done(let runId, _), .runComplete(let runId, _):
                recordTerminated(threadId: threadId, runId: runId)
                remoteBusyThreadIds.remove(threadId)
                clearActiveRunState(for: threadId)
            case .interrupt(_, _, let abortedRuns):
                if let aborted = abortedRuns
                    .map({ $0.trimmingCharacters(in: .whitespacesAndNewlines) })
                    .last(where: { !$0.isEmpty }) {
                    terminatedActiveRunIdsByThread[threadId] = aborted
                }
                remoteBusyThreadIds.remove(threadId)
                clearActiveRunState(for: threadId)
            case .error(let runId, _, _):
                if transientError {
                    remoteBusyThreadIds.insert(threadId)
                    // Tracker fix: a transient gateway blip no longer drops
                    // the local run claim.
                    if !appliesTrackerFixes {
                        clearActiveRunState(for: threadId)
                    }
                } else {
                    recordTerminated(threadId: threadId, runId: runId)
                    remoteBusyThreadIds.remove(threadId)
                    clearActiveRunState(for: threadId)
                }
            default:
                break
            }
        }

        // The legacy replay path ran updateRemoteBusyState only.
        mutating func applyReplayEvent(_ event: GaryxChatStreamEvent, transientError: Bool) {
            _ = transientError
            let threadId = Self.threadId(from: event)
            guard !threadId.isEmpty else { return }
            switch event {
            case .accepted, .userMessage, .assistantDelta, .assistantBoundary,
                 .userAck, .toolUse, .toolResult:
                remoteBusyThreadIds.insert(threadId)
            case .streamInput(let status, _, _, _):
                if GaryxGatewayStreamStatusClassifier.isSuccessfulStreamInput(status) {
                    remoteBusyThreadIds.insert(threadId)
                } else if !(appliesTrackerFixes && activeRunThreadId == threadId) {
                    // Tracker fix: a rejected queued input no longer cancels
                    // a local run claim.
                    remoteBusyThreadIds.remove(threadId)
                }
            case .done, .runComplete, .error, .interrupt:
                remoteBusyThreadIds.remove(threadId)
            default:
                break
            }
        }

        // updateThreadRuntimeState
        mutating func reconcileTranscript(
            threadId: String,
            activeRunPresent: Bool,
            activeRunId: String?,
            hasActivePendingInput: Bool
        ) {
            let isActive = GaryxMobileThreadActivityModel.shouldTreatThreadRuntimeAsActive(
                activeRunPresent: activeRunPresent,
                activeRunId: activeRunId,
                hasActivePendingInput: hasActivePendingInput,
                lastTerminatedRunId: terminatedActiveRunIdsByThread[threadId]
            )
            if isActive {
                remoteBusyThreadIds.insert(threadId)
            } else {
                remoteBusyThreadIds.remove(threadId)
                if !pendingChatStartThreadIds.contains(threadId), activeRunThreadId == threadId {
                    clearActiveRunState(for: threadId)
                }
            }
        }

        // refreshRemoteBusyIdsForVisibleThreads
        mutating func syncThreadSummaries(_ summaries: [(threadId: String, activeRunId: String?)]) {
            for summary in summaries {
                let active = !(summary.activeRunId?
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                    .isEmpty ?? true)
                if active {
                    remoteBusyThreadIds.insert(summary.threadId)
                } else {
                    remoteBusyThreadIds.remove(summary.threadId)
                }
            }
        }

        private mutating func recordTerminated(threadId: String, runId: String) {
            let normalized = runId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !normalized.isEmpty else { return }
            terminatedActiveRunIdsByThread[threadId] = normalized
        }

        private static func threadId(from event: GaryxChatStreamEvent) -> String {
            switch event {
            case .accepted(_, let threadId),
                 .assistantDelta(_, let threadId, _, _),
                 .assistantBoundary(_, let threadId),
                 .toolUse(_, let threadId, _),
                 .toolResult(_, let threadId, _),
                 .userMessage(_, let threadId, _, _),
                 .userAck(_, let threadId, _),
                 .threadTitleUpdated(_, let threadId, _),
                 .done(_, let threadId),
                 .runComplete(_, let threadId),
                 .streamInput(_, let threadId, _, _),
                 .interrupt(_, let threadId, _),
                 .snapshot(let threadId, _),
                 .error(_, let threadId, _):
                return threadId
            case .ping, .unknown:
                return ""
            }
        }
    }

    // MARK: Shared harness

    private enum DiffOp {
        case send
        case chatStartAccepted(runId: String)
        case chatStartFailed
        case queueInputAccepted
        case queueInputRejected
        case activity(runId: String)
        case userAck
        case streamInputEventOk
        case streamInputEventRejected
        case done(runId: String)
        case interrupt(abortedRunId: String?)
        case errorTransient
        case errorFatal(runId: String)
        case reconcileActive(runId: String)
        case reconcileInactive
    }

    private struct DiffHarness {
        let threadId: String
        var oracle = LegacyRunFlagsOracle()
        var tracker = GaryxConversationRunTracker()
        private var nextIntentSerial = 0
        private var dispatchIntentId: String?
        private var steerIntentId: String?

        init(threadId: String) {
            self.threadId = threadId
        }

        mutating func apply(_ op: DiffOp) {
            switch op {
            case .send:
                // send() routes to the queue flow when the thread is busy;
                // mirror that control flow so both sides take the same path.
                if oracle.isThreadBusy(threadId) || tracker.isThreadBusy(threadId) {
                    let intentId = makeIntentId()
                    steerIntentId = intentId
                    tracker.beginQueuedSteer(threadId: threadId, intentId: intentId, text: "queued")
                    return
                }
                let intentId = makeIntentId()
                dispatchIntentId = intentId
                oracle.sendOptimistic(threadId: threadId)
                _ = tracker.beginLocalDispatch(threadId: threadId, intentId: intentId, text: "send")
            case .chatStartAccepted(let runId):
                guard let intentId = dispatchIntentId else { return }
                oracle.chatStartAccepted(requestedThreadId: threadId, acceptedThreadId: threadId)
                tracker.confirmChatStartAccepted(
                    requestedThreadId: threadId,
                    acceptedThreadId: threadId,
                    intentId: intentId,
                    runId: runId
                )
            case .chatStartFailed:
                guard let intentId = dispatchIntentId else { return }
                dispatchIntentId = nil
                oracle.chatStartFailed(threadId: threadId)
                tracker.failLocalDispatch(threadId: threadId, intentId: intentId, error: "failed")
            case .queueInputAccepted:
                guard let intentId = steerIntentId else { return }
                oracle.queuedInputAccepted(threadId: threadId)
                tracker.confirmQueuedSteerAccepted(
                    threadId: threadId,
                    intentId: intentId,
                    pendingInputId: "p-\(intentId)"
                )
            case .queueInputRejected:
                guard let intentId = steerIntentId else { return }
                steerIntentId = nil
                oracle.queuedInputRejected(threadId: threadId)
                tracker.failQueuedSteer(threadId: threadId, intentId: intentId, error: "rejected")
            case .activity(let runId):
                applyEvent(.assistantDelta(runId: runId, threadId: threadId, delta: "x", metadata: nil))
            case .userAck:
                applyEvent(.userAck(runId: "", threadId: threadId, pendingInputId: nil))
            case .streamInputEventOk:
                applyEvent(.streamInput(status: "queued", threadId: threadId, clientIntentId: nil, pendingInputId: nil))
            case .streamInputEventRejected:
                applyEvent(.streamInput(status: "rejected", threadId: threadId, clientIntentId: nil, pendingInputId: nil))
            case .done(let runId):
                dispatchIntentId = nil
                steerIntentId = nil
                applyEvent(.done(runId: runId, threadId: threadId))
            case .interrupt(let abortedRunId):
                dispatchIntentId = nil
                steerIntentId = nil
                applyEvent(.interrupt(
                    status: "ok",
                    threadId: threadId,
                    abortedRuns: abortedRunId.map { [$0] } ?? []
                ))
            case .errorTransient:
                applyEvent(.error(runId: "", threadId: threadId, error: "request timed out"))
            case .errorFatal(let runId):
                dispatchIntentId = nil
                steerIntentId = nil
                applyEvent(.error(runId: runId, threadId: threadId, error: "provider exploded"))
            case .reconcileActive(let runId):
                oracle.reconcileTranscript(
                    threadId: threadId,
                    activeRunPresent: true,
                    activeRunId: runId,
                    hasActivePendingInput: false
                )
                _ = tracker.reconcileTranscriptRuntime(
                    threadId: threadId,
                    activeRunPresent: true,
                    activeRunId: runId,
                    hasActivePendingInput: false
                )
            case .reconcileInactive:
                oracle.reconcileTranscript(
                    threadId: threadId,
                    activeRunPresent: false,
                    activeRunId: nil,
                    hasActivePendingInput: false
                )
                _ = tracker.reconcileTranscriptRuntime(
                    threadId: threadId,
                    activeRunPresent: false,
                    activeRunId: nil,
                    hasActivePendingInput: false
                )
            }
        }

        private mutating func applyEvent(_ event: GaryxChatStreamEvent) {
            let transient: Bool
            if case .error(_, _, let message) = event {
                transient = GaryxGatewayStreamStatusClassifier.isTransientGatewayErrorMessage(message)
            } else {
                transient = false
            }
            oracle.applyLiveEvent(event, transientError: transient)
            tracker.apply(streamEvent: event)
        }

        private mutating func makeIntentId() -> String {
            nextIntentSerial += 1
            return "intent-\(nextIntentSerial)"
        }
    }

    private func assertBusyParity(
        _ harness: DiffHarness,
        step: Int,
        history: [String],
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(
            harness.tracker.isThreadBusy(harness.threadId),
            harness.oracle.isThreadBusy(harness.threadId),
            "busy diverged at step \(step): \(history.joined(separator: " → "))",
            file: file,
            line: line
        )
    }

    // MARK: Scripted parity sequences

    private func runParitySequence(_ ops: [(String, DiffOp)]) {
        var harness = DiffHarness(threadId: "t1")
        var history: [String] = []
        for (index, namedOp) in ops.enumerated() {
            history.append(namedOp.0)
            harness.apply(namedOp.1)
            assertBusyParity(harness, step: index, history: history)
        }
    }

    func testParityHappySendPath() {
        runParitySequence([
            ("send", .send),
            ("accepted", .chatStartAccepted(runId: "run-1")),
            ("activity", .activity(runId: "run-1")),
            ("done", .done(runId: "run-1")),
            ("reconcileInactive", .reconcileInactive),
        ])
    }

    func testParityFailedChatStart() {
        runParitySequence([
            ("send", .send),
            ("chatStartFailed", .chatStartFailed),
            ("reconcileInactive", .reconcileInactive),
        ])
    }

    func testParityQueuedSteerDuringRun() {
        runParitySequence([
            ("send", .send),
            ("accepted", .chatStartAccepted(runId: "run-1")),
            ("send→queue", .send),
            ("queueAccepted", .queueInputAccepted),
            ("userAck", .userAck),
            ("activity", .activity(runId: "run-1")),
            ("done", .done(runId: "run-1")),
        ])
    }

    func testParityTransientErrorKeepsBusy() {
        runParitySequence([
            ("send", .send),
            ("accepted", .chatStartAccepted(runId: "run-1")),
            ("errorTransient", .errorTransient),
            ("activity", .activity(runId: "run-1")),
            ("done", .done(runId: "run-1")),
        ])
    }

    func testParityChatStartWindowSurvivesInactiveTranscript() {
        runParitySequence([
            ("send", .send),
            ("reconcileInactive", .reconcileInactive),
            ("accepted", .chatStartAccepted(runId: "run-1")),
            ("done", .done(runId: "run-1")),
        ])
    }

    func testParityDroppedTerminalEventReconciles() {
        runParitySequence([
            ("send", .send),
            ("accepted", .chatStartAccepted(runId: "run-1")),
            ("activity", .activity(runId: "run-1")),
            // The done event never arrives; the authoritative transcript
            // reload reports no active run and must release the thread.
            ("reconcileInactive", .reconcileInactive),
        ])
    }

    func testParityStaleActiveRunSnapshotAfterDone() {
        runParitySequence([
            ("send", .send),
            ("accepted", .chatStartAccepted(runId: "run-1")),
            ("done", .done(runId: "run-1")),
            // A racing transcript still reports run-1 as active; the client
            // observed its termination, so both sides must stay idle.
            ("staleReconcile", .reconcileActive(runId: "run-1")),
            ("freshReconcile", .reconcileActive(runId: "run-2")),
        ])
    }

    func testParityRemoteOnlyRunObservedFromStream() {
        runParitySequence([
            ("activity", .activity(runId: "run-9")),
            ("streamInputOk", .streamInputEventOk),
            ("streamInputRejected", .streamInputEventRejected),
            ("activity", .activity(runId: "run-9")),
            ("interrupt", .interrupt(abortedRunId: "run-9")),
            ("staleReconcile", .reconcileActive(runId: "run-9")),
        ])
    }

    // MARK: Randomized parity

    /// Full operation set against the corrected oracle: the tracker must
    /// equal legacy behavior plus exactly the documented fixes.
    func testRandomizedSingleThreadParityWithDocumentedFixes() {
        var rng = SeededGenerator(seed: 0x6172_7978)
        for iteration in 0..<200 {
            var harness = DiffHarness(threadId: "t1")
            harness.oracle.appliesTrackerFixes = true
            var history: [String] = []
            for step in 0..<40 {
                let op = randomOp(using: &rng, includeFixDivergentOps: true)
                history.append(op.0)
                harness.apply(op.1)
                assertBusyParity(harness, step: step, history: ["iteration \(iteration)"] + history)
            }
        }
    }

    /// Strict legacy parity over the operation set that does not touch the
    /// intentionally fixed corners (no transient errors, no rejected queued
    /// inputs): on these flows the migration must be behavior-identical.
    func testRandomizedSingleThreadStrictLegacyParity() {
        var rng = SeededGenerator(seed: 0x6D61_6368)
        for iteration in 0..<200 {
            var harness = DiffHarness(threadId: "t1")
            var history: [String] = []
            for step in 0..<40 {
                let op = randomOp(using: &rng, includeFixDivergentOps: false)
                history.append(op.0)
                harness.apply(op.1)
                assertBusyParity(harness, step: step, history: ["iteration \(iteration)"] + history)
            }
        }
    }

    private func randomOp(
        using rng: inout SeededGenerator,
        includeFixDivergentOps: Bool
    ) -> (String, DiffOp) {
        let runId = "run-\(rng.next() % 3)"
        let bound: UInt64 = includeFixDivergentOps ? 14 : 12
        switch rng.next() % bound {
        case 0: return ("send", .send)
        case 1: return ("accepted", .chatStartAccepted(runId: runId))
        case 2: return ("chatStartFailed", .chatStartFailed)
        case 3: return ("queueAccepted", .queueInputAccepted)
        case 4: return ("queueRejected", .queueInputRejected)
        case 5: return ("activity", .activity(runId: runId))
        case 6: return ("userAck", .userAck)
        case 7: return ("streamInputOk", .streamInputEventOk)
        case 8: return ("done", .done(runId: runId))
        case 9: return ("errorFatal", .errorFatal(runId: runId))
        case 10: return ("reconcileInactive", .reconcileInactive)
        case 11: return ("interrupt", .interrupt(abortedRunId: runId))
        case 12: return ("streamInputRejected", .streamInputEventRejected)
        default: return ("errorTransient", .errorTransient)
        }
    }

    private struct SeededGenerator {
        private var state: UInt64

        init(seed: UInt64) {
            state = seed == 0 ? 0x9E37_79B9_7F4A_7C15 : seed
        }

        mutating func next() -> UInt64 {
            // SplitMix64: deterministic across runs and platforms.
            state &+= 0x9E37_79B9_7F4A_7C15
            var z = state
            z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
            z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
            return z ^ (z >> 31)
        }
    }

    // MARK: Documented divergences (legacy bugs the tracker fixes)

    /// Legacy bug: `activeRunThreadId` was a single global slot. Sending in
    /// thread B while thread A's chat start was still in flight silently
    /// dropped A's busy claim, so A's thinking indicator vanished. The
    /// per-thread runtime keeps both claims.
    func testDivergenceConcurrentDispatchesKeepBothThreadsBusy() {
        var oracle = LegacyRunFlagsOracle()
        var tracker = GaryxConversationRunTracker()

        oracle.sendOptimistic(threadId: "tA")
        _ = tracker.beginLocalDispatch(threadId: "tA", intentId: "iA", text: "a")
        oracle.sendOptimistic(threadId: "tB")
        _ = tracker.beginLocalDispatch(threadId: "tB", intentId: "iB", text: "b")

        XCTAssertFalse(oracle.isThreadBusy("tA"), "documents the legacy bug")
        XCTAssertTrue(tracker.isThreadBusy("tA"), "thread A keeps its dispatch claim")
        XCTAssertTrue(tracker.isThreadBusy("tB"))
    }

    /// Legacy bug: the queued-input fallback re-dispatched as a fresh chat
    /// start without marking the chat-start window, so a racing transcript
    /// reload cleared the sending state mid-dispatch. The tracker's
    /// `dispatching_sync` runtime protects the window on every dispatch path.
    func testDivergenceFallbackDispatchSurvivesInactiveTranscript() {
        var tracker = GaryxConversationRunTracker()
        _ = tracker.beginLocalDispatch(threadId: "t1", intentId: "i1", text: "fallback dispatch")

        let outcome = tracker.reconcileTranscriptRuntime(
            threadId: "t1",
            activeRunPresent: false,
            activeRunId: nil,
            hasActivePendingInput: false
        )

        XCTAssertEqual(outcome, .inactive(clearedLocalRun: false))
        XCTAssertTrue(tracker.isThreadBusy("t1"), "chat-start window stays claimed")
    }

    /// Legacy bug: the thread-list summary sync re-marked a thread busy from
    /// a stale `activeRunId` even when the client had already observed that
    /// run terminate, pinning the busy state until the next refresh.
    func testDivergenceSummarySyncIgnoresTerminatedRun() {
        var oracle = LegacyRunFlagsOracle()
        var tracker = GaryxConversationRunTracker()
        let doneEvent = GaryxChatStreamEvent.done(runId: "run-1", threadId: "t1")
        oracle.applyLiveEvent(doneEvent, transientError: false)
        tracker.apply(streamEvent: doneEvent)

        oracle.syncThreadSummaries([(threadId: "t1", activeRunId: "run-1")])
        tracker.syncThreadSummaries([(threadId: "t1", activeRunId: "run-1")])

        XCTAssertTrue(oracle.isThreadBusy("t1"), "documents the legacy bug")
        XCTAssertFalse(tracker.isThreadBusy("t1"), "terminated runs do not resurrect busy state")

        // A genuinely new run still marks the thread busy.
        tracker.syncThreadSummaries([(threadId: "t1", activeRunId: "run-2")])
        XCTAssertTrue(tracker.isThreadBusy("t1"))
    }

    /// Legacy bug: a transient gateway error dropped the local run claim
    /// (while keeping the coarse busy bit), so a later rejected queued input
    /// turned a still-running thread fully idle and the stop button and
    /// thinking indicator vanished mid-run. The tracker keeps the claim
    /// through transient noise; only terminal signals release it.
    func testDivergenceTransientErrorThenRejectedInputKeepsClaimedRun() {
        var oracle = LegacyRunFlagsOracle()
        var tracker = GaryxConversationRunTracker()

        oracle.sendOptimistic(threadId: "t1")
        _ = tracker.beginLocalDispatch(threadId: "t1", intentId: "i1", text: "send")
        oracle.chatStartAccepted(requestedThreadId: "t1", acceptedThreadId: "t1")
        tracker.confirmChatStartAccepted(
            requestedThreadId: "t1",
            acceptedThreadId: "t1",
            intentId: "i1",
            runId: "run-1"
        )

        let transient = GaryxChatStreamEvent.error(runId: "run-1", threadId: "t1", error: "request timed out")
        oracle.applyLiveEvent(transient, transientError: true)
        tracker.apply(streamEvent: transient)

        let rejected = GaryxChatStreamEvent.streamInput(
            status: "rejected",
            threadId: "t1",
            clientIntentId: nil,
            pendingInputId: nil
        )
        oracle.applyLiveEvent(rejected, transientError: false)
        tracker.apply(streamEvent: rejected)

        XCTAssertFalse(oracle.isThreadBusy("t1"), "documents the legacy bug")
        XCTAssertTrue(tracker.isThreadBusy("t1"), "the claimed run survives transient noise")

        let done = GaryxChatStreamEvent.done(runId: "run-1", threadId: "t1")
        tracker.apply(streamEvent: done)
        XCTAssertFalse(tracker.isThreadBusy("t1"), "terminal signals still release the run")
    }

    /// Legacy bug: the event replay path only ran the coarse busy-set update,
    /// so a replayed transient gateway error dropped the busy state that the
    /// live path would have kept.
    func testDivergenceReplayedTransientErrorKeepsBusy() {
        var oracle = LegacyRunFlagsOracle()
        var tracker = GaryxConversationRunTracker()
        let activity = GaryxChatStreamEvent.assistantDelta(
            runId: "run-1",
            threadId: "t1",
            delta: "x",
            metadata: nil
        )
        oracle.applyReplayEvent(activity, transientError: false)
        tracker.apply(streamEvent: activity)

        let transientError = GaryxChatStreamEvent.error(
            runId: "run-1",
            threadId: "t1",
            error: "websocket connection closed"
        )
        oracle.applyReplayEvent(transientError, transientError: true)
        tracker.apply(streamEvent: transientError)

        XCTAssertFalse(oracle.isThreadBusy("t1"), "documents the legacy bug")
        XCTAssertTrue(tracker.isThreadBusy("t1"), "transient errors keep the run busy on replay too")
    }
}
