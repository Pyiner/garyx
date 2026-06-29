import XCTest
@testable import GaryxMobileCore

/// Reproduction tests for #TASK-1449 (mobile message-state parity walkthrough).
///
/// These are deterministic, UI-free GREEN characterizations of the *current*
/// (buggy) behavior, driven by real `GaryxMobileCore` functions and real data.
/// Each test asserts the present state and documents the objectively-correct
/// semantics (the "ORACLE") in comments.
///
/// IMPORTANT: these are reproduction artifacts, NOT acceptance gates. Where a
/// test models an App-target formula (e.g. the header spinner condition), it
/// mirrors that formula in a LOCAL variable, so a fix that rewires the App
/// surfaces does not touch these locals — the characterization documents the
/// bug but does not gate it. The acceptance gates (red specs) are listed in
/// `docs/design/mobile-message-state-parity.md`.
final class GaryxMobileMessageStateParityReproTests: XCTestCase {

    // MARK: Symptom 1 — conversation surface "kind" is entry-path dependent

    /// The recent-list row tap carries the full summary, so the kind classifies
    /// to `.chat` immediately. The widget / deep-link by-id open has no summary
    /// at open time, so the same thread classifies to `.unresolved` — which the
    /// App layer renders via the workflow ("Workflow Run") surface
    /// (`showResolvingWorkflowThread` → `workflowRunPanelState.beginResolving`).
    /// Same thread, two entry points, two surfaces ⇒ oracle violation.
    func testThreadKindClassificationDivergesByEntryPath() {
        let byId = GaryxWorkflowRunDestination.destination(threadId: "thread::T", summary: nil)
        let bySummary = GaryxWorkflowRunDestination.destination(for: chatSummary("thread::T"))

        XCTAssertEqual(byId, .unresolved(threadId: "thread::T"), "by-id open (no summary) cannot classify yet")
        XCTAssertEqual(bySummary, .chat(threadId: "thread::T"), "by-summary open classifies the chat thread")

        // ORACLE: the rendered surface kind must be identical for the same thread
        // regardless of entry path. The classifier outputs differ, and the App
        // maps `.unresolved` to the workflow surface but `.chat` to chat.
        XCTAssertNotEqual(
            byId, bySummary,
            "BUG: same thread classifies differently by entry path; App renders .unresolved as workflow surface"
        )

        // The objective type is always decidable from server data (thread_type
        // defaults to "chat"); a real workflow thread is the only workflow kind.
        XCTAssertEqual(
            GaryxWorkflowRunDestination.destination(for: workflowSummary("thread::W", runId: "wfr::1")),
            .workflowRun(runId: "wfr::1")
        )
    }

    /// `beginResolving` (entered for any by-id open before the type is known)
    /// puts the panel into a non-idle mode with NO actual workflow run, yet the
    /// App's `isWorkflowRunSurfaceActive` is `mode != .idle` — so an unclassified
    /// chat thread is presented as a workflow surface.
    func testResolvingAnUnknownThreadActivatesWorkflowSurfaceWithoutAWorkflowRun() {
        var state = GaryxWorkflowRunPanelState()
        XCTAssertEqual(state.mode, .idle)

        state.beginResolving(threadId: "thread::T")

        XCTAssertNil(state.activeWorkflowRunId, "resolving a thread by id is not a workflow run")
        let isWorkflowRunSurfaceActive = state.mode != .idle // mirrors GaryxMobileModel+WorkflowRuns.swift:7-14
        XCTAssertTrue(
            isWorkflowRunSurfaceActive,
            "BUG: resolving an unclassified thread activates the workflow surface despite no workflow run"
        )
        // ORACLE: an unclassified by-id open is a neutral chat-loading state and
        // must NOT activate the workflow surface.
    }

    // MARK: Symptom 3 — loading indicator stays stuck after the transcript renders

    /// The top spinner is a LOADING indicator (initial history / render
    /// resolution), which is the correct, intended role. The fix aligns the
    /// loading-complete predicate with the render predicate so the indicator
    /// settles once the window is applied:
    ///
    /// - The mapper renders EVERY snapshot row, substituting a placeholder for an
    ///   unresolved ref (`GaryxRenderUserTurnRow.mobileRow`:
    ///   `mobileMessage(for:) ?? .userStepPlaceholder(for:)`,
    ///   GaryxMobileRenderState.swift) — so the transcript is NOT blank.
    /// - `isAwaitingInitialHistory` now settles to false once `historyLoaded` is
    ///   true (the committed window is applied); out-of-window / unresolved refs
    ///   are placeholdered, not "still loading" (#TASK-1449 symptom 3). Before the
    ///   window is applied it still reports an in-flight resolve.
    func testLoadingIndicatorSettlesOnceWindowAppliedEvenWithOutOfWindowRefs() {
        let snapshot = snapshotWithUnresolvedRef()

        // The transcript renders: the mapper emits a (placeholder) row even though
        // the ref is not present in the local cache.
        let renderedRows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [],
            transcriptMessages: []
        )
        XCTAssertGreaterThanOrEqual(renderedRows.count, 1, "mapper renders a placeholder row — transcript is not blank")

        // Once the committed window is applied (historyLoaded), the loading
        // indicator settles even though the snapshot has an out-of-window ref.
        let awaitingAfterWindowApplied = GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
            threadId: "thread::T",
            historyLoaded: true,
            liveRenderSnapshot: snapshot,
            cachedTranscript: nil
        )
        XCTAssertFalse(awaitingAfterWindowApplied, "loaded + rendered ⇒ indicator settles (no stuck spinner)")
        let headerSpinnerShows = false /* isLoadingSelectedThreadHistory */ || awaitingAfterWindowApplied
        XCTAssertFalse(headerSpinnerShows, "spinner is off over a fully-rendered, loaded transcript")

        // Before the window is applied, an unresolved visible ref is a genuine
        // in-flight resolve and the indicator stays on.
        XCTAssertTrue(
            GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
                threadId: "thread::T",
                historyLoaded: false,
                liveRenderSnapshot: snapshot,
                cachedTranscript: nil
            ),
            "pre-window: still resolving"
        )
        // The indicator carries NO running semantics — purely a loading state.
    }

    // MARK: Fixtures

    private func snapshotWithUnresolvedRef() -> GaryxRenderSnapshot {
        // A windowed snapshot whose single user-turn row references seq 1, which
        // is not present in the (empty) mobile cache / transcript — i.e. an
        // out-of-window / not-yet-materialized ref.
        GaryxRenderSnapshot(
            basedOnSeq: 1,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:1",
                    user: GaryxRenderMessageRef(id: "seq:1", seq: 1, role: "user"),
                    activity: []
                )),
            ],
            window: GaryxRenderWindow(floorSeq: 1, hasMoreAbove: true)
        )
    }

    private func chatSummary(_ id: String) -> GaryxThreadSummary {
        summary(id: id, threadType: "chat", workflowRunId: nil)
    }

    private func workflowSummary(_ id: String, runId: String) -> GaryxThreadSummary {
        summary(id: id, threadType: "workflow_run", workflowRunId: runId)
    }

    private func summary(id: String, threadType: String, workflowRunId: String?) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: "Test Thread",
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            teamId: nil,
            teamName: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil,
            threadType: threadType,
            workflowRunId: workflowRunId
        )
    }
}
