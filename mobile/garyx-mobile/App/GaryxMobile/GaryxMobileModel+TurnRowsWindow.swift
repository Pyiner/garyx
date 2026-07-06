import Foundation

// TASK-1751 P3 — floor-anchored render window over the selected thread's turn
// rows, plus P4 per-thread residency trimming. The window state is a plain
// (non-published) var; the floor is only ever written from these event
// handlers, never from the body getter, so no ObservableObject change is
// published during a SwiftUI view update.
extension GaryxMobileModel {
    /// Bump the published revision so a window change re-renders the body.
    private func bumpSelectedTurnRowsWindowRevision() {
        selectedTurnRowsWindowRevision &+= 1
    }

    /// Reset the window + memo on a thread switch. Event-driven (called from
    /// `showSelectedThread` / `openNewThreadDraft`), never from the body.
    func resetSelectedTurnRowsWindow() {
        selectedTurnRowsCache.invalidate()
        guard selectedTurnRowsWindowState.floorRowId != nil else { return }
        selectedTurnRowsWindowState = GaryxTurnRowsWindowState()
        bumpSelectedTurnRowsWindowRevision()
    }

    /// Lock the floor the first time rows appear for the selected thread. Once
    /// the floor is set, this is a no-op — so a streaming tail append keeps the
    /// same floor (the window grows only at the bottom, never sliding the head).
    /// Event-driven (message/render write funnels).
    func lockSelectedTurnRowsWindowFloorIfNeeded() {
        guard selectedTurnRowsWindowState.floorRowId == nil else { return }
        let full = selectedThreadFullTurnRows()
        guard !full.isEmpty else { return }
        let resolved = GaryxTurnRowsWindowPlanner.resolve(
            rows: full,
            state: selectedTurnRowsWindowState
        )
        guard resolved.state != selectedTurnRowsWindowState else { return }
        selectedTurnRowsWindowState = resolved.state
        bumpSelectedTurnRowsWindowRevision()
    }

    /// Reveal the next older `expandStep` rows already held in memory (scroll-up
    /// boundary / Load-earlier tap when the window still hides rows). Lowers the
    /// floor; never a network fetch.
    func expandSelectedTurnRowsWindow() {
        let full = selectedThreadFullTurnRows()
        let next = GaryxTurnRowsWindowPlanner.expand(rows: full, state: selectedTurnRowsWindowState)
        guard next != selectedTurnRowsWindowState else { return }
        selectedTurnRowsWindowState = next
        bumpSelectedTurnRowsWindowRevision()
    }

    /// After a network older-history page is prepended, extend the floor to the
    /// new oldest row so the fetched page is visible (the network path only runs
    /// when the window was already exhausted — floor at index 0 — so the prepend
    /// would otherwise be hidden below the anchored floor). Event-driven.
    func extendSelectedTurnRowsWindowToLoadedHistory() {
        let full = selectedThreadFullTurnRows()
        guard let firstId = full.first?.id else { return }
        guard selectedTurnRowsWindowState.floorRowId != firstId else { return }
        selectedTurnRowsWindowState = GaryxTurnRowsWindowState(floorRowId: firstId)
        bumpSelectedTurnRowsWindowRevision()
    }

    /// True when the window already shows every in-memory row (gates the
    /// two-stage boundary action and the Load-earlier button).
    var isSelectedTurnRowsWindowExhausted: Bool {
        GaryxTurnRowsWindowPlanner.isWindowExhausted(
            rows: selectedThreadFullTurnRows(),
            state: selectedTurnRowsWindowState
        )
    }

    /// Whether there is more renderable history to reveal — either window-hidden
    /// in-memory rows, or older committed pages still on the gateway.
    var selectedThreadHasMoreRenderableHistory: Bool {
        !isSelectedTurnRowsWindowExhausted || selectedThreadHasMoreHistoryBefore
    }

    /// Two-stage scroll-up / Load-earlier boundary action: reveal window-hidden
    /// in-memory rows first (instant), and only fetch an older network page once
    /// the window is exhausted.
    func advanceSelectedThreadHistoryBoundary() async {
        if !isSelectedTurnRowsWindowExhausted {
            expandSelectedTurnRowsWindow()
            return
        }
        await loadOlderSelectedThreadHistory()
    }

    // MARK: - P4 residency

    /// Mark a thread most-recently-used and evict over-cap least-recently-used
    /// threads' in-memory projections. Called from every per-thread write funnel.
    func touchThreadResidency(_ threadId: String) {
        let id = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty else { return }
        threadResidencyTracker.touch(id)
        trimThreadResidency()
    }

    func trimThreadResidency() {
        // Fast path: the tracker can only evict when it holds more than the cap
        // (evictable ⊆ resident), so skip the pinned-set scan otherwise.
        guard threadResidencyTracker.count > threadResidencyTracker.maxResidentThreads else { return }
        let pinned = residencyPinnedThreadIds()
        let evicted = threadResidencyTracker.evict(pinned: pinned)
        for id in evicted {
            evictThreadProjections(id)
        }
    }

    /// Threads that must never be evicted: the open thread, the stream-owned
    /// thread, and any thread holding unsettled local rows (optimistic sends /
    /// pending acks that are not yet durable).
    func residencyPinnedThreadIds() -> Set<String> {
        var pinned: Set<String> = []
        if let selectedId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
           !selectedId.isEmpty {
            pinned.insert(selectedId)
        }
        if let streamId = streamOwnedThreadId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !streamId.isEmpty {
            pinned.insert(streamId)
        }
        for (threadId, messages) in messagesByThread
        where GaryxThreadResidencyPolicy.hasUnsettledLocalRows(messages) {
            pinned.insert(threadId)
        }
        return pinned
    }

    /// Drop every in-memory projection for an evicted thread together (they must
    /// go as a set or the signature-skip in `setMessages` would refuse to rebuild
    /// a partially-evicted thread). Re-opening re-derives from the disk cache +
    /// gateway, so no data is lost. The mirror clear goes straight through the
    /// store (bumps the generation) but must NOT re-touch residency.
    private func evictThreadProjections(_ threadId: String) {
        messagesByThread[threadId] = nil
        messageSignaturesByThread[threadId] = nil
        activeAssistantMessageIdsByThread[threadId] = nil
        renderSnapshotsByThread[threadId] = nil
        selectedThreadRenderFloorByThread[threadId] = nil
        transcriptMirror.set(nil, for: threadId)
        // Presentation-wise the thread is cold again: re-opening must take the
        // loading path, not flash the empty-conversation view.
        threadHistoryLoadedIds.remove(threadId)
    }
}
