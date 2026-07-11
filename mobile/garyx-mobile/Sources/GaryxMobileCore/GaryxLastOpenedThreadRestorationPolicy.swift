import Foundation

enum GaryxLastOpenedThreadRestorationPolicy {
    static func isCurrentSessionRestorable(
        navigationState: GaryxMobileNavigationState,
        selectedThreadId: String?
    ) -> Bool {
        navigationState.presentsContent
            && navigationState.activePanel == .chat
            && normalizedId(selectedThreadId) != nil
    }

    static func restoreThreadId(
        persistedLastOpenedThreadId: String?,
        persistedLastSessionWasOnThread: Bool,
        selectedThreadId: String?,
        hasPendingMobileRoute: Bool,
        hasPendingThreadIntent: Bool,
        navigationState: GaryxMobileNavigationState,
        sidebarVisible: Bool
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
        return persistedThreadId
    }

    private static func normalizedId(_ value: String?) -> String? {
        let normalized = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return normalized.isEmpty ? nil : normalized
    }
}
