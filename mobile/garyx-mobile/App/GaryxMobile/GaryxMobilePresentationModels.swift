import Foundation

struct GaryxSidebarThreadRowPresentation: Equatable {
    let title: String
    let subtitle: String?
    let trailingTimestamp: String?
    let isSelected: Bool
    let isPinned: Bool
    let isRunning: Bool

    init(
        thread: GaryxThreadSummary,
        isSelected: Bool,
        isPinned: Bool,
        trailingTimestamp: String?
    ) {
        self.title = thread.title.isEmpty ? "Untitled" : thread.title
        self.subtitle = Self.subtitle(for: thread)
        self.trailingTimestamp = trailingTimestamp
        self.isSelected = isSelected
        self.isPinned = isPinned
        self.isRunning = Self.isRunning(thread)
    }

    private static func subtitle(for thread: GaryxThreadSummary) -> String? {
        if let workspacePath = thread.workspacePath, !workspacePath.isEmpty {
            return workspacePath.garyxLastPathComponent
        }
        if let teamName = thread.teamName, !teamName.isEmpty {
            return teamName
        }
        return thread.agentId
    }

    private static func isRunning(_ thread: GaryxThreadSummary) -> Bool {
        let state = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let activeRunId = thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if state == "running" {
            return true
        }
        return !activeRunId.isEmpty
    }
}
