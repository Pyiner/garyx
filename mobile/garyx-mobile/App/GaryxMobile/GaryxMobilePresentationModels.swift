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

struct GaryxAutomationDraft: Equatable {
    var label = ""
    var prompt = ""
    var intervalHours = "24"
    var targetsExistingThread = false
    var targetThreadId = ""
    var workspacePath = ""

    var trimmedLabel: String {
        label.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var trimmedPrompt: String {
        prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var trimmedWorkspacePath: String {
        workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var trimmedTargetThreadId: String {
        targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var parsedIntervalHours: Int? {
        let trimmed = intervalHours.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
    }

    func canSubmit(workspacePaths: [String], threadOptions: [GaryxThreadSummary]) -> Bool {
        guard !trimmedLabel.isEmpty,
              !trimmedPrompt.isEmpty,
              parsedIntervalHours != nil else {
            return false
        }
        if targetsExistingThread {
            return !effectiveThreadId(threadOptions: threadOptions).isEmpty
        }
        return !effectiveWorkspacePath(workspacePaths: workspacePaths).isEmpty
    }

    func effectiveWorkspacePath(workspacePaths: [String]) -> String {
        if !trimmedWorkspacePath.isEmpty, workspacePaths.contains(trimmedWorkspacePath) {
            return trimmedWorkspacePath
        }
        return workspacePaths.first ?? ""
    }

    func effectiveThreadId(threadOptions: [GaryxThreadSummary]) -> String {
        if !trimmedTargetThreadId.isEmpty, threadOptions.contains(where: { $0.id == trimmedTargetThreadId }) {
            return trimmedTargetThreadId
        }
        return threadOptions.first?.id ?? ""
    }

    mutating func ensureTargetSelection(workspacePaths: [String], threadOptions: [GaryxThreadSummary]) {
        if targetsExistingThread {
            let nextThreadId = effectiveThreadId(threadOptions: threadOptions)
            targetThreadId = nextThreadId
            if let workspacePath = threadOptions.first(where: { $0.id == nextThreadId })?.workspacePath?
                .trimmingCharacters(in: .whitespacesAndNewlines),
               !workspacePath.isEmpty {
                self.workspacePath = workspacePath
            }
        } else {
            workspacePath = effectiveWorkspacePath(workspacePaths: workspacePaths)
        }
    }
}
