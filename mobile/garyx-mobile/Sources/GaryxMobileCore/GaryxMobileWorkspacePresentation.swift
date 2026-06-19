import Foundation

enum GaryxMobileWorkspacePresentation {
    static func userWorkspacePaths(
        savedWorkspacePaths: [String],
        additionalPaths: [String] = []
    ) -> [String] {
        uniqueSortedWorkspacePaths(savedWorkspacePaths + additionalPaths, filtersDynamicPaths: false)
    }

    static func workspacePathSuggestions(
        threadWorkspacePaths: [String?],
        threadWorktreePaths: [String?],
        automationWorkspacePaths: [String],
        savedWorkspacePaths: [String],
        additionalPaths: [String]
    ) -> [String] {
        let worktreePaths = Set(threadWorktreePaths.compactMap { $0 }.map(normalizedWorkspacePathKey))
        let values = savedWorkspacePaths
            + threadWorkspacePaths.compactMap { $0 }
            + automationWorkspacePaths
            + additionalPaths
        return uniqueSortedWorkspacePaths(values, filtersDynamicPaths: true)
            .filter { !worktreePaths.contains(normalizedWorkspacePathKey($0)) }
    }

    private static func uniqueSortedWorkspacePaths(
        _ values: [String],
        filtersDynamicPaths: Bool
    ) -> [String] {
        var seen = Set<String>()
        return values
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .filter { filtersDynamicPaths ? isVisibleWorkspacePath($0) : true }
            .filter { seen.insert($0).inserted }
            .sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }

    static func isVisibleWorkspacePath(_ path: String) -> Bool {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        let normalized = trimmed.replacingOccurrences(of: "\\", with: "/")
        if normalized.contains("/.garyx/worktrees/") || normalized.contains("/.codex/worktrees/") {
            return false
        }
        if normalized == "/tmp" || normalized == "/private/tmp" {
            return false
        }
        let pathComponents = normalized
            .split(separator: "/", omittingEmptySubsequences: true)
            .map(String.init)
        if pathComponents.contains(where: { $0.hasPrefix("garyx-agent-loop-smoke.") }) {
            return false
        }
        return true
    }

    private static func normalizedWorkspacePathKey(_ path: String) -> String {
        path.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\\", with: "/")
    }
}
