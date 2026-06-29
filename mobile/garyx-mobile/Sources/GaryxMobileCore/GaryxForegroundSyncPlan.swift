import Foundation

/// What the app must do when it returns to the foreground so the currently-open
/// thread converges to the server's latest state (#TASK-1449 symptom 2).
///
/// iOS suspends a backgrounded app and tears down the per-thread SSE stream, so
/// the only way to stay correct is to re-sync on return. The previous logic
/// gated the whole resync on a cached `connectionState == .ready` and did
/// nothing otherwise — so a connection that dropped/changed while backgrounded
/// was never recovered, and the open thread stayed stale until a manual
/// re-open. This planner makes the decision explicit and testable: a selected
/// thread always resyncs + restarts its stream, and a non-ready connection
/// reconnects first.
public struct GaryxForegroundSyncPlan: Equatable, Sendable {
    /// Re-establish the gateway connection before anything else (it
    /// dropped/changed while backgrounded).
    public var reconnect: Bool
    /// Re-fetch the open thread's committed history so it matches the server.
    public var resyncOpenThread: Bool
    /// Re-establish the open thread's resumable per-thread SSE stream (it was
    /// stopped on background).
    public var restartStream: Bool

    public init(reconnect: Bool, resyncOpenThread: Bool, restartStream: Bool) {
        self.reconnect = reconnect
        self.resyncOpenThread = resyncOpenThread
        self.restartStream = restartStream
    }

    public static func plan(
        connectionState: GaryxMobileConnectionState,
        selectedThreadId: String?
    ) -> Self {
        let hasThread = (selectedThreadId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false)
        switch connectionState {
        case .ready:
            // Connected: resync + restart the open thread's stream (it was stopped
            // on background). No reconnect needed.
            return Self(reconnect: false, resyncOpenThread: hasThread, restartStream: hasThread)
        case .checking:
            // A connect is already in flight; don't kick a second one. Its
            // completion path handles routing/refresh.
            return Self(reconnect: false, resyncOpenThread: false, restartStream: false)
        case .disconnected, .failed:
            // The connection dropped/changed while backgrounded: reconnect first so
            // the open thread can converge to the server's latest state on return,
            // instead of staying frozen until a manual re-open.
            return Self(reconnect: true, resyncOpenThread: hasThread, restartStream: hasThread)
        }
    }
}
