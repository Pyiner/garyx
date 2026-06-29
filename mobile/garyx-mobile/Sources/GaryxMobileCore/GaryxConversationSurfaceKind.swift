import Foundation

/// Which surface the conversation route should present, as a pure function of
/// the open thread's **objective type** — never of the entry path or the
/// previously-viewed thread (#TASK-1449 symptom 1).
///
/// `.workflowRun` is reserved for a thread the server objectively marks
/// `thread_type == "workflow_run"`. An as-yet-unclassified by-id open (no
/// summary loaded) is `.loadingUnknown`, which renders the **chat** surface in a
/// loading state — it must never present the workflow surface. Everything else
/// is `.chat`.
public enum GaryxConversationSurfaceKind: Equatable, Sendable {
    case chat
    case workflowRun(runId: String)
    case loadingUnknown

    /// Whether this kind presents the workflow-run surface. Only a confirmed
    /// workflow-run thread does; chat and not-yet-resolved opens do not.
    public var presentsWorkflowRun: Bool {
        if case .workflowRun = self { return true }
        return false
    }

    /// Resolve the surface kind from the open thread's summary.
    ///
    /// - `summary == nil`: a by-id open whose summary has not loaded yet is
    ///   `.loadingUnknown` (chat surface, loading); a plain draft (not resolving
    ///   a specific id) is `.chat` (composer).
    /// - `summary` present: classified purely by `thread_type` via
    ///   `GaryxWorkflowRunDestination`. A `thread_type` the gateway hasn't
    ///   stamped yet (`.unresolved`) is treated as chat-loading, never workflow.
    public static func resolve(summary: GaryxThreadSummary?, isResolvingById: Bool) -> Self {
        guard let summary else {
            return isResolvingById ? .loadingUnknown : .chat
        }
        switch GaryxWorkflowRunDestination.destination(for: summary) {
        case .workflowRun(let runId):
            return .workflowRun(runId: runId)
        case .chat:
            return .chat
        case .unresolved:
            return isResolvingById ? .loadingUnknown : .chat
        }
    }
}
