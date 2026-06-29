import XCTest
@testable import GaryxMobileCore

/// #TASK-1453 problem A — the composer's stop button stayed on an *idle* chat
/// thread whose tail row is a capsule card.
///
/// The composer's stop button is driven only by `isThreadBusy`
/// (`runTracker.isThreadBusy || runStateByThread[id].busy`); `runStateByThread`
/// is `GaryxTranscriptRunStateReducer.reduce(committedWindow)`. The committed
/// window served by `/api/threads/history` nests the control payload under
/// `message.message.control` (only the live SSE event shape carries it at the
/// top-level `control`). The reducer used to read only `message.control` and
/// `message.content`, so it was **blind** to every `/history` control record:
/// it never observed `run_start` (could not show an active run) nor
/// `run_complete` (could not let the app clear a stuck local run on a resync /
/// `range_rewrite` refetch / cold open) — leaving the stop button on an idle
/// thread. The rewrite planner already reads `message.message`; this aligns the
/// run-state reducer with it.
///
/// These tests decode the exact `/history` wrapper wire shape through
/// `GaryxThreadTranscript` (the production cold-open / refetch path) and assert
/// the reduced run state, so they reproduce the bug headlessly with no UI.
final class GaryxCapsuleTailComposerBusyTests: XCTestCase {
    private func reduceHistory(_ json: String) throws -> GaryxTranscriptRunState {
        let transcript = try JSONDecoder().decode(GaryxThreadTranscript.self, from: Data(json.utf8))
        return GaryxTranscriptRunStateReducer.reduce(transcript.messages)
    }

    /// A `/history` window for a completed run that ends with a capsule card.
    /// Mirrors the real wire shape: top-level rendering fields plus the committed
    /// record (control payload) nested under `message`.
    private let completedCapsuleRunHistory = """
    {
      "ok": true,
      "messages": [
        {"index":0,"role":"system","kind":"control","internal_kind":"control","content":"","text":"",
         "message":{"role":"system","kind":"control","internal_kind":"control",
                    "control":{"kind":"run_start","run_id":"run-1","thread_id":"thread::T","at":"2026-06-29T00:00:00Z"}}},
        {"index":1,"role":"user","kind":"user_input","content":"make a capsule","text":"make a capsule",
         "message":{"role":"user","content":"make a capsule"}},
        {"index":2,"role":"assistant","kind":"assistant_reply","content":"done","text":"done",
         "message":{"role":"assistant","content":"done"}},
        {"index":3,"role":"system","kind":"control","internal_kind":"control","content":"","text":"",
         "message":{"role":"system","kind":"control","internal_kind":"control",
                    "control":{"kind":"capsule_attached","capsule_id":"cap-1","revision":1,"action":"created","run_id":"run-1","thread_id":"thread::T"}}},
        {"index":4,"role":"system","kind":"control","internal_kind":"control","content":"","text":"",
         "message":{"role":"system","kind":"control","internal_kind":"control",
                    "control":{"kind":"done","run_id":"run-1","thread_id":"thread::T"}}},
        {"index":5,"role":"system","kind":"control","internal_kind":"control","content":"","text":"",
         "message":{"role":"system","kind":"control","internal_kind":"control",
                    "control":{"kind":"run_complete","run_id":"run-1","status":"completed","thread_id":"thread::T"}}}
      ],
      "pending_user_inputs": [],
      "message_stats": {"returned_messages": 6}
    }
    """

    /// The same run mid-flight: `run_start` committed, no `run_complete` yet.
    private let activeRunHistory = """
    {
      "ok": true,
      "messages": [
        {"index":0,"role":"system","kind":"control","internal_kind":"control","content":"","text":"",
         "message":{"role":"system","kind":"control","internal_kind":"control",
                    "control":{"kind":"run_start","run_id":"run-1","thread_id":"thread::T","at":"2026-06-29T00:00:00Z"}}},
        {"index":1,"role":"user","kind":"user_input","content":"make a capsule","text":"make a capsule",
         "message":{"role":"user","content":"make a capsule"}},
        {"index":2,"role":"assistant","kind":"assistant_reply","content":"working","text":"working",
         "message":{"role":"assistant","content":"working"}}
      ],
      "pending_user_inputs": [],
      "message_stats": {"returned_messages": 3}
    }
    """

    /// Idle chat thread, tail row is a capsule card: the committed window shows
    /// the run completed, so the composer must read the thread as idle (no stop
    /// button). The reducer must observe the terminal `run_complete` for the app
    /// to clear any stale local-run state on a resync.
    func testCompletedCapsuleRunHistoryIsObservedAsIdle() throws {
        let state = try reduceHistory(completedCapsuleRunHistory)
        XCTAssertFalse(state.busy, "an idle completed thread must not be busy")
        XCTAssertNil(state.activeRunId)
        XCTAssertEqual(
            state.terminalStatus, "completed",
            "the reducer must observe run_complete from the /history wire shape (control nested under message)"
        )
        XCTAssertEqual(state.activity, .idle)
    }

    /// An active run's `/history` window must read as busy — guards the inverse
    /// blindness (a genuinely running thread showing the send button).
    func testActiveRunHistoryIsObservedAsBusy() throws {
        let state = try reduceHistory(activeRunHistory)
        XCTAssertTrue(state.busy, "a run with run_start and no run_complete must be busy")
        XCTAssertEqual(state.activeRunId, "run-1")
    }

    /// The capsule marker itself must not perturb run state — it is a side-list
    /// render artifact, not a run-lifecycle control.
    func testCapsuleAttachedControlDoesNotChangeBusy() throws {
        let withoutCapsule = """
        {
          "ok": true,
          "messages": [
            {"index":0,"role":"system","kind":"control","internal_kind":"control","content":"","text":"",
             "message":{"role":"system","kind":"control","internal_kind":"control",
                        "control":{"kind":"run_start","run_id":"run-1","thread_id":"thread::T"}}},
            {"index":1,"role":"assistant","kind":"assistant_reply","content":"done","text":"done",
             "message":{"role":"assistant","content":"done"}},
            {"index":2,"role":"system","kind":"control","internal_kind":"control","content":"","text":"",
             "message":{"role":"system","kind":"control","internal_kind":"control",
                        "control":{"kind":"run_complete","run_id":"run-1","status":"completed","thread_id":"thread::T"}}}
          ],
          "pending_user_inputs": [],
          "message_stats": {"returned_messages": 3}
        }
        """
        let withCapsule = try reduceHistory(completedCapsuleRunHistory)
        let plain = try reduceHistory(withoutCapsule)
        XCTAssertEqual(withCapsule.busy, plain.busy)
        XCTAssertEqual(withCapsule.terminalStatus, plain.terminalStatus)
    }

    /// Regression guard: the live SSE shape (control at the top level of the
    /// committed message) must keep working unchanged after the fix.
    func testLiveSseControlShapeStillObservesRunLifecycle() throws {
        let runStart = GaryxTranscriptMessage(
            index: 0,
            role: .system,
            kind: "control",
            internalKind: "control",
            internalMessage: true,
            control: .object(["kind": .string("run_start"), "run_id": .string("run-1")]),
            likelyUserVisible: false
        )
        let runComplete = GaryxTranscriptMessage(
            index: 1,
            role: .system,
            kind: "control",
            internalKind: "control",
            internalMessage: true,
            control: .object(["kind": .string("run_complete"), "status": .string("completed")]),
            likelyUserVisible: false
        )
        let state = GaryxTranscriptRunStateReducer.reduce([runStart, runComplete])
        XCTAssertFalse(state.busy)
        XCTAssertEqual(state.terminalStatus, "completed")
    }
}
