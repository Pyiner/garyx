import Foundation
import SwiftUI
import UIKit

// Conversation task-tree sidebar state: anchored forest fetch/cache with
// stale-response gating, open/close, row navigation, and the poll policy.
// All mapping decisions live in GaryxTaskTreeSidebarPresentation (Core).
extension GaryxMobileModel {
    var taskTreeSidebarRows: [GaryxTaskTreeRow] {
        guard let page = taskTreeForestPage else { return [] }
        return GaryxTaskTreeSidebarPresentation.rows(
            page: page,
            currentThreadId: selectedThread?.id
        )
    }

    var taskTreeActiveBadgeCount: Int {
        guard let page = taskTreeForestPage else { return 0 }
        return GaryxTaskTreeSidebarPresentation.activeBadgeCount(page: page)
    }

    /// Entry points (header button, edge gesture) exist only when the tree is
    /// known non-empty, matching the Mac popover's hidden-when-empty rule.
    var isTaskTreeSidebarAvailable: Bool {
        guard let page = taskTreeForestPage else { return false }
        return GaryxTaskTreeSidebarPresentation.isSidebarAvailable(page: page)
    }

    /// While the very first fetch for a thread is in flight the edge gesture
    /// may still open the panel onto a loading state.
    var isTaskTreeFirstLoadInFlight: Bool {
        taskTreeForestPage == nil && taskTreeLoadPhase.isLoading
    }

    /// Re-anchor the sidebar to the selected thread: restore the cached tree
    /// snapshot (anchor → tree index → per-tree cache) for instant rendering
    /// and close the panel when the conversation is left entirely. The
    /// restored snapshot is stale-while-revalidate: the caller always follows
    /// up with `refreshSelectedThreadTaskForest()`.
    func syncTaskTreeSidebarAnchor() {
        let anchor = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !anchor.isEmpty else {
            taskTreeForestPage = nil
            taskTreeLoadPhase = .idle
            taskTreeRevealInteraction.invalidate(
                position: .closed,
                event: .routeInvalidated
            )
            isTaskTreeSidebarOpen = false
            return
        }
        let cached = taskTreeOriginKeyByAnchor[taskTreeScopedCacheKey(anchor)]
            .flatMap { taskTreeSnapshotsByOrigin[$0] }
        taskTreeForestPage = cached
        taskTreeLoadPhase = cached == nil ? .idle : .loaded
    }

    func refreshSelectedThreadTaskForest() async {
        guard hasGatewaySettings,
              let anchor = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !anchor.isEmpty else {
            return
        }
        let token = taskTreeRequestGate.begin(
            gatewayKey: activeGatewayScopeId,
            anchorThreadId: anchor
        )
        if taskTreeForestPage == nil {
            taskTreeLoadPhase = .loading
        }
        do {
            let page = try await client().listTaskForest(anchorThreadId: anchor)
            guard taskTreeRequestGate.accepts(
                token: token,
                gatewayKey: activeGatewayScopeId,
                anchorThreadId: anchor
            ), selectedThread?.id == anchor else {
                return
            }
            taskTreeForestPage = page
            storeTaskTreeSnapshot(page, anchor: anchor)
            taskTreeLoadPhase = .loaded
        } catch {
            guard taskTreeRequestGate.accepts(
                token: token,
                gatewayKey: activeGatewayScopeId,
                anchorThreadId: anchor
            ), selectedThread?.id == anchor else {
                return
            }
            // Transient errors keep the previous snapshot and retry silently
            // on the next poll tick (desktop popover parity).
            if taskTreeForestPage == nil {
                taskTreeLoadPhase = .failed(displayMessage(for: error))
            }
        }
    }

    func openTaskTreeSidebar() {
        guard !isTaskTreeSidebarOpen else { return }
        dismissKeyboardForTaskTreeSidebar()
        taskTreeRevealInteraction.setTarget(.open, animated: true)
        isTaskTreeSidebarOpen = true
        GaryxMobileHaptics.shared.play(.taskTreeVisibilityCommitted)
        Task { [weak self] in
            await self?.refreshSelectedThreadTaskForest()
        }
    }

    func closeTaskTreeSidebar() {
        guard isTaskTreeSidebarOpen else { return }
        taskTreeRevealInteraction.setTarget(.closed, animated: true)
        isTaskTreeSidebarOpen = false
        GaryxMobileHaptics.shared.play(.taskTreeVisibilityCommitted)
    }

