import Foundation

/// The new-thread draft workspace selection is an explicit tri-state, never a
/// bare string: `path` is a chosen workspace, `none` is the explicit
/// "No workspace" choice, and `unresolved` means the draft has not resolved a
/// default yet. Resolution happens exactly once (`resolved(against:)`); after
/// that the selection never drifts — a resolved choice survives catalog
/// refreshes and membership changes alike.
public enum GaryxDraftWorkspaceSelection: Equatable, Sendable {
    case unresolved
    case none
    case path(String)

    public var workspacePath: String? {
        if case .path(let value) = self { return value }
        return nil
    }

    public var isResolved: Bool {
        if case .unresolved = self { return false }
        return true
    }

    /// The `workspace_dir` create-payload value: a concrete path for `.path`,
    /// absent for `.none`. An unresolved draft also sends nothing — sending
    /// before resolution is equivalent to declining to choose.
    public var createPayloadWorkspaceDir: String? {
        workspacePath
    }

    /// The explicit No-workspace choice needs its own wire bit: an absent
    /// `workspace_dir` alone lets the agent default substitute, while
    /// `noWorkspace=true` makes the gateway provision the private managed
    /// thread workspace.
    public var isExplicitNoWorkspace: Bool {
        if case .none = self { return true }
        return false
    }

    /// Resolves this selection against the server-ordered catalog.
    /// - `.unresolved` resolves once to the first catalog row (the list
    ///   arrives pre-sorted, so a pinned row wins), or `.none` when the
    ///   catalog is empty. A selection made before the catalog loads is
    ///   final; this only fills the default in.
    /// - `.none` is explicit and never overridden.
    /// - `.path` is never auto-replaced. Catalog membership is not a
    ///   validity test for an explicit path — agent default directories
    ///   and freshly removed workspaces are legitimate selections that the
    ///   picker presents as the "Current" row.
    public func resolved(against catalog: GaryxWorkspaceCatalog, catalogLoaded: Bool) -> GaryxDraftWorkspaceSelection {
        switch self {
        case .unresolved:
            guard catalogLoaded else { return .unresolved }
            if let first = catalog.workspaces.first {
                return .path(first.path)
            }
            return .none
        case .none:
            return .none
        case .path:
            return self
        }
    }

    // MARK: - Persistence encoding

    /// Stable persisted encoding for the gateway-scoped draft state.
    /// `""` is not a valid encoding: absence of a stored value means
    /// unresolved, `"none"` is the explicit No-workspace choice, and
    /// `"path:<absolute path>"` is a chosen workspace.
    public var persistedValue: String? {
        switch self {
        case .unresolved: return nil
        case .none: return "none"
        case .path(let value): return "path:" + value
        }
    }

    public static func fromPersistedValue(_ raw: String?) -> GaryxDraftWorkspaceSelection {
        guard let raw, !raw.isEmpty else { return .unresolved }
        if raw == "none" { return .none }
        if raw.hasPrefix("path:") {
            let path = String(raw.dropFirst("path:".count))
            let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? .unresolved : .path(trimmed)
        }
        return .unresolved
    }
}
