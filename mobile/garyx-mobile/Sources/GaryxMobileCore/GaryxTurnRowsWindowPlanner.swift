import Foundation

/// Floor-anchored render window over prepared turn rows (TASK-1751 P3).
///
/// The transcript renders in an eager `VStack` (LazyVStack breaks the tail
/// anchor — see the view comment), so instantiating every cached history row on
/// open is the cost. This window bounds that: it renders only the rows from an
/// **anchored floor** (the stable id of the oldest visible row) to the tail.
///
/// The floor is an *absolute row identity*, not a count from the end. That is
/// the crucial property: when the stream appends rows at the tail, the floor id
/// still resolves to the same row, so the window's hidden-head set is unchanged
/// — the window grows only at the bottom, never removing a row from the top.
/// This is what makes streaming safe while the reader is browsing off-tail (a
/// sliding `suffix(limit)` would delete the oldest rendered row on every append
/// and shift content under the viewport — a scroll-jump regression).
///
/// The floor only ever moves *up* (older) on an explicit `expand`; it is never
/// pushed down by `resolve`. So within a thread session the visible set is
/// monotonically non-shrinking. It re-anchors to the tail only when its row was
/// dropped from the list entirely (a windowed-resume reset, which already
/// reflows the transcript and resets scroll). Thread switch resets the state.
struct GaryxTurnRowsWindowState: Equatable {
    /// Stable id of the oldest currently-visible row. `nil` = uninitialized
    /// (a fresh thread open), which resolves to the newest `initialLimit` rows.
    var floorRowId: String?

    init(floorRowId: String? = nil) {
        self.floorRowId = floorRowId
    }
}

enum GaryxTurnRowsWindowPlanner {
    /// Rows shown on a fresh open before any expansion.
    static let initialLimit = 60
    /// Rows revealed per expansion step (scroll-up boundary / Load-earlier tap).
    static let expandStep = 60

    /// The floor index for a set of rows and window state: the anchored row's
    /// index when it is still present, else the newest-`initialLimit` fallback.
    /// Pure; shared by `resolve`/`expand`/`isWindowExhausted` so they agree.
    private static func floorIndex(
        rows: [GaryxMobileTurnRow],
        state: GaryxTurnRowsWindowState,
        limit: Int = initialLimit
    ) -> Int {
        guard !rows.isEmpty else { return 0 }
        if let floorRowId = state.floorRowId,
           let index = rows.firstIndex(where: { $0.id == floorRowId }) {
            return index
        }
        return max(0, rows.count - limit)
    }

    /// The visible tail window and the resolved state (with the floor row id
    /// written back). A plain read with no floor change returns an equal state,
    /// so it never thrashes `@Published`.
    static func resolve(
        rows: [GaryxMobileTurnRow],
        state: GaryxTurnRowsWindowState,
        limit: Int = initialLimit
    ) -> (visible: [GaryxMobileTurnRow], state: GaryxTurnRowsWindowState) {
        guard !rows.isEmpty else {
            return ([], GaryxTurnRowsWindowState(floorRowId: nil))
        }
        let index = floorIndex(rows: rows, state: state, limit: limit)
        let visible = Array(rows[index...])
        return (visible, GaryxTurnRowsWindowState(floorRowId: rows[index].id))
    }

    /// Lower the floor by `expandStep` (reveal older rows), clamped at the
    /// list start. The floor never rises here, so the window only grows.
    static func expand(
        rows: [GaryxMobileTurnRow],
        state: GaryxTurnRowsWindowState,
        step: Int = expandStep,
        limit: Int = initialLimit
    ) -> GaryxTurnRowsWindowState {
        guard !rows.isEmpty else { return GaryxTurnRowsWindowState(floorRowId: nil) }
        let current = floorIndex(rows: rows, state: state, limit: limit)
        let next = max(0, current - max(0, step))
        return GaryxTurnRowsWindowState(floorRowId: rows[next].id)
    }

    /// True when the window already shows every in-memory row (floor at the
    /// list start). Gates network history paging and the Load-earlier button.
    static func isWindowExhausted(
        rows: [GaryxMobileTurnRow],
        state: GaryxTurnRowsWindowState,
        limit: Int = initialLimit
    ) -> Bool {
        guard !rows.isEmpty else { return true }
        return floorIndex(rows: rows, state: state, limit: limit) == 0
    }
}
