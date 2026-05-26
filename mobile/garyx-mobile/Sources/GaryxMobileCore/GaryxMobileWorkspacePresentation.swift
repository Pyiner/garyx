import Foundation

enum GaryxMobileWorkspacePresentation {
    static func knownWorkspacePaths(
        threadWorkspacePaths: [String?],
        threadWorktreePaths: [String?],
        automationWorkspacePaths: [String],
        autoResearchWorkspaceDirs: [String?],
        additionalPaths: [String]
    ) -> [String] {
        var seen = Set<String>()
        let worktreePaths = Set(threadWorktreePaths.compactMap { $0 }.map(normalizedWorkspacePathKey))
        let values = threadWorkspacePaths.compactMap { $0 }
            + automationWorkspacePaths
            + autoResearchWorkspaceDirs.compactMap { $0 }
            + additionalPaths
        return values
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .filter(isVisibleWorkspacePath)
            .filter { !worktreePaths.contains(normalizedWorkspacePathKey($0)) }
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
        return true
    }

    private static func normalizedWorkspacePathKey(_ path: String) -> String {
        path.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\\", with: "/")
    }
}
