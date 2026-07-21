import Foundation

/// Pure state machine for the remote directory browser (add-workspace flow
/// and directory picker embedders). Owns navigation, the local filter, and
/// the typed-error stay-put rule: a failed navigation renders inline and the
/// browser stays exactly where it was.
public struct GaryxWorkspaceDirectoryBrowserState: Equatable, Sendable {
    public enum InlineError: Equatable, Sendable {
        case typed(GaryxWorkspaceDirectoryError)
        case transport(String)

        public var message: String {
            switch self {
            case .typed(let error): return error.code.userMessage
            case .transport(let message): return message
            }
        }
    }

    /// The listing currently on screen; nil until the first load lands.
    public private(set) var listing: GaryxWorkspaceDirectoryListing?
    public private(set) var isLoading = false
    public private(set) var inlineError: InlineError?
    public var filterText = ""

    public init() {}

    public var currentPath: String? { listing?.path }
    public var parentPath: String? { listing?.parentPath }

    public var filteredEntries: [GaryxWorkspaceDirectoryEntry] {
        let needle = filterText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let entries = listing?.entries else { return [] }
        guard !needle.isEmpty else { return entries }
        return entries.filter { $0.name.localizedCaseInsensitiveContains(needle) }
    }

    /// Breadcrumb segments for the current path: root first, each segment
    /// carrying the absolute path it jumps to.
    public var pathSegments: [PathSegment] {
        guard let path = currentPath, path.hasPrefix("/") else { return [] }
        var segments: [PathSegment] = [PathSegment(label: "/", path: "/")]
        var accumulated = ""
        for component in path.split(separator: "/") {
            accumulated += "/" + component
            segments.append(PathSegment(label: String(component), path: accumulated))
        }
        return segments
    }

    public struct PathSegment: Equatable, Sendable, Identifiable {
        public var id: String { path }
        public var label: String
        public var path: String

        public init(label: String, path: String) {
            self.label = label
            self.path = path
        }
    }

    // MARK: - Transitions

    public mutating func beginLoad() {
        isLoading = true
        inlineError = nil
    }

    /// A landed listing replaces the view and resets the local filter — the
    /// filter narrows one directory, not a navigation session.
    public mutating func apply(_ listing: GaryxWorkspaceDirectoryListing) {
        self.listing = listing
        isLoading = false
        inlineError = nil
        filterText = ""
    }

    /// Any failure renders inline and keeps the previous listing in place.
    public mutating func fail(_ error: Error) {
        isLoading = false
        if let typed = error as? GaryxWorkspaceDirectoryError {
            inlineError = .typed(typed)
        } else {
            inlineError = .transport(error.localizedDescription)
        }
    }

    /// Normalizes a typed/pasted path-bar submission. Empty input is a no-op
    /// (nil); anything else must be absolute — a relative path short-circuits
    /// to the same inline error the server would return, without a round
    /// trip.
    public mutating func normalizeTypedPath(_ raw: String) -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        guard trimmed.hasPrefix("/") else {
            inlineError = .typed(
                GaryxWorkspaceDirectoryError(
                    code: .invalidPath,
                    message: GaryxWorkspaceDirectoryErrorCode.invalidPath.userMessage
                )
            )
            return nil
        }
        if trimmed.count > 1, trimmed.hasSuffix("/") {
            return String(trimmed.dropLast())
        }
        return trimmed
    }
}
