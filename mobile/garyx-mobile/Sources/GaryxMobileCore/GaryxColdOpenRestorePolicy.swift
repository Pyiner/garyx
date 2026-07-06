import Foundation

/// Decides whether an async cold-open transcript restore may apply its decoded
/// result (TASK-1751 P1).
///
/// Opening a thread no longer decodes the persisted cache synchronously on the
/// main actor. Instead a background task loads + decodes + maps, then asks this
/// policy — on the main actor — whether its (now possibly stale) output may be
/// applied. `cachedMessages.isEmpty` alone is NOT a sufficient freshness marker
/// (design review v1 finding 2): a history fetch can legitimately complete with
/// an *empty* transcript (it still marks history loaded), and a stream frame can
/// apply a newer render snapshot before the throttled message flush populates
/// `messages`. So the policy aborts if any newer content path has run.
enum GaryxColdOpenRestorePolicy {
    /// Inputs captured at apply time (main actor). All must clear for the
    /// restore to apply; any newer path (thread switch, history apply, stream
    /// snapshot, messages present) suppresses it.
    struct State: Equatable {
        /// The thread the restore task decoded for.
        var restoredThreadId: String
        /// Currently-selected thread id.
        var selectedThreadId: String?
        /// Cold-open generation captured when the task spawned.
        var capturedGeneration: UInt64
        /// Current cold-open generation (bumped on every thread switch).
        var currentGeneration: UInt64
        /// Whether the network history apply has already run for this thread
        /// (`markThreadHistoryLoaded`, called even for an empty transcript).
        var threadHistoryLoaded: Bool
        /// Whether a live/stream render snapshot has been applied for this
        /// thread.
        var hasRenderSnapshot: Bool
        /// Whether the local message store already holds rows for this thread.
        var hasMessages: Bool
    }

    /// Apply the restored output only when the thread is unchanged, no
    /// thread-switch churn occurred, and no newer content path has populated
    /// history / render snapshot / messages.
    static func shouldApply(_ state: State) -> Bool {
        state.selectedThreadId == state.restoredThreadId
            && state.capturedGeneration == state.currentGeneration
            && !state.threadHistoryLoaded
            && !state.hasRenderSnapshot
            && !state.hasMessages
    }

    /// Whether the restore task may seed the in-memory cache mirror
    /// (`cachedTranscriptSnapshots`) with the decoded window. Looser than
    /// `shouldApply` — it does not require empty messages / unloaded history,
    /// because seeding the mirror only advances the forward cursor and never
    /// changes visible rows — but it must still not clobber a fresher live
    /// window: gated on same thread, no switch churn, and no live render
    /// snapshot already present.
    static func shouldSeedMirror(_ state: State) -> Bool {
        state.selectedThreadId == state.restoredThreadId
            && state.capturedGeneration == state.currentGeneration
            && !state.hasRenderSnapshot
    }
}
