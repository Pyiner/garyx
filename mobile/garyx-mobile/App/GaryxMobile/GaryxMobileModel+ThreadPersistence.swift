import Foundation
import WidgetKit

actor GaryxRecentThreadsWidgetPersistenceQueue {
    private let planner = GaryxRecentThreadsWidgetPersistencePlanner()
    private var latestGeneration: UInt64 = 0

    func persist(
        input: GaryxRecentThreadsWidgetSnapshotInput,
        generation: UInt64,
        avatarStore: any GaryxAvatarStore,
        validator: any GaryxAvatarImageValidating
    ) async {
        latestGeneration = max(latestGeneration, generation)
        guard generation == latestGeneration else { return }
        let upserts = GaryxAvatarWriteThroughPlan.candidates(
            scope: input.gatewayScopeId,
            agents: input.agents
        )
        if !upserts.isEmpty {
            await avatarStore.upsert(upserts, validator: validator, now: Date())
        }
        guard generation == latestGeneration else { return }
        let avatarIdentities = GaryxRecentThreadsWidgetSnapshotProjector.avatarIdentities(from: input)
        let avatarFallback = await avatarStore.avatarFingerprints(for: avatarIdentities, now: Date())
        guard generation == latestGeneration else { return }
        let widgetThreads = GaryxRecentThreadsWidgetSnapshotProjector.widgetThreads(
            from: input,
            avatarFallback: avatarFallback
        )
        guard generation == latestGeneration else { return }
        switch planner.nextWrite(for: widgetThreads) {
        case .skipUnchanged:
            return
        case .write(let threads):
            GaryxMobileWidgetStore.saveRecentThreads(threads)
            WidgetCenter.shared.reloadTimelines(ofKind: GaryxRecentThreadsWidgetConstants.kind)
        }
    }
}

// Pinned-thread state, local archive removal, and last-opened-thread /
// last-session restore persistence for the home thread list.
extension GaryxMobileModel {
    func isThreadPinned(_ threadId: String) -> Bool {
        pinnedThreadIds.contains(threadId.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    func togglePinnedThread(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        let pinned = !isThreadPinned(normalizedId)
        Task { await setThreadPinned(normalizedId, pinned: pinned) }
    }

    func unpinThread(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        Task { await setThreadPinned(normalizedId, pinned: false) }
    }

    func setThreadPinned(_ threadId: String, pinned: Bool) async {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        let previousIds = pinnedThreadIds
        pinnedThreadIds = Self.pinnedThreadIdsWith(
            pinnedThreadIds,
            threadId: normalizedId,
            pinned: pinned
        )
        recentThreadFeeds.noteLocalMutation()
        do {
            let page = try await client().setThreadPinned(threadId: normalizedId, pinned: pinned)
            applyPinnedThreadIds(page.threadIds)
            recentThreadFeeds.noteLocalMutation()
            persistRecentThreadsWidgetSnapshot()
        } catch {
            pinnedThreadIds = previousIds
            recentThreadFeeds.noteLocalMutation()
            persistRecentThreadsWidgetSnapshot()
            lastError = displayMessage(for: error)
        }
    }

    func applyPinnedThreadIds(_ ids: [String]) {
        let normalized = Self.normalizedPinnedThreadIds(ids)
        // The silent sidebar refresh loop calls this every few seconds; skip
        // the publish when nothing changed so observers do not re-render.
        if pinnedThreadIds != normalized {
            pinnedThreadIds = normalized
        }
    }

    func removePinnedThreadIdLocally(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        pinnedThreadIds.removeAll { $0 == normalizedId }
        recentThreadFeeds.noteLocalMutation()
    }

    @discardableResult
    func removeArchivedThreadLocally(_ threadId: String) -> GaryxRecentThreadRemovalRollback {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else {
            return GaryxRecentThreadRemovalRollback(
                threadId: "",
                allPosition: nil,
                nonTaskPosition: nil
            )
        }
        let transactionId = homeProjectionGateway.beginTransaction(label: "archive-local-remove")
        defer { homeProjectionGateway.endTransaction(transactionId) }
        pinnedThreadIds.removeAll { $0 == normalizedId }
        let rollback = recentThreadFeeds.removeThread(normalizedId)
        threads.removeAll { $0.id == normalizedId }
        // Any in-flight refresh captured this thread before the removal;
        // invalidate its commit so stale snapshots cannot resurrect the row
        // even after the archive tombstone resolves (review #TASK-1804).
        clearPersistedLastOpenedThreadId(ifMatches: normalizedId)
        persistRecentThreadsWidgetSnapshot()
        return rollback
    }

    // MARK: - Last opened thread restore

    /// Remembers the most recently opened thread per gateway scope so a fresh
    /// app launch can land back in it instead of the new-thread draft.
    func persistLastOpenedThreadId(_ threadId: String) {
        #if DEBUG
        if debugSnapshotActive { return }
        #endif
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        defaults.set(normalizedId, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId))
    }

    func clearPersistedLastOpenedThreadId(ifMatches threadId: String) {
        let key = scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId)
        guard defaults.string(forKey: key) == threadId else { return }
        defaults.removeObject(forKey: key)
    }

