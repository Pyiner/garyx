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
/// mirrors that formula in a LOCAL variable so a fix that rewires App surfaces
/// does not silently rewrite the characterization.
final class GaryxMobileMessageStateParityReproTests: XCTestCase {
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

}
