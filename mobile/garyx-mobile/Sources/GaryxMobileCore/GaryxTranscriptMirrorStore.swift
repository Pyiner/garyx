import Foundation

/// In-memory mirror of the on-disk committed-transcript window per thread, with
/// a monotonic per-thread generation that bumps on **every** mutation — set and
/// clear alike (TASK-1751 P1).
///
/// The cold-open restore policy (`GaryxColdOpenRestorePolicy`) compares a
/// generation captured at restore spawn against the current one to decide
/// whether a decoded disk window is still fresh enough to apply/seed. Design
/// review v3 caught that a raw `[String: GaryxCachedTranscript]` dictionary lets
/// a write path (notably `clearTranscriptCache`, reachable mid-restore from
/// stream control-rewrite recovery) mutate the mirror *without* bumping the
/// generation, so a stale restore could resurrect a pre-clear window. Wrapping
/// the dict makes that bypass structurally impossible: there is no way to change
/// the mirror except through `set(_:for:)`, which always bumps.
struct GaryxTranscriptMirrorStore {
    private var snapshots: [String: GaryxCachedTranscript] = [:]
    private var generations: [String: UInt64] = [:]

    init() {}

    /// Set (non-nil) or clear (nil) the mirror for a thread; always bumps the
    /// thread's generation. Returns the new generation (the persistence-ordering
    /// callers ignore it; the restore policy reads it via `generation(for:)`).
    @discardableResult
    mutating func set(_ window: GaryxCachedTranscript?, for threadId: String) -> UInt64 {
        if let window {
            snapshots[threadId] = window
        } else {
            snapshots[threadId] = nil
        }
        let next = (generations[threadId] ?? 0) &+ 1
        generations[threadId] = next
        return next
    }

    /// Current mirror window for a thread, or nil when absent/cleared.
    func snapshot(for threadId: String) -> GaryxCachedTranscript? {
        snapshots[threadId]
    }

    /// Current mirror generation for a thread (0 before the first mutation).
    /// The restore policy captures this at spawn and compares at apply/seed.
    func generation(for threadId: String) -> UInt64 {
        generations[threadId] ?? 0
    }

    /// Whether the mirror currently holds a window for a thread.
    func contains(_ threadId: String) -> Bool {
        snapshots[threadId] != nil
    }

    /// Clear every thread's mirror (gateway/profile switch). Bumps each present
    /// thread's generation so an in-flight restore for any of them aborts, and
    /// keeps the generation counters (monotonic — never reset to 0, so a reused
    /// thread id cannot present a stale-but-equal generation later).
    mutating func clearAll() {
        for threadId in snapshots.keys {
            generations[threadId] = (generations[threadId] ?? 0) &+ 1
        }
        snapshots.removeAll()
    }
}
