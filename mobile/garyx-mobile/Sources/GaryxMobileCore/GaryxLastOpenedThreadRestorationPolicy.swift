import Foundation

enum GaryxLastOpenedThreadRestorationPolicy {
    static func persistedThreadId(
        afterOpening destination: GaryxWorkflowRunDestination,
        previousThreadId: String?
    ) -> String? {
        switch destination {
        case .chat(let threadId):
            normalizedId(threadId) ?? normalizedId(previousThreadId)
        case .workflowRun, .unresolved:
            normalizedId(previousThreadId)
        }
    }

    static func isSessionRestorableAfterOpening(_ destination: GaryxWorkflowRunDestination) -> Bool {
        switch destination {
        case .chat(let threadId):
            return normalizedId(threadId) != nil
        case .workflowRun, .unresolved:
            return false
        }
    }

    static func isCurrentSessionRestorable(
        navigationState: GaryxMobileNavigationState,
        selectedThreadId: String?,
        activeWorkflowRunId: String?
    ) -> Bool {
        navigationState.presentsContent
            && navigationState.activePanel == .chat
            && normalizedId(selectedThreadId) != nil
            && normalizedId(activeWorkflowRunId) == nil
    }

    static func restoreThreadId(
        persistedLastOpenedThreadId: String?,
        persistedLastSessionWasOnThread: Bool,
        selectedThreadId: String?,
        hasPendingMobileRoute: Bool,
        hasPendingThreadIntent: Bool,
        navigationState: GaryxMobileNavigationState,
        sidebarVisible: Bool,
        resolvedDestination: GaryxWorkflowRunDestination? = nil
    ) -> String? {
        guard normalizedId(selectedThreadId) == nil,
              !hasPendingMobileRoute,
              !hasPendingThreadIntent,
              navigationState.activePanel == .chat,
              !sidebarVisible,
              persistedLastSessionWasOnThread,
              let persistedThreadId = normalizedId(persistedLastOpenedThreadId) else {
            return nil
        }
        if let resolvedDestination {
            switch resolvedDestination {
            case .chat(let threadId):
                return normalizedId(threadId)
            case .workflowRun, .unresolved:
                return nil
            }
        }
        return persistedThreadId
    }

    private static func normalizedId(_ value: String?) -> String? {
        let normalized = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return normalized.isEmpty ? nil : normalized
    }
}