    func restorePersistedLastOpenedThreadId(_ threadId: String?) {
        let key = scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId)
        let normalizedId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if normalizedId.isEmpty {
            defaults.removeObject(forKey: key)
        } else {
            defaults.set(normalizedId, forKey: key)
        }
    }

    /// True when the app last went to background while showing a
    /// conversation; launches restore the thread only in that case.
    func persistLastSessionLocation() {
        #if DEBUG
        if debugSnapshotActive { return }
        #endif
        let onThread = GaryxLastOpenedThreadRestorationPolicy.isCurrentSessionRestorable(
            navigationState: navigationState,
            selectedThreadId: selectedThread?.id
        )
        defaults.set(onThread, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastSessionOnThread))
    }

    func persistLastSessionRestorable(_ restorable: Bool) {
        #if DEBUG
        if debugSnapshotActive { return }
        #endif
        defaults.set(restorable, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastSessionOnThread))
    }

    var persistedLastSessionWasOnThread: Bool {
        defaults.bool(forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastSessionOnThread))
    }

    var persistedLastOpenedThreadId: String? {
        let value = defaults.string(forKey: scopedSettingsKey(GaryxMobileSettingsKeys.lastOpenedThreadId))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    /// One-shot launch restore: when nothing else (deep link, widget link,
    /// pending route) claimed navigation, reopen the last opened thread
    /// through the shared open path.
    func restoreLastOpenedThreadIfNeeded() async {
        guard !hasAttemptedLastOpenedThreadRestore else { return }
        hasAttemptedLastOpenedThreadRestore = true
        #if DEBUG
        guard !debugSnapshotActive else { return }
        #endif
        guard let threadId = GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
            persistedLastOpenedThreadId: persistedLastOpenedThreadId,
            persistedLastSessionWasOnThread: persistedLastSessionWasOnThread,
            selectedThreadId: selectedThread?.id,
            hasPendingMobileRoute: pendingMobileRoute != nil,
            hasPendingThreadIntent: threadOpenState.hasPendingIntent,
            navigationState: navigationState,
            sidebarVisible: sidebarVisible
        ) else {
            return
        }
        await restoreLastOpenedThread(id: threadId)
    }

    static func pinnedThreadIdsWith(
        _ ids: [String],
        threadId: String,
        pinned: Bool
    ) -> [String] {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return normalizedPinnedThreadIds(ids) }
        let remaining = normalizedPinnedThreadIds(ids).filter { $0 != normalizedId }
        return pinned ? [normalizedId] + remaining : remaining
    }

    static func normalizedPinnedThreadIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []
        for rawId in ids {
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            normalized.append(id)
        }
        return normalized
    }
}