    /// Row tap: the current thread's row only closes the panel; any other row
    /// closes and routes through the shared `openThread` path. The tapped row
    /// belongs to the tree on screen, so the anchor→tree index is pre-seeded
    /// and the target conversation renders this snapshot instantly.
    func handleTaskTreeRowTap(_ row: GaryxTaskTreeRow) async {
        let currentThreadId = selectedThread?.id
        if let page = taskTreeForestPage {
            let originKey = taskTreeScopedCacheKey(
                GaryxTaskTreeSidebarPresentation.treeCacheKey(
                    page: page,
                    anchorThreadId: currentThreadId ?? row.threadId
                ))
            taskTreeOriginKeyByAnchor[taskTreeScopedCacheKey(row.threadId)] = originKey
        }
        closeTaskTreeSidebar()
        guard GaryxTaskTreeSidebarPresentation.shouldNavigate(
            currentThreadId: currentThreadId,
            targetThreadId: row.threadId
        ) else {
            return
        }
        await openThread(id: row.threadId, source: .replace)
    }

    /// Drops every cached tree snapshot (any tree may have changed; the
    /// anchor→tree index stays, a wrong entry only costs one refetch) and
    /// refreshes the open conversation's snapshot so the badge and sidebar
    /// catch the change immediately.
    func noteTaskTreeLocalMutation() {
        taskTreeSnapshotsByOrigin.removeAll()
        taskTreeSnapshotOriginOrder.removeAll()
        guard selectedThread != nil else { return }
        Task { [weak self] in
            await self?.refreshSelectedThreadTaskForest()
        }
    }

    /// Snapshot keys compose the gateway scope so switching gateways can
    /// never resurface another gateway's trees.
    private func taskTreeScopedCacheKey(_ raw: String) -> String {
        "\(activeGatewayScopeId)|\(raw)"
    }

    private static let taskTreeSnapshotCap = 16

    private func storeTaskTreeSnapshot(_ page: GaryxTaskForestPage, anchor: String) {
        let originKey = taskTreeScopedCacheKey(
            GaryxTaskTreeSidebarPresentation.treeCacheKey(page: page, anchorThreadId: anchor))
        taskTreeOriginKeyByAnchor[taskTreeScopedCacheKey(anchor)] = originKey
        if taskTreeSnapshotsByOrigin[originKey] == nil {
            taskTreeSnapshotOriginOrder.append(originKey)
        }
        taskTreeSnapshotsByOrigin[originKey] = page
        while taskTreeSnapshotOriginOrder.count > Self.taskTreeSnapshotCap {
            let evicted = taskTreeSnapshotOriginOrder.removeFirst()
            taskTreeSnapshotsByOrigin.removeValue(forKey: evicted)
        }
    }

    #if DEBUG
    /// Deterministic production-host fixture for A5 edge arbitration tests.
    /// It is inert unless the UI-test-only environment flag is present.
    func seedDebugTaskTreeGestureFixtureIfRequested() {
        guard ProcessInfo.processInfo.environment["GARYX_MOBILE_A5_TASK_TREE_FIXTURE"] == "1",
              let anchor = selectedThread?.id,
              let page = Self.decodeDebugFixture(GaryxTaskForestPage.self, from: """
              {
                "tasks": [
                  {
                    "kind": "thread",
                    "node_id": "thread-root:thread-history",
                    "thread_id": "thread-history",
                    "title": "Thread History",
                    "provider_type": "codex",
                    "agent_id": "codex",
                    "message_count": 36,
                    "run_state": "idle",
                    "depth": 0
                  },
                  {
                    "kind": "task",
                    "node_id": "task:synthetic-a5",
                    "parent_node_id": "thread-root:thread-history",
                    "thread_id": "thread-synthetic-a5",
                    "task_id": "#TASK-100",
                    "number": 100,
                    "title": "Synthetic gesture review",
                    "status": "in_progress",
                    "runtime_agent_id": "codex",
                    "reply_count": 0,
                    "parent_thread_id": "thread-history",
                    "run_state": "running",
                    "depth": 0
                  }
                ],
                "total": 2,
                "active_count": 1,
                "root_thread_ids": ["thread-history"],
                "skipped_pinned_thread_ids": []
              }
              """) else { return }
        taskTreeForestPage = page
        taskTreeLoadPhase = .loaded
        storeTaskTreeSnapshot(page, anchor: anchor)
    }
    #endif

    /// Identity hint for a sidebar row resolved against the loaded agent
    /// targets so avatars reuse the shared cache and presentation.
    func taskTreeRowAvatar(for row: GaryxTaskTreeRow) -> GaryxSidebarThreadRowAvatar {
        let identityId = row.identityAgentId
        if !identityId.isEmpty,
           let target = agentTargets.first(where: { $0.id == identityId }) {
            return GaryxSidebarThreadRowAvatar(
                agentId: target.id,
                avatarDataUrl: target.avatarDataUrl,
                label: target.title,
                providerType: target.providerType,
                builtIn: target.builtIn
            )
        }
        return GaryxSidebarThreadRowAvatar(
            agentId: identityId,
            avatarDataUrl: "",
            label: identityId.isEmpty ? row.title : identityId,
            providerType: row.providerType,
            builtIn: false
        )
    }

    private func dismissKeyboardForTaskTreeSidebar() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )
    }
}
