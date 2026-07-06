import Foundation

/// Memoizes the prepared turn rows produced by `GaryxMobileRenderStateMapper`
/// (TASK-1751 P2).
///
/// The conversation view calls `selectedThreadTurnRows()` at least twice per
/// SwiftUI body evaluation (body + `.onChange(of:)`), and the mapper rebuilds
/// its entire `MessageLookup` (re-mapping every transcript message) on every
/// call — 55–137ms for a 700-turn thread in the repro. This cache stores the
/// last inputs and prepared output and returns the cached rows when all inputs
/// compare equal, so an unchanged body evaluation performs **zero** mapper work
/// and a real input change performs exactly one rebuild.
///
/// It is a plain (non-published) cache held on the main actor: the mapper stays
/// a dumb, pure mapping of its inputs — no derivation logic lives here. The key
/// is every input the mapper reads, so any change that could alter output
/// invalidates.
struct GaryxTurnRowsCache {
    private struct Key: Equatable {
        var threadId: String?
        var snapshot: GaryxRenderSnapshot?
        var messages: [GaryxMobileMessage]
        var transcriptMessages: [GaryxTranscriptMessage]
    }

    private var key: Key?
    private var cachedRows: [GaryxMobileTurnRow] = []
    private var cachedRowIds: [String] = []

    init() {}

    /// Return the prepared turn rows for these inputs, rebuilding through
    /// `build` only when any input changed since the last call. `build` must be
    /// a pure function of the same inputs (the render-state mapper is).
    mutating func rows(
        threadId: String?,
        snapshot: GaryxRenderSnapshot?,
        messages: [GaryxMobileMessage],
        transcriptMessages: [GaryxTranscriptMessage],
        build: () -> [GaryxMobileTurnRow]
    ) -> [GaryxMobileTurnRow] {
        let nextKey = Key(
            threadId: threadId,
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: transcriptMessages
        )
        if key == nextKey {
            return cachedRows
        }
        let built = build()
        key = nextKey
        cachedRows = built
        cachedRowIds = built.map(\.id)
        return built
    }

    /// The cached rows' ids, without triggering a rebuild — the `.onChange`
    /// observer only needs the id list, and reusing the cached ids means a body
    /// evaluation that already resolved `rows(...)` costs nothing more.
    /// Returns `nil` when nothing is cached yet (caller falls back to `rows`).
    var cachedIds: [String]? {
        key == nil ? nil : cachedRowIds
    }

    /// Drop the memo (thread switch / gateway reset). The next `rows(...)` call
    /// rebuilds.
    mutating func invalidate() {
        key = nil
        cachedRows = []
        cachedRowIds = []
    }
}
